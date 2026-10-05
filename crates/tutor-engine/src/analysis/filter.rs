//! The typed form of a `turn_analysis` reply and the semantic filters of
//! contract T2.
//!
//! The schema check already happened inside the LLM client. Everything here is
//! what a schema cannot say: the quoted words must be the learner's words, the
//! objective must be one we asked about, speech has no spelling, and the caps.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use storage::Severity;

use crate::prompt::MAX_NOTES;
use crate::support::{fold, names_a_level, quote_in, truncate_words};

use super::prompt::InputMode;

/// A note for the next turn is one short sentence of at most this many words.
pub const MAX_NOTE_WORDS: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Demonstrated,
    Partial,
    NotDemonstrated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Understood {
    Yes,
    Partly,
    No,
    NotApplicable,
}

/// One error as the model reported it, before filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorFinding {
    pub category: String,
    pub quote: String,
    pub correction: String,
    pub severity: Severity,
    pub addressed_in_reply: bool,
}

/// What the turn showed about one objective.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFinding {
    pub objective_id: String,
    pub status: EvidenceStatus,
    pub quote: String,
}

/// One turn of the reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysedTurn {
    pub turn_seq: i64,
    pub errors: Vec<ErrorFinding>,
    pub objective_evidence: Vec<EvidenceFinding>,
    pub understood_tutor: Understood,
    pub note_for_next_turn: String,
}

/// The whole reply, as the contract schema defines it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAnalysis {
    pub turns: Vec<AnalysedTurn>,
}

/// What the filters need to know about the turn they are judging.
#[derive(Debug, Clone, Copy)]
pub struct FilterContext<'a> {
    pub learner_text: &'a str,
    pub mode: InputMode,
    /// Objective ids that were in the input.
    pub objective_ids: &'a HashSet<String>,
    /// 5 for a conversation turn or chat message, 20 for a writing draft.
    pub max_errors: usize,
}

/// A turn after the filters, with the count of what they removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filtered {
    pub turn: AnalysedTurn,
    /// Errors and evidence entries the model returned for this turn.
    pub entries: usize,
    /// How many of them a filter removed.
    pub dropped: usize,
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Blocking => 0,
        Severity::Major => 1,
        Severity::Minor => 2,
    }
}

/// Applies the filters of T2 to one turn of the reply.
///
/// 1. An error is dropped when its quote is not in the learner's text after
///    lower-casing and collapsing whitespace. An empty quote is not in the text:
///    it would match everywhere and prove nothing.
/// 2. Evidence is dropped when its objective was not in the input, or when its
///    quote is not in the text. Evidence that claims `demonstrated` or `partial`
///    with no quote at all is dropped too: nothing backs it. (The contract only
///    asks to drop a non-empty quote that is wrong; the empty case is stricter
///    on purpose, because mastery is built from this evidence.)
/// 3. For voice, `spelling` and `punctuation` errors are dropped.
/// 4. Errors that change nothing (the correction is the quote) and repeats of
///    the same quote and category are dropped.
/// 5. At most `max_errors` errors are kept, most severe first.
///
/// The note is cut to 25 words and discarded if it names a CEFR level.
pub fn apply_filters(raw: &AnalysedTurn, ctx: &FilterContext<'_>) -> Filtered {
    let text = fold(ctx.learner_text);
    let entries = raw.errors.len() + raw.objective_evidence.len();
    let mut kept_errors: Vec<ErrorFinding> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for error in &raw.errors {
        if !quote_in(&text, &error.quote) {
            continue;
        }
        if ctx.mode == InputMode::Voice
            && matches!(error.category.as_str(), "spelling" | "punctuation")
        {
            continue;
        }
        if fold(&error.correction) == fold(&error.quote) {
            continue;
        }
        if !seen.insert((error.category.clone(), fold(&error.quote))) {
            continue;
        }
        kept_errors.push(error.clone());
    }
    kept_errors.sort_by_key(|error| severity_rank(error.severity));
    kept_errors.truncate(ctx.max_errors);

    let mut kept_evidence: Vec<EvidenceFinding> = Vec::new();
    for evidence in &raw.objective_evidence {
        if !ctx.objective_ids.contains(&evidence.objective_id) {
            continue;
        }
        let quoted = !evidence.quote.trim().is_empty();
        let supported = if quoted {
            quote_in(&text, &evidence.quote)
        } else {
            evidence.status == EvidenceStatus::NotDemonstrated
        };
        if !supported {
            continue;
        }
        let repeat = kept_evidence
            .iter()
            .any(|e| e.objective_id == evidence.objective_id && e.status == evidence.status);
        if !repeat {
            kept_evidence.push(evidence.clone());
        }
    }

    let note = truncate_words(raw.note_for_next_turn.trim(), MAX_NOTE_WORDS);
    let note = if names_a_level(&note) {
        String::new()
    } else {
        note
    };

    let kept = kept_errors.len() + kept_evidence.len();
    Filtered {
        turn: AnalysedTurn {
            turn_seq: raw.turn_seq,
            errors: kept_errors,
            objective_evidence: kept_evidence,
            understood_tutor: raw.understood_tutor,
            note_for_next_turn: note,
        },
        entries,
        dropped: entries.saturating_sub(kept),
    }
}

