//! The rubric scorer service (S5-03): score one productive response with the T3
//! prompt, in one or two runs, apply the cross-checks X1 to X7 and the
//! confidence table, store the result through the evidence recorder, and score
//! the backlog of responses that waited for a provider.
//!
//! A model never sets a level here. Its reply is bands per dimension with quotes;
//! the bands are checked, turned into a score from 0 to 1 and stored. What a level
//! estimate does with them is `assessment-engine`'s business.

use std::sync::Arc;
use std::time::Instant;

use assessment_engine::{Level, TimingMetrics, VocabularyProfile, text_counts, vocabulary_profile};
use curriculum::validate::{GrammarCheck, WordLevels};
use llm_client::{
    ChatMessage, Contract, LadderLevel, LlmClient, LlmError, StructuredOutput, StructuredRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use storage::{
    Attempt, AttemptStatus, Database, EvidenceKind, LlmCallType, NewEvidence, ScoreUpdate,
};
use tokio_util::sync::CancellationToken;

use super::{
    CheckedRun, CrossCheck, CurriculumWords, RawRubric, RubricOutcome, WorkshopRubric,
    WorkshopTask, cross_check, merge_rerun, system_prompt, user_message,
};
use crate::activity::ActivityError;
use crate::error::{EngineError, Result};
use crate::evidence::{EvidenceRecorder, Recorded, RubricMeta, Subject};
use crate::support::{CallLog, Clock};

/// The `kind` of a queue entry written for a scored activity response.
pub const PENDING_KIND: &str = "rubric_response";

/// Entries read from the queue per pass.
const BACKLOG_BATCH: u32 = 50;

/// An entry that failed this many times on the model's output, not on the
/// connection, is given up on: its attempts become `needs_review` and the entry
/// leaves the queue, so it cannot sit in front of the others for ever.
pub const MAX_OUTPUT_TRIES: i64 = 5;

/// How the response was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    Text,
    /// A speech transcript: spelling, capitals and punctuation are ignored.
    Voice,
}

impl InputMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
        }
    }
}

/// One run for practice and the workshop, two for checkpoint and placement tasks
/// (assessment spec 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runs {
    One,
    Two,
}

impl Runs {
    fn count(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }
}

/// What the scorer works with. Cheap to clone.
#[derive(Clone)]
pub struct ScorerEnv {
    pub client: Arc<dyn LlmClient>,
    pub db: Database,
    pub clock: Clock,
    /// The model's name, stored with every score.
    pub model: String,
    pub provider_profile_id: Option<i64>,
    /// The rule-based checker for X5. Without one X5 is not evaluated: a missing
    /// checker is not a response with no findings.
    pub grammar: Option<Arc<dyn GrammarCheck + Send + Sync>>,
    /// Word levels for the range cross-check (X4) and the vocabulary profile.
    pub word_levels: Option<Arc<WordLevels>>,
    /// Whether the provider passed scorer qualification: it sets the starting
    /// confidence of a rubric attempt.
    pub provider_qualified: bool,
}

/// One productive response to score. It is also the payload of a queue entry, so
/// everything the scorer needs travels with it: it holds the learner's text and
/// goes with the session when the session is deleted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreRequest {
    pub subject: Subject,
    pub rubric: WorkshopRubric,
    pub task: WorkshopTask,
    pub response: String,
    pub input_mode: InputMode,
    pub runs: Runs,
    /// The first language written in English ("Indonesian"), for the feedback.
    pub first_language: String,
    /// Timing figures of a spoken response. Descriptive: they go into the
    /// stored metrics and nowhere else.
    pub timing: Option<TimingMetrics>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingResponse {
    kind: String,
    request: ScoreRequest,
}

/// What `score` did with a response.
#[derive(Debug, Clone, PartialEq)]
pub enum ScoreResult {
    /// The model scored it. The rows are stored.
    Scored {
        recorded: Recorded,
        outcome: Box<RubricOutcome>,
    },
    /// No provider could be reached. The response is stored as `pending_llm`
    /// and waits in the queue for [`RubricScorer::score_pending_backlog`].
    Queued { recorded: Recorded },
}

