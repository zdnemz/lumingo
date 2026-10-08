//! The writing workshop (S4-12, PRD section 8.7): drafts in, feedback out.
//!
//! The first layer is the rule-based checker (grammar and spelling, no network,
//! [`RuleChecker`]); the second is the T2 turn analysis at the draft's cap of 20
//! errors, run on the draft's text with the empty tutor fields T2 asks for
//! ([`AnalysisTurn::draft`]); the third, the rubric bands (T3), needs the rubric
//! scorer and is S5-03's, recorded as a gap in ADR-051.
//!
//! A revision is compared with the nearest analysed earlier draft
//! ([`compare_drafts`]): an earlier error is "fixed" when its quoted text is
//! gone from the new draft and the analysis does not report it again,
//! "remaining" when it is still reported, and anything else reported is "new"
//! (context_pack.md section 9, ASSESSMENT_SPEC section 11).
//!
//! The workshop is practice only: every draft is stored as a turn and as an
//! attempt with origin `free_mode` and `counts_toward_estimate = false`
//! (FR-W5, ASSESSMENT_SPEC section 11). The caller stores the rows, exactly as
//! in `chat` and `unit`. A draft whose analysis cannot run waits in the pending
//! queue as a [`DraftPending`] payload, which carries everything a later drain
//! needs (FR-W6).

use assessment_engine::text_metrics::word_count;
use curriculum::Level;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::analysis::{
    AnalysisFailure, AnalysisInput, AnalysisOutcome, AnalysisTurn, DRAFT_ERROR_CAP, ErrorFinding,
    InputMode, normalise, run_analysis,
};
use crate::chat::clean_topic;
use crate::event::UiEvent;
use crate::llm::LlmClient;
use crate::session::{Channel, Event, Phase, Session, SessionKind, TransitionError};
use crate::topics::WritingPrompt;

/// The `scorer_version` a draft's attempt row carries. The row is practice:
/// `origin = free_mode` and `counts_toward_estimate = false`, so it never
/// enters a level estimate (ASSESSMENT_SPEC section 11).
pub const WORKSHOP_ATTEMPT_SCORER_VERSION: &str = "writing_workshop/1";

/// The payload format of a draft waiting for a provider (FR-W6).
pub const DRAFT_PENDING_VERSION: &str = "draft_pending/1";

/// Where the draft comes from (FR-W1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftSource {
    /// A prompt from the authored bank for the learner's level.
    Prompt(WritingPrompt),
    /// The learner's own topic, cleaned to one safe line.
    Topic(String),
    /// A text the learner pasted; it answers no task.
    Pasted,
}

/// What one workshop session needs to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopConfig {
    /// The level the feedback is written at, never a value a model produced.
    pub level: Level,
    /// The first language written in English ("Indonesian").
    pub first_language: String,
    pub source: DraftSource,
}

/// Why a draft could not be submitted or analysed.
#[derive(Debug, thiserror::Error)]
pub enum WorkshopError {
    #[error("the draft is empty")]
    EmptyDraft,
    #[error("the typed topic is empty")]
    EmptyTopic,
    #[error("no draft is in flight; submit one first")]
    NoDraftInFlight,
    #[error(transparent)]
    Transition(#[from] TransitionError),
}

/// One error of a draft as the comparison matches it: the quoted words and
/// their category. The rest of the finding (correction, severity) stays in the
/// stored analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftError {
    pub quote: String,
    pub category: String,
}

impl From<&ErrorFinding> for DraftError {
    fn from(finding: &ErrorFinding) -> Self {
        Self {
            quote: finding.quote.clone(),
            category: finding.category.clone(),
        }
    }
}

/// What happened to one error of the earlier draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Fixed,
    Remaining,
}

/// One error of the earlier draft with its resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EarlierError {
    pub error: DraftError,
    pub resolution: Resolution,
}

/// The revision comparison (FR-W4): every earlier error classified, and
/// everything the new analysis reported that does not continue one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftComparison {
    pub earlier: Vec<EarlierError>,
    pub new: Vec<DraftError>,
}

impl DraftComparison {
    /// Earlier errors whose words are gone and that are not reported again.
    pub fn fixed(&self) -> usize {
        self.earlier
            .iter()
            .filter(|e| e.resolution == Resolution::Fixed)
            .count()
    }