/// A rolling record of how many entries the filters removed, for the "analysis
/// unreliable" mark of contract T2: more than a third of the entries of the last
/// 20 analyses dropped.
#[derive(Debug, Clone, Default)]
pub struct DropWindow {
    recent: std::collections::VecDeque<(usize, usize)>,
}

/// Analyses in the window.
pub const DROP_WINDOW: usize = 20;
/// Fewer analyses than this give no verdict: with one or two replies, a single
/// bad entry would already be "more than a third".
const MIN_ANALYSES_FOR_VERDICT: usize = 5;

impl DropWindow {
    pub fn record(&mut self, entries: usize, dropped: usize) {
        if self.recent.len() == DROP_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back((entries, dropped));
    }

    pub fn analyses(&self) -> usize {
        self.recent.len()
    }

    pub fn unreliable(&self) -> bool {
        if self.recent.len() < MIN_ANALYSES_FOR_VERDICT {
            return false;
        }
        let entries: usize = self.recent.iter().map(|(e, _)| e).sum();
        let dropped: usize = self.recent.iter().map(|(_, d)| d).sum();
        entries > 0 && dropped * 3 > entries
    }
}

/// Keeps the latest non-empty notes, newest first, at most [`MAX_NOTES`].
#[derive(Debug, Clone, Default)]
pub struct Notes {
    newest_first: Vec<String>,
}

impl Notes {
    pub fn push(&mut self, note: &str) {
        let note = note.trim();
        if note.is_empty() {
            return;
        }
        self.newest_first.retain(|known| known != note);
        self.newest_first.insert(0, note.to_owned());
        self.newest_first.truncate(MAX_NOTES);
    }