/// What a pass over the backlog did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BacklogReport {
    /// Responses scored and removed from the queue.
    pub scored: usize,
    /// Responses given up on after repeated unusable output: their attempts are
    /// now `needs_review`.
    pub given_up: usize,
    /// Entries that could not be read, or whose attempts are gone.
    pub skipped: usize,
    /// The pass stopped because the provider could not be reached.
    pub provider_unreachable: bool,
    /// Entries of this kind still in the queue after the pass.
    pub remaining: usize,
}

pub struct RubricScorer {
    env: ScorerEnv,
    recorder: EvidenceRecorder,
}

/// One call to the model, checked.
pub(crate) struct RawRun {
    raw: RawRubric,
    repaired: bool,
    ladder_level: LadderLevel,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn call_rubric(
    env: &ScorerEnv,
    first_language: &str,
    level: Level,
    input_mode: InputMode,
    task: &WorkshopTask,
    rubric: &WorkshopRubric,
    response: &str,
    cancel: &CancellationToken,
) -> Result<RawRun> {
    let request = StructuredRequest::new(
        Contract::RubricScore,
        system_prompt(first_language),
        vec![ChatMessage::user(user_message(
            level,
            input_mode.as_str(),
            task,
            rubric,
            response,
        ))],
        u32::try_from(500 + 150 * rubric.dimensions.len()).unwrap_or(1500),
    )
    .with_temperature(0.0);
    let started_at = (env.clock)();
    let timer = Instant::now();
    let result: std::result::Result<StructuredOutput, LlmError> =
        env.client.structured(request, cancel.clone()).await;
    CallLog {
        db: &env.db,
        provider_profile_id: env.provider_profile_id,
        call_type: LlmCallType::RubricScore,
        model: &env.model,
        started_at,
        elapsed: timer.elapsed(),
    }
    .structured(&result)
    .await;
    let output = result?;
    let raw: RawRubric =
        serde_json::from_value(output.value).map_err(|_| EngineError::Output("rubric_score"))?;
    Ok(RawRun {
        raw,
        repaired: output.repaired,
        ladder_level: output.ladder_level,
    })
}

/// Runs the rule-based checker off the async worker threads: a grammar engine is
/// CPU work. `None` when no checker is linked or the checker failed. For a voice
/// response, findings about spelling are left out (spec 5.3, X5): a transcript's
/// spelling is the recogniser's, not the learner's. A checker marks those by
/// starting the finding with `Spelling`.
pub(crate) async fn grammar_findings(
    env: &ScorerEnv,
    text: &str,
    input_mode: InputMode,
) -> Option<usize> {
    let grammar = env.grammar.clone()?;
    let owned = text.to_owned();
    match tokio::task::spawn_blocking(move || grammar.findings(&owned)).await {
        Ok(findings) => Some(
            findings
                .iter()
                .filter(|f| {
                    input_mode == InputMode::Text || !f.to_lowercase().starts_with("spelling")
                })
                .count(),
        ),
        Err(error) => {
            tracing::warn!(%error, "the rule-based checker failed");
            None
        }
    }
}

/// One logical run: a call, checked, and one more call when the evidence of a
/// dimension did not hold (X1). A dimension that fails twice is left for review.
pub(crate) async fn checked_run(
    env: &ScorerEnv,
    request: &ScoreRequest,
    findings: Option<usize>,
    cancel: &CancellationToken,
) -> Result<(CheckedRun, LadderLevel)> {
    let words = env.word_levels.as_deref().map(CurriculumWords);
    let check = |run: &RawRun| {
        cross_check(
            &run.raw,
            &CrossCheck {
                response: &request.response,
                level: request.subject.level,
                task: &request.task,
                rubric: &request.rubric,
                grammar_findings: findings,
                word_levels: words
                    .as_ref()
                    .map(|list| list as &dyn assessment_engine::WordList),
                provider_qualified: env.provider_qualified,
                repaired: run.repaired,
                ladder_level: run.ladder_level,
            },
        )
    };
    let call = || {
        call_rubric(
            env,
            &request.first_language,
            request.subject.level,
            request.input_mode,
            &request.task,
            &request.rubric,
            &request.response,
            cancel,
        )
    };
    let first_run = call().await?;
    let first = check(&first_run);
    let degraded = |run: &RawRun| run.repaired || run.ladder_level == LadderLevel::PromptOnly;
    if first.rejected.is_empty() {
        return Ok((
            CheckedRun {
                repaired: degraded(&first_run),
                result: first,
            },
            first_run.ladder_level,
        ));
    }
    let second_run = call().await?;
    let second = check(&second_run);
    let repaired = degraded(&first_run) || degraded(&second_run);
    let ladder = std::cmp::max_by_key(first_run.ladder_level, second_run.ladder_level, |l| {
        l.as_u8()
    });
    Ok((
        CheckedRun {
            result: merge_rerun(first, &second),
            repaired,
        },
        ladder,
    ))
}

/// The metrics stored next to a rubric score: word counts, the vocabulary
/// profile when a list is known, and timing for a spoken response.
fn metrics_json(
    request: &ScoreRequest,
    profile: Option<&VocabularyProfile>,
    findings: Option<usize>,
) -> Value {
    let counts = text_counts(&request.response);
    json!({
        "words": counts.words,
        "distinct_words": counts.distinct_words,
        "sentences": counts.sentences,
        "min_words": request.task.min_words,
        "vocabulary": profile.map(|p| json!({
            "target": p.target.as_str(),
            "considered": p.considered,
            "below": p.below,
            "at": p.at,
            "above": p.above,
            "unlisted": p.unlisted,
            "distinct_above": p.distinct_above,
            "above_share": p.above_share(),
        })),
        "grammar_findings": findings,
        "timing": request.timing,
    })
}

impl RubricScorer {
    pub fn new(env: ScorerEnv) -> Self {
        let recorder = EvidenceRecorder::new(env.db.clone(), env.clock.clone());
        Self { env, recorder }
    }