    /// Earlier errors the new analysis still reports on the same words.
    pub fn remaining(&self) -> usize {
        self.earlier
            .iter()
            .filter(|e| e.resolution == Resolution::Remaining)
            .count()
    }
}

/// The same words and category, ignoring case, spacing and the quote's own
/// surrounding whitespace.
fn same_error(a: &DraftError, b: &DraftError) -> bool {
    a.category == b.category && normalise(&a.quote) == normalise(&b.quote)
}

/// Compares the errors of the earlier analysed draft with the new draft and the
/// errors its analysis reported. An earlier error is `Remaining` when the new
/// analysis reports it again (same category, same words) and those words are
/// still in the draft; every other earlier error is `Fixed`. Anything the new
/// analysis reports that does not continue an earlier error is `New` — an
/// analysis can never hide a report, even one about words that have vanished.
///
/// The T2 filters already guarantee that every reported error's quote is in the
/// analysed text, so on filtered analyses the two conditions coincide and this
/// is exactly the spec's rule. Quotes match in the same normalised form the T2
/// filters use, so two quotes the filter calls equal compare equal; an empty
/// quote is never present.
pub fn compare_drafts(
    first: &[DraftError],
    second_text: &str,
    second: &[DraftError],
) -> DraftComparison {
    let text = normalise(second_text);
    let present = |error: &DraftError| {
        let quote = normalise(&error.quote);
        !quote.is_empty() && text.contains(&quote)
    };
    let earlier = first
        .iter()
        .map(|error| {
            let reported_again = second.iter().any(|candidate| same_error(error, candidate));
            EarlierError {
                error: error.clone(),
                resolution: if reported_again && present(error) {
                    Resolution::Remaining
                } else {
                    Resolution::Fixed
                },
            }
        })
        .collect();
    let new = second
        .iter()
        .filter(|candidate| {
            !first
                .iter()
                .any(|error| same_error(error, candidate) && present(error))
        })
        .cloned()
        .collect();
    DraftComparison { earlier, new }
}

/// One rule-based finding: character offsets into the checked text, the lint
/// kind, the message, and the replacement suggestions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleFinding {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub message: String,
    pub suggestions: Vec<String>,
}

/// What the rule-based checker said about one text. `checked` is false when the
/// crate was built without the `grammar` feature: the list is then empty
/// because nothing was checked, not because the text is clean.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleReport {
    pub checked: bool,
    pub findings: Vec<RuleFinding>,
}

#[cfg(feature = "grammar")]
mod checker {
    use super::{RuleFinding, RuleReport};

    /// The rule-based grammar and spelling checker (harper-core through
    /// `assessment-engine`, no network): the workshop's first layer (FR-W2).
    /// Building one loads a dictionary, so create it once and reuse it. It is
    /// not `Send`: keep it on the thread that created it.
    pub struct RuleChecker {
        checker: assessment_engine::text_metrics::GrammarChecker,
    }

    impl Default for RuleChecker {
        fn default() -> Self {
            Self::new()
        }
    }

    impl RuleChecker {
        pub fn new() -> Self {
            Self {
                checker: assessment_engine::text_metrics::GrammarChecker::new(),
            }
        }

        /// Grammar and spelling findings for one draft, at once and offline.
        /// Spelling is included: a draft is typed text, not a transcript.
        pub fn findings(&mut self, text: &str) -> RuleReport {
            RuleReport {
                checked: true,
                findings: self
                    .checker
                    .findings(text, true)
                    .into_iter()
                    .map(|finding| RuleFinding {
                        start: finding.start,
                        end: finding.end,
                        kind: finding.kind,
                        message: finding.message,
                        suggestions: finding.suggestions,
                    })
                    .collect(),
            }
        }
    }
}

#[cfg(not(feature = "grammar"))]
mod checker {
    use super::RuleReport;

    /// Without the `grammar` feature the checker cannot run; its report says
    /// `checked: false`, so a caller never reads an empty list as a clean draft.
    pub struct RuleChecker;

    impl Default for RuleChecker {
        fn default() -> Self {
            Self::new()
        }
    }