    pub fn latest(&self) -> Vec<String> {
        self.newest_first.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(category: &str, quote: &str, correction: &str, severity: Severity) -> ErrorFinding {
        ErrorFinding {
            category: category.into(),
            quote: quote.into(),
            correction: correction.into(),
            severity,
            addressed_in_reply: false,
        }
    }

    fn evidence(id: &str, status: EvidenceStatus, quote: &str) -> EvidenceFinding {
        EvidenceFinding {
            objective_id: id.into(),
            status,
            quote: quote.into(),
        }
    }

    fn turn(errors: Vec<ErrorFinding>, evidence: Vec<EvidenceFinding>, note: &str) -> AnalysedTurn {
        AnalysedTurn {
            turn_seq: 2,
            errors,
            objective_evidence: evidence,
            understood_tutor: Understood::Yes,
            note_for_next_turn: note.into(),
        }
    }

    fn run(raw: &AnalysedTurn, text: &str, mode: InputMode, max: usize) -> Filtered {
        let ids: HashSet<String> = ["o1".to_owned(), "o2".to_owned()].into();
        apply_filters(
            raw,
            &FilterContext {
                learner_text: text,
                mode,
                objective_ids: &ids,
                max_errors: max,
            },
        )
    }

    #[test]
    fn an_error_whose_quote_is_not_in_the_text_is_dropped() {
        let raw = turn(
            vec![
                error(
                    "verb_tense",
                    "I go yesterday",
                    "I went yesterday",
                    Severity::Major,
                ),
                error(
                    "article",
                    "a invented phrase",
                    "an invented phrase",
                    Severity::Minor,
                ),
            ],
            vec![],
            "",
        );
        let out = run(
            &raw,
            "Hello. I  GO yesterday to school.",
            InputMode::Text,
            5,
        );
        assert_eq!(out.turn.errors.len(), 1);
        assert_eq!(out.turn.errors[0].quote, "I go yesterday");
        assert_eq!((out.entries, out.dropped), (2, 1));
    }

    #[test]
    fn an_empty_quote_proves_nothing_and_is_dropped() {
        let raw = turn(
            vec![error("article", "  ", "a", Severity::Minor)],
            vec![],
            "",
        );
        assert!(
            run(&raw, "anything", InputMode::Text, 5)
                .turn
                .errors
                .is_empty()
        );
    }

    #[test]
    fn evidence_for_an_objective_that_was_not_asked_about_is_dropped() {
        let raw = turn(
            vec![],
            vec![
                evidence("o1", EvidenceStatus::Demonstrated, "my name is Dewi"),
                evidence("o9", EvidenceStatus::Demonstrated, "my name is Dewi"),
            ],
            "",
        );
        let out = run(&raw, "My name is Dewi", InputMode::Text, 5);
        let ids: Vec<&str> = out
            .turn
            .objective_evidence
            .iter()
            .map(|e| e.objective_id.as_str())
            .collect();
        assert_eq!(ids, ["o1"]);
        assert_eq!(out.dropped, 1);
    }

    #[test]
    fn evidence_with_a_quote_that_is_not_in_the_text_is_dropped() {
        let raw = turn(
            vec![],
            vec![
                evidence("o1", EvidenceStatus::Partial, "I am from Bali"),
                evidence("o2", EvidenceStatus::Partial, "I live in Bali"),
            ],
            "",
        );
        let out = run(&raw, "I am from Bali", InputMode::Text, 5);
        assert_eq!(out.turn.objective_evidence.len(), 1);
        assert_eq!(out.turn.objective_evidence[0].objective_id, "o1");
    }

    #[test]
    fn not_demonstrated_may_have_an_empty_quote_but_demonstrated_may_not() {
        let raw = turn(
            vec![],
            vec![
                evidence("o1", EvidenceStatus::NotDemonstrated, ""),
                evidence("o2", EvidenceStatus::Demonstrated, ""),
            ],
            "",
        );
        let out = run(&raw, "Hello", InputMode::Text, 5);
        assert_eq!(out.turn.objective_evidence.len(), 1);
        assert_eq!(
            out.turn.objective_evidence[0].status,
            EvidenceStatus::NotDemonstrated
        );
    }

    #[test]
    fn voice_turns_lose_spelling_and_punctuation_errors_and_text_turns_keep_them() {
        let raw = turn(
            vec![
                error("spelling", "freind", "friend", Severity::Minor),
                error("punctuation", "hello how", "hello, how", Severity::Minor),
                error("verb_form", "he go", "he goes", Severity::Major),
            ],
            vec![],
            "",
        );
        let text = "hello how are you my freind he go home";
        let voice = run(&raw, text, InputMode::Voice, 5);
        let categories: Vec<&str> = voice
            .turn
            .errors
            .iter()
            .map(|e| e.category.as_str())
            .collect();
        assert_eq!(categories, ["verb_form"]);
        assert_eq!(run(&raw, text, InputMode::Text, 5).turn.errors.len(), 3);
    }

    #[test]
    fn the_cap_keeps_the_most_severe_errors_first() {
        let errors = vec![
            error("article", "a apple", "an apple", Severity::Minor),
            error("verb_tense", "I go", "I went", Severity::Major),
            error(
                "unclear_meaning",
                "blue run",
                "blue run",
                Severity::Blocking,
            ),
            error("word_choice", "very good", "great", Severity::Minor),
        ];
        let raw = turn(errors, vec![], "");
        let out = run(&raw, "a apple I go very good blue run", InputMode::Text, 2);
        // "blue run" has no change, so it is dropped as a no-op; the cap then
        // keeps the major error before the first minor one.
        let categories: Vec<&str> = out
            .turn
            .errors
            .iter()
            .map(|e| e.category.as_str())
            .collect();
        assert_eq!(categories, ["verb_tense", "article"]);
    }

    #[test]
    fn a_blocking_error_outranks_the_rest_under_the_cap() {
        let errors = vec![
            error("article", "a apple", "an apple", Severity::Minor),
            error(
                "unclear_meaning",
                "blue run",
                "green sun",
                Severity::Blocking,
            ),
        ];
        let raw = turn(errors, vec![], "");
        let out = run(&raw, "a apple blue run", InputMode::Text, 1);
        assert_eq!(out.turn.errors[0].severity, Severity::Blocking);
        assert_eq!(out.dropped, 1);
    }

    #[test]
    fn the_draft_cap_is_twenty_and_a_turn_cap_is_five() {
        let words: Vec<String> = (0..25).map(|n| format!("wrd{n}")).collect();
        let text = words.join(" ");
        let errors: Vec<ErrorFinding> = words
            .iter()
            .map(|w| error("word_choice", w, "other", Severity::Minor))
            .collect();
        let raw = turn(errors, vec![], "");
        assert_eq!(run(&raw, &text, InputMode::Text, 20).turn.errors.len(), 20);
        assert_eq!(run(&raw, &text, InputMode::Text, 5).turn.errors.len(), 5);
    }

    #[test]
    fn repeats_and_no_op_corrections_are_dropped() {
        let raw = turn(
            vec![
                error("article", "a apple", "an apple", Severity::Minor),
                error("article", "A  Apple", "an apple", Severity::Minor),
                error("word_choice", "good", "Good", Severity::Minor),
            ],
            vec![],
            "",
        );
        let out = run(&raw, "a apple is good", InputMode::Text, 5);
        assert_eq!(out.turn.errors.len(), 1);
        assert_eq!(out.dropped, 2);
    }

    #[test]
    fn a_long_note_is_cut_and_a_note_naming_a_level_is_discarded() {
        let long = (1..=40)
            .map(|n| format!("w{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let out = run(&turn(vec![], vec![], &long), "x", InputMode::Text, 5);
        assert_eq!(out.turn.note_for_next_turn.split_whitespace().count(), 25);
        let out = run(
            &turn(vec![], vec![], "Speak at B2 level next time."),
            "x",
            InputMode::Text,
            5,
        );
        assert_eq!(out.turn.note_for_next_turn, "");
    }

    #[test]
    fn the_stored_shape_matches_the_contract_field_names() {
        let analysis = RawAnalysis {
            turns: vec![turn(
                vec![error("article", "a apple", "an apple", Severity::Minor)],
                vec![evidence("o1", EvidenceStatus::Partial, "a apple")],
                "Practise a and an.",
            )],
        };
        let value = serde_json::to_value(&analysis).expect("serialise");
        let first = &value["turns"][0];
        assert_eq!(first["errors"][0]["severity"], "minor");
        assert_eq!(first["errors"][0]["addressed_in_reply"], false);
        assert_eq!(first["objective_evidence"][0]["status"], "partial");
        assert_eq!(first["understood_tutor"], "yes");
        let back: RawAnalysis = serde_json::from_value(value).expect("deserialise");
        assert_eq!(back, analysis);
    }

    #[test]
    fn the_drop_window_marks_a_profile_unreliable_above_a_third() {
        let mut window = DropWindow::default();
        for _ in 0..4 {
            window.record(3, 3);
        }
        assert!(!window.unreliable(), "too few analyses for a verdict");
        window.record(3, 3);
        assert!(window.unreliable());

        let mut fine = DropWindow::default();
        for _ in 0..10 {
            fine.record(3, 1);
        }
        assert!(
            !fine.unreliable(),
            "exactly a third is not more than a third"
        );
        fine.record(3, 2);
        assert!(fine.unreliable());
    }

    #[test]
    fn the_drop_window_forgets_analyses_older_than_twenty() {
        let mut window = DropWindow::default();
        for _ in 0..20 {
            window.record(3, 3);
        }
        assert!(window.unreliable());
        for _ in 0..20 {
            window.record(3, 0);
        }
        assert_eq!(window.analyses(), 20);
        assert!(!window.unreliable());
    }

    #[test]
    fn notes_keep_the_latest_three_newest_first_without_repeats() {
        let mut notes = Notes::default();
        for note in ["one", "", "two", "one", "three", "four"] {
            notes.push(note);
        }
        assert_eq!(notes.latest(), ["four", "three", "one"]);
    }
}