    pub fn recorder(&self) -> &EvidenceRecorder {
        &self.recorder
    }

    /// The model runs of a request, joined by X6, with the stored metrics.
    async fn outcome(
        &self,
        request: &ScoreRequest,
        cancel: &CancellationToken,
    ) -> Result<(RubricOutcome, u8, Value)> {
        let findings = grammar_findings(&self.env, &request.response, request.input_mode).await;
        let words = self.env.word_levels.as_deref().map(CurriculumWords);
        let profile = words
            .as_ref()
            .map(|list| vocabulary_profile(&request.response, request.subject.level, list));
        let mut checked = Vec::new();
        let mut ladder = 0_u8;
        for _ in 0..request.runs.count() {
            let (run, level) = checked_run(&self.env, request, findings, cancel).await?;
            ladder = ladder.max(level.as_u8());
            checked.push(run);
        }
        let outcome =
            RubricOutcome::join(&checked, self.env.provider_qualified, request.subject.level);
        let metrics = metrics_json(request, profile.as_ref(), findings);
        Ok((outcome, ladder, metrics))
    }

    fn meta<'a>(
        &'a self,
        request: &'a ScoreRequest,
        model: Option<&'a str>,
        metrics: Value,
        ladder: Option<u8>,
    ) -> RubricMeta<'a> {
        RubricMeta {
            rubric: &request.rubric,
            model,
            input_mode: request.input_mode.as_str(),
            response_text: &request.response,
            metrics,
            provider_qualified: self.env.provider_qualified,
            ladder_level: ladder,
        }
    }

    /// Scores one response.
    ///
    /// With a provider that answers, the response is scored and stored. When the
    /// provider cannot be reached, or its output stays unusable after the client's
    /// own repair, the response is stored as `pending_llm` and queued, and the
    /// result says so. A cancelled call queues the response too and returns the
    /// cancellation, so what the learner wrote is never lost. An empty response
    /// is refused before anything is stored or sent.
    pub async fn score(
        &self,
        request: ScoreRequest,
        cancel: &CancellationToken,
    ) -> Result<ScoreResult> {
        if request.response.trim().is_empty() {
            return Err(ActivityError::Empty.into());
        }
        match self.outcome(&request, cancel).await {
            Ok((outcome, ladder, metrics)) => {
                let meta = self.meta(&request, Some(&self.env.model), metrics, Some(ladder));
                let recorded = self
                    .recorder
                    .record_rubric(&request.subject, &meta, &outcome)
                    .await?;
                Ok(ScoreResult::Scored {
                    recorded,
                    outcome: Box::new(outcome),
                })
            }
            Err(error @ EngineError::Llm(LlmError::Cancelled)) => {
                self.queue(&request).await?;
                Err(error)
            }
            Err(EngineError::Llm(_) | EngineError::Output(_)) => {
                let recorded = self.queue(&request).await?;
                Ok(ScoreResult::Queued { recorded })
            }
            Err(other) => Err(other),
        }
    }

    /// Stores a response as waiting for a provider.
    pub async fn queue(&self, request: &ScoreRequest) -> Result<Recorded> {
        let payload = serde_json::to_value(PendingResponse {
            kind: PENDING_KIND.to_owned(),
            request: request.clone(),
        })
        .map_err(|_| EngineError::Output("pending response"))?;
        let meta = self.meta(request, None, Value::Null, None);
        self.recorder
            .queue_rubric(&request.subject, &meta, &payload)
            .await
    }

    /// Scores the responses that waited for a provider, oldest first.
    ///
    /// It stops at the first failure of the connection, so a provider that is
    /// still away is asked once and not once per entry; those entries stay queued
    /// untouched. An entry whose output the model keeps getting wrong is given up
    /// on after [`MAX_OUTPUT_TRIES`]: its attempts become `needs_review` and it
    /// leaves the queue. A cancelled call returns the cancellation.
    pub async fn score_pending_backlog(&self, cancel: &CancellationToken) -> Result<BacklogReport> {
        let mut report = BacklogReport::default();
        let entries = self
            .env
            .db
            .pending_scoring()
            .oldest_of_kind(PENDING_KIND, BACKLOG_BATCH)
            .await?;
        for entry in entries {
            let Ok(pending) = serde_json::from_value::<PendingResponse>(entry.payload.clone())
            else {
                report.skipped += 1;
                continue;
            };
            let request = pending.request;
            let attempts = self
                .env
                .db
                .attempts()
                .by_response(&request.subject.response_id)
                .await?;
            if attempts.is_empty() {
                report.skipped += 1;
                continue;
            }
            match self.outcome(&request, cancel).await {
                Ok((outcome, ladder, metrics)) => {
                    let meta = self.meta(&request, Some(&self.env.model), metrics, Some(ladder));
                    self.recorder
                        .complete_rubric(&attempts, &request.subject, &meta, &outcome)
                        .await?;
                    self.env.db.pending_scoring().remove(entry.id).await?;
                    report.scored += 1;
                }
                Err(error @ EngineError::Llm(LlmError::Cancelled)) => return Err(error),
                Err(EngineError::Llm(LlmError::InvalidOutput(_) | LlmError::Refusal))
                | Err(EngineError::Output(_)) => {
                    self.env.db.pending_scoring().record_try(entry.id).await?;
                    if entry.tries + 1 >= MAX_OUTPUT_TRIES {
                        self.give_up(&attempts, entry.tries + 1).await?;
                        self.env.db.pending_scoring().remove(entry.id).await?;
                        report.given_up += 1;
                    }
                }
                Err(EngineError::Llm(_)) => {
                    report.provider_unreachable = true;
                    break;
                }
                Err(other) => return Err(other),
            }
        }
        report.remaining = self
            .env
            .db
            .pending_scoring()
            .oldest_of_kind(PENDING_KIND, BACKLOG_BATCH)
            .await?
            .len();
        Ok(report)
    }

    /// Marks the attempts of a response the model could not score as needing
    /// review. They never count.
    async fn give_up(&self, attempts: &[Attempt], tries: i64) -> Result<()> {
        let now = (self.env.clock)();
        for attempt in attempts {
            self.env
                .db
                .attempts()
                .update_score(
                    attempt.id,
                    &ScoreUpdate {
                        scorer_version: attempt.scorer_version.clone(),
                        raw_score: None,
                        max_score: None,
                        normalized: None,
                        confidence: None,
                        status: AttemptStatus::NeedsReview,
                    },
                )
                .await?;
            if attempt.counts_toward_estimate {
                self.env
                    .db
                    .attempts()
                    .set_counts_toward_estimate(attempt.id, false)
                    .await?;
            }
        }
        if let Some(first) = attempts.first() {
            self.env
                .db
                .evidence()
                .add(&NewEvidence {
                    attempt_id: first.id,
                    kind: EvidenceKind::ScorerReason,
                    content: Some(format!(
                        "The model's output was unusable after {tries} tries, so the response needs review."
                    )),
                    data: None,
                    created_at: now,
                })
                .await?;
        }
        Ok(())
    }
}