    impl RuleChecker {
        pub fn new() -> Self {
            Self
        }

        pub fn findings(&mut self, _text: &str) -> RuleReport {
            RuleReport::default()
        }
    }
}
pub use checker::RuleChecker;

/// What one submitted draft produced at once: the seq the caller stores the
/// draft turn at, its word count for the attempt's evidence row, and the
/// first feedback layer — the rule-based findings, which need no network and
/// arrive before any provider call (FR-W2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSubmission {
    pub learner_seq: i64,
    pub words: usize,
    pub rule: RuleReport,
}

/// The attempt row data for one stored draft (FR-W5: the workshop is
/// practice). The caller fills a `storage::NewAttempt` from it with origin
/// `free_mode`, status `insufficient`, no scores, and
/// `counts_toward_estimate = false`, so it never enters a level estimate but
/// the learner can look the draft back up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAttempt {
    pub response_id: String,
    pub activity_id: String,
    pub activity_type: String,
    pub level: Level,
    pub dimension: String,
    pub scorer_version: String,
    /// The draft's word count, kept as metric evidence.
    pub words: i64,
}

/// What one analysis produced.
#[derive(Debug)]
pub enum DraftRun {
    /// T2 ran: the errors are in, and the comparison when an earlier analysed
    /// draft exists.
    Analysed {
        outcome: AnalysisOutcome,
        comparison: Option<DraftComparison>,
    },
    /// The provider could not be reached: the draft waits for the queue drain
    /// (S5-03). The session is back to waiting, so the learner can keep
    /// revising (FR-W6).
    Pending {
        payload: Box<DraftPending>,
        failure: AnalysisFailure,
    },
}

/// A draft waiting for a provider: everything a later drain needs to finish the
/// feedback without the session. It holds the learner's own text, so it goes
/// away with the session like the turn does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftPending {
    pub version: String,
    /// The seq the draft's turn is stored at.
    pub turn_seq: i64,
    pub text: String,
    pub level: Level,
    pub l1: String,
    /// The authored prompt's id, when the draft answers one (FR-W1).
    pub prompt_id: Option<String>,
    /// The task the draft answers: the bank prompt or the learner's topic.
    /// `None` for a pasted text.
    pub prompt: Option<String>,
    pub content_points: Vec<String>,
}

impl DraftPending {
    /// The T2 input this draft needs, so the drain can run the analysis from
    /// the payload alone.
    pub fn analysis_input(&self) -> AnalysisInput {
        AnalysisInput::free(
            self.level,
            InputMode::Text,
            &self.l1,
            vec![AnalysisTurn::draft(self.turn_seq, &self.text)],
        )
    }
}

/// A writing workshop in progress: the session state machine on the text
/// channel, the source the drafts answer, and the nearest analysed earlier
/// draft for the next comparison.
#[derive(Debug, Clone)]
pub struct Workshop {
    level: Level,
    first_language: String,
    source: DraftSource,
    session: Session,
    last_draft: Option<Vec<DraftError>>,
    /// The draft in flight: its seq and text, until [`Workshop::analyse`] ends.
    in_flight: Option<(i64, String)>,
    next_seq: i64,
}

impl Workshop {
    /// Starts a workshop. The source decides the task the feedback judges the
    /// drafts against.
    pub fn start(config: WorkshopConfig) -> Result<Self, WorkshopError> {
        let source = match config.source {
            DraftSource::Topic(text) => {
                let cleaned = clean_topic(&text);
                if cleaned.is_empty() {
                    return Err(WorkshopError::EmptyTopic);
                }
                DraftSource::Topic(cleaned)
            }
            other => other,
        };
        Ok(Self {
            level: config.level,
            first_language: config.first_language,
            source,
            session: Session::new(SessionKind::Writing, Channel::Text),
            last_draft: None,
            in_flight: None,
            next_seq: 1,
        })
    }

    /// The session state machine, for the caller that forwards phases.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Seeds the comparison base with the errors of the nearest analysed
    /// earlier draft, for a session reopened from storage.
    pub fn set_earlier_draft(&mut self, errors: Vec<DraftError>) {
        self.last_draft = Some(errors);
    }

    /// Submits one draft: moves the session to `Thinking`, runs the rule-based
    /// checker on it (the first layer, at once and offline), and returns the
    /// seq the caller stores the draft turn at. The T2 analysis follows in
    /// [`Workshop::analyse`].
    pub fn submit_draft(
        &mut self,
        text: &str,
        checker: &mut RuleChecker,
        emit: &mut dyn FnMut(UiEvent),
    ) -> Result<DraftSubmission, WorkshopError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(WorkshopError::EmptyDraft);
        }
        let rule = checker.findings(text);
        let phase = self.session.apply(Event::TextSent)?;
        if let Phase::Active { turn } = phase {
            emit(UiEvent::turn_state(turn));
        }
        let learner_seq = self.next_seq;
        self.next_seq += 1;
        self.in_flight = Some((learner_seq, text.to_owned()));
        Ok(DraftSubmission {
            learner_seq,
            words: word_count(text),
            rule,
        })
    }

    /// Analyses the draft in flight: the T2 call on the draft's text, then the
    /// comparison with the nearest analysed earlier draft. The caller stores
    /// the filtered analysis and its error events, exactly as for a chat
    /// message. A provider failure is not an error: the draft comes back as a
    /// [`DraftPending`] payload for the queue, and the session returns to
    /// waiting so the learner can keep revising (FR-W6).
    pub async fn analyse(
        &mut self,
        client: &dyn LlmClient,
        cancel: &CancellationToken,
        emit: &mut dyn FnMut(UiEvent),
    ) -> Result<DraftRun, WorkshopError> {
        let Some((seq, text)) = self.in_flight.clone() else {
            return Err(WorkshopError::NoDraftInFlight);
        };
        let input = AnalysisInput::free(
            self.level,
            InputMode::Text,
            &self.first_language,
            vec![AnalysisTurn::draft(seq, &text)],
        );
        let result = run_analysis(client, &input, DRAFT_ERROR_CAP, cancel).await;
        self.in_flight = None;
        match result {
            Ok(outcome) => {
                // The feedback is shown: replying, then back to waiting.
                let phase = self.session.apply(Event::ReplyStarted)?;
                if let Phase::Active { turn } = phase {
                    emit(UiEvent::turn_state(turn));
                }
                let errors: Vec<DraftError> = outcome
                    .filtered
                    .turns
                    .iter()
                    .find(|entry| entry.turn_seq == seq)
                    .map(|entry| entry.errors.iter().map(DraftError::from).collect())
                    .unwrap_or_default();
                let comparison = self
                    .last_draft
                    .as_ref()
                    .map(|first| compare_drafts(first, &text, &errors));
                self.last_draft = Some(errors);
                let phase = self.session.apply(Event::ReplyFinished)?;
                if let Phase::Active { turn } = phase {
                    emit(UiEvent::turn_state(turn));
                }
                Ok(DraftRun::Analysed {
                    outcome,
                    comparison,
                })
            }
            Err(failure) => {
                // The turn completes without feedback; the caller queues the
                // payload and the drain finishes it later.
                let payload = self.pending_payload(seq, &text);
                let phase = self.session.apply(Event::ReplyFinished)?;
                if let Phase::Active { turn } = phase {
                    emit(UiEvent::turn_state(turn));
                }
                Ok(DraftRun::Pending {
                    payload: Box::new(payload),
                    failure,
                })
            }
        }
    }

    /// The attempt row data for one stored draft (FR-W5: practice only).
    pub fn attempt_for(&self, session_id: i64, seq: i64, words: i64) -> DraftAttempt {
        DraftAttempt {
            response_id: format!("draft-{session_id}-{seq}"),
            activity_id: "writing_workshop".to_owned(),
            activity_type: "writing_workshop".to_owned(),
            level: self.level,
            dimension: "draft".to_owned(),
            scorer_version: WORKSHOP_ATTEMPT_SCORER_VERSION.to_owned(),
            words,
        }
    }

    fn pending_payload(&self, turn_seq: i64, text: &str) -> DraftPending {
        let (prompt_id, prompt, content_points) = match &self.source {
            DraftSource::Prompt(prompt) => (
                Some(prompt.id.clone()),
                Some(prompt.prompt.en.clone()),
                prompt.content_points.clone(),
            ),
            DraftSource::Topic(topic) => (None, Some(topic.clone()), Vec::new()),
            DraftSource::Pasted => (None, None, Vec::new()),
        };
        DraftPending {
            version: DRAFT_PENDING_VERSION.to_owned(),
            turn_seq,
            text: text.to_owned(),
            level: self.level,
            l1: self.first_language.clone(),
            prompt_id,
            prompt,
            content_points,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curriculum::Localized;

    fn err(quote: &str, category: &str) -> DraftError {
        DraftError {
            quote: quote.to_owned(),
            category: category.to_owned(),
        }
    }

    fn no_events(_event: UiEvent) {}

    fn workshop(source: DraftSource) -> Workshop {
        Workshop::start(WorkshopConfig {
            level: Level::A1,
            first_language: "Indonesian".to_owned(),
            source,
        })
        .unwrap()
    }

    #[test]
    fn an_error_whose_words_are_gone_and_not_reported_is_fixed() {
        let first = [err("I go to the market yesterday", "verb_tense")];
        let result = compare_drafts(&first, "Yesterday I went to the market.", &[]);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.remaining(), 0);
        assert!(result.new.is_empty());
    }

    #[test]
    fn an_error_reported_again_on_the_same_words_remains() {
        let first = [err("She have two brother", "subject_verb_agreement")];
        let second = [err("she have two brother", "subject_verb_agreement")];
        let result = compare_drafts(&first, "She have two brother. They are kind.", &second);
        assert_eq!(result.remaining(), 1);
        assert_eq!(result.fixed(), 0);
        assert!(result.new.is_empty(), "the same error is not also new");
    }

    #[test]
    fn something_reported_that_was_not_there_before_is_new() {
        let first = [err("I go", "verb_tense")];
        let second = [err("two brothers is", "subject_verb_agreement")];
        let result = compare_drafts(&first, "I went. I have two brothers is kind.", &second);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.new, second);
    }

    #[test]
    fn the_same_words_with_a_different_category_are_a_different_error() {
        let first = [err("in the morning", "preposition")];
        let second = [err("in the morning", "word_order")];
        let result = compare_drafts(&first, "I wake in the morning.", &second);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.new.len(), 1);
    }

    #[test]
    fn a_reported_error_whose_words_vanished_is_fixed_and_surfaced_as_new() {
        // The analysis repeats an old report, but the learner already rewrote
        // those words. The earlier error is fixed; the report about words that
        // are not in the draft is shown, not hidden.
        let first = [err("I am agree", "verb_form")];
        let second = [err("I am agree", "verb_form")];
        let result = compare_drafts(&first, "I agree with you.", &second);
        assert_eq!(result.remaining(), 0);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.new.len(), 1);
    }

    #[test]
    fn matching_ignores_case_and_spacing() {
        let first = [err("He  don't like", "verb_form")];
        let second = [err("he don't like", "verb_form")];
        let result = compare_drafts(&first, "he don't   like it", &second);
        assert_eq!(result.remaining(), 1);
    }

    #[test]
    fn every_earlier_error_gets_exactly_one_resolution_in_order() {
        let first = [err("a b", "x"), err("c d", "y"), err("e f", "z")];
        let second = [err("c d", "y")];
        let result = compare_drafts(&first, "c d and more", &second);
        let resolutions: Vec<Resolution> = result.earlier.iter().map(|e| e.resolution).collect();
        assert_eq!(
            resolutions,
            [Resolution::Fixed, Resolution::Remaining, Resolution::Fixed]
        );
        assert_eq!(result.earlier.len(), first.len());
        assert_eq!(result.earlier[0].error, first[0]);
    }

    #[test]
    fn an_empty_quote_is_never_present() {
        let first = [err("", "x")];
        let second = [err("", "x")];
        let result = compare_drafts(&first, "Some text.", &second);
        assert_eq!(result.fixed(), 1);
        assert_eq!(result.remaining(), 0);
        assert_eq!(result.new.len(), 1);
    }

    #[test]
    fn with_no_errors_in_either_draft_there_is_nothing_to_report() {
        let result = compare_drafts(&[], "A clean text.", &[]);
        assert_eq!(result, DraftComparison::default());
    }

    #[cfg(feature = "grammar")]
    #[test]
    fn the_rule_checker_flags_a_mistake_and_passes_a_clean_sentence() {
        let mut checker = RuleChecker::new();
        let bad = checker.findings("This is an test.");
        assert!(bad.checked);
        assert!(!bad.findings.is_empty(), "expected a finding for 'an test'");
        let clean = checker.findings("She goes to school every day.");
        assert!(clean.checked);
        assert!(clean.findings.is_empty(), "{:?}", clean.findings);
    }

    #[test]
    fn an_empty_draft_is_refused_and_a_draft_gets_the_next_seq() {
        let mut session = workshop(DraftSource::Pasted);
        let mut checker = RuleChecker::new();
        let error = session
            .submit_draft("   ", &mut checker, &mut no_events)
            .unwrap_err();
        assert!(matches!(error, WorkshopError::EmptyDraft));
        let first = session
            .submit_draft("  My first draft.  ", &mut checker, &mut no_events)
            .unwrap();
        assert_eq!(first.learner_seq, 1);
        assert_eq!(first.words, 3);
        // The first layer arrives with the submission, before any provider call.
        assert_eq!(first.rule.checked, cfg!(feature = "grammar"));
        assert!(matches!(
            session.session().turn(),
            Some(crate::session::TurnState::Thinking)
        ));
        // A second draft while one is in flight is refused: analyse first.
        let error = session
            .submit_draft("Second.", &mut checker, &mut no_events)
            .unwrap_err();
        assert!(matches!(error, WorkshopError::Transition(_)));
    }

    #[test]
    fn an_empty_topic_is_refused_and_a_topic_is_cleaned() {
        let error = Workshop::start(WorkshopConfig {
            level: Level::A1,
            first_language: "Indonesian".to_owned(),
            source: DraftSource::Topic(" \n ".to_owned()),
        })
        .unwrap_err();
        assert!(matches!(error, WorkshopError::EmptyTopic));
        let session = workshop(DraftSource::Topic("  My <b>holiday</b>  ".to_owned()));
        assert_eq!(
            session.source,
            DraftSource::Topic("My b holiday /b".to_owned())
        );
    }

    #[test]
    fn a_draft_attempt_is_practice_only() {
        let session = workshop(DraftSource::Pasted);
        let attempt = session.attempt_for(7, 3, 42);
        assert_eq!(attempt.response_id, "draft-7-3");
        assert_eq!(attempt.activity_id, "writing_workshop");
        assert_eq!(attempt.dimension, "draft");
        assert_eq!(attempt.scorer_version, WORKSHOP_ATTEMPT_SCORER_VERSION);
        assert_eq!(attempt.level, Level::A1);
        assert_eq!(attempt.words, 42);
    }

    #[test]
    fn the_pending_payload_carries_the_task_and_rebuilds_the_analysis_input() {
        let session = workshop(DraftSource::Prompt(WritingPrompt {
            id: "note".to_owned(),
            prompt: Localized {
                en: "Write a note to a friend.".to_owned(),
                id: Some("Tulis catatan.".to_owned()),
            },
            reader: "a friend".to_owned(),
            purpose: "invite".to_owned(),
            content_points: vec!["when".to_owned(), "where".to_owned()],
        }));
        let payload = session.pending_payload(4, "Dear Sari, come at 7.");
        assert_eq!(payload.version, DRAFT_PENDING_VERSION);
        assert_eq!(payload.turn_seq, 4);
        assert_eq!(payload.level, Level::A1);
        assert_eq!(payload.l1, "Indonesian");
        assert_eq!(payload.prompt_id.as_deref(), Some("note"));
        assert_eq!(payload.prompt.as_deref(), Some("Write a note to a friend."));
        assert_eq!(payload.content_points, ["when", "where"]);
        // The drain can run T2 from the payload alone.
        let input = payload.analysis_input();
        assert_eq!(input.level, Level::A1);
        assert_eq!(input.l1, "Indonesian");
        assert_eq!(input.input_mode, InputMode::Text);
        assert_eq!(input.turns.len(), 1);
        assert_eq!(input.turns[0].turn_seq, 4);
        assert_eq!(input.turns[0].learner_text, "Dear Sari, come at 7.");
        assert!(input.turns[0].tutor_before.is_empty());
        assert!(input.turns[0].tutor_reply.is_empty());

        // A pasted text answers no task.
        let pasted = workshop(DraftSource::Pasted).pending_payload(1, "text");
        assert_eq!(pasted.prompt_id, None);
        assert_eq!(pasted.prompt, None);
        assert!(pasted.content_points.is_empty());
        // A typed topic becomes the task prompt.
        let topic = workshop(DraftSource::Topic("My dog".to_owned())).pending_payload(2, "text");
        assert_eq!(topic.prompt_id, None);
        assert_eq!(topic.prompt.as_deref(), Some("My dog"));
    }

    #[tokio::test]
    async fn analyse_without_a_draft_is_refused() {
        let mut workshop = workshop(DraftSource::Pasted);
        let client = UnusedClient;
        let cancel = CancellationToken::new();
        let error = workshop
            .analyse(&client, &cancel, &mut no_events)
            .await
            .unwrap_err();
        assert!(matches!(error, WorkshopError::NoDraftInFlight));
    }

    #[tokio::test]
    async fn a_seeded_earlier_draft_is_compared_by_the_next_analysis() {
        // A session reopened from storage seeds the comparison base, and the
        // next analysed draft is classified against it.
        let client = FixedClient(serde_json::json!({
            "turns": [{
                "turn_seq": 1,
                "errors": [{
                    "category": "verb_tense",
                    "quote": "go",
                    "correction": "went",
                    "severity": "major",
                    "addressed_in_reply": false
                }],
                "objective_evidence": [],
                "understood_tutor": "not_applicable",
                "note_for_next_turn": ""
            }]
        }));
        let mut session = workshop(DraftSource::Pasted);
        session.set_earlier_draft(vec![err("go", "verb_tense"), err("cat", "plural_number")]);
        session
            .submit_draft("I go to school.", &mut RuleChecker::new(), &mut no_events)
            .unwrap();
        let cancel = CancellationToken::new();
        let run = session
            .analyse(&client, &cancel, &mut no_events)
            .await
            .unwrap();
        let DraftRun::Analysed { comparison, .. } = run else {
            panic!("the fixed client answers");
        };
        let comparison = comparison.expect("the seeded draft is compared");
        assert_eq!(
            comparison.remaining(),
            1,
            "\"go\" is still there and reported"
        );
        assert_eq!(comparison.fixed(), 1, "\"cat\" is not");
        assert!(comparison.new.is_empty());
        // The session ended the turn back at waiting.
        assert!(matches!(
            session.session().turn(),
            Some(crate::session::TurnState::Waiting)
        ));
    }

    /// A client that answers every structured call with one fixed value.
    struct FixedClient(serde_json::Value);

    impl LlmClient for FixedClient {
        fn stream_text(
            &self,
            _request: llm_client::TextRequest,
            _cancel: CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<crate::TextStream, llm_client::LlmError>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async { Err(llm_client::LlmError::Transport("no stream".to_owned())) })
        }

        fn structured(
            &self,
            _request: llm_client::StructuredRequest,
            _cancel: CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<llm_client::StructuredOutput, llm_client::LlmError>,
                    > + Send
                    + '_,
            >,
        > {
            let value = self.0.clone();
            Box::pin(async move {
                Ok(llm_client::StructuredOutput {
                    value,
                    level: llm_client::Level::NativeSchema,
                    repaired: false,
                })
            })
        }
    }

    /// A client that fails every call: `analyse` must not reach it without a
    /// draft, so the test never depends on its answers.
    struct UnusedClient;

    impl LlmClient for UnusedClient {
        fn stream_text(
            &self,
            _request: llm_client::TextRequest,
            _cancel: CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<crate::TextStream, llm_client::LlmError>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async { Err(llm_client::LlmError::Transport("unused".to_owned())) })
        }

        fn structured(
            &self,
            _request: llm_client::StructuredRequest,
            _cancel: CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<llm_client::StructuredOutput, llm_client::LlmError>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async { Err(llm_client::LlmError::Transport("unused".to_owned())) })
        }
    }
}
