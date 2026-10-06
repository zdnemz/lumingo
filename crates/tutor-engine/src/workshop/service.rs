//! The workshop service: layers one to three of a draft's feedback, the
//! comparison with the earlier draft, and the pending queue.
//!
//! Layer one (the rule-based findings) needs no network and is returned by
//! [`Workshop::submit_draft`] before any provider call. Layer two (structured
//! errors, T2) and layer three (rubric bands, T3) come from
//! [`Workshop::analyse_draft`]. When a provider cannot be reached the draft is
//! already stored as a turn, and a pending entry carries everything needed to
//! finish later; [`process_pending_drafts`] does that. Workshop work is practice:
//! its attempts have origin `free_mode` and never count toward an estimate.

use std::sync::Arc;

use assessment_engine::{Level, Origin};
use curriculum::validate::{GrammarCheck, WordLevels};
use llm_client::{LlmClient, LlmError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use storage::{Attempt, Database, InputMode, NewSession, NewTurn, SessionStatus, TurnRole};
use tokio_util::sync::CancellationToken;

use crate::analysis::{
    AnalysedRecord, AnalysisKind, AnalyzerConfig, ErrorFinding, InputMode as AnalysisInput,
    RawAnalysis, TurnAnalyzer, TurnToAnalyse,
};
use crate::drafts::{DraftComparison, DraftError, compare_drafts};
use crate::error::{EngineError, Result};
use crate::evidence::{EvidenceRecorder, RubricMeta, Subject, rubric_scorer_version};
use crate::rubric::{
    CheckedRun, InputMode as RubricInputMode, RubricOutcome, RubricResult, Runs, ScoreRequest,
    ScorerEnv, WorkshopRubric, WorkshopTask, checked_run, grammar_findings,
};
use crate::support::Clock;

/// What the workshop works with. Cheap to clone.
#[derive(Clone)]
pub struct WorkshopEnv {
    pub client: Arc<dyn LlmClient>,
    pub db: Database,
    pub clock: Clock,
    pub model: String,
    pub provider_profile_id: Option<i64>,
    /// The rule-based checker. Harper is wired in a later task; any
    /// implementation of the trait works.
    pub grammar: Arc<dyn GrammarCheck + Send + Sync>,
    /// Word levels for the range cross-check (X4). Optional.
    pub word_levels: Option<Arc<WordLevels>>,
    /// Whether the provider passed scorer qualification. It sets the starting
    /// confidence of a rubric attempt.
    pub provider_qualified: bool,
}

#[derive(Debug, Clone)]
pub struct WorkshopConfig {
    pub profile_id: i64,
    /// The level the task is written at, never a value a model produced.
    pub level: Level,
    pub first_language: String,
    pub app_version: String,
    /// The topic-bank prompt the learner picked, if any.
    pub prompt_id: Option<String>,
    /// Without a rubric the workshop gives layers one and two only.
    pub rubric: Option<WorkshopRubric>,
    pub task: WorkshopTask,
}

/// What `submit_draft` returns at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSubmission {
    pub turn_id: i64,
    pub turn_seq: i64,
    pub text: String,
    pub words: usize,
    /// Layer one: findings of the rule-based checker.
    pub rule_findings: Vec<String>,
    /// The draft is shorter than the task's minimum.
    pub below_minimum: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStatus {
    /// Layers two and three are done (or no rubric is set, so layer three does not exist).
    Analysed,
    /// The errors are in, the rubric bands wait for a provider.
    RubricPending,
    /// Nothing from a provider yet: the draft waits in the pending queue.
    Pending,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DraftFeedback {
    pub turn_id: i64,
    pub turn_seq: i64,
    pub status: DraftStatus,
    /// Layer two, after the filters.
    pub analysis: Option<AnalysedRecord>,
    /// Against the earlier analysed draft of the session, when there is one.
    pub comparison: Option<DraftComparison>,
    /// Layer three.
    pub rubric: Option<RubricResult>,
}

/// The pending entry's payload: everything needed to finish a draft's feedback
/// without the session. It holds the learner's text, so it goes away with the
/// session like the turn does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PendingDraft {
    kind: String,
    profile_id: i64,
    session_id: i64,
    turn_id: i64,
    turn_seq: i64,
    text: String,
    level: Level,
    first_language: String,
    response_id: String,
    rubric: Option<WorkshopRubric>,
    task: WorkshopTask,
}

const PENDING_KIND: &str = "writing_draft";
/// Entries read from the queue per pass.
const PENDING_BATCH: u32 = 50;

pub struct Workshop {
    env: WorkshopEnv,
    config: WorkshopConfig,
    session_id: i64,
    analyzer: TurnAnalyzer,
}

fn analyzer_for(
    env: &WorkshopEnv,
    profile_id: i64,
    session_id: i64,
    level: Level,
    first_language: &str,
) -> TurnAnalyzer {
    TurnAnalyzer::new(
        AnalyzerConfig {
            profile_id,
            session_id,
            provider_profile_id: env.provider_profile_id,
            model: env.model.clone(),
            level,
            first_language: first_language.to_owned(),
            kind: AnalysisKind::Draft,
            objectives: Vec::new(),
            target_language: Vec::new(),
        },
        env.client.clone(),
        env.db.clone(),
        env.clock.clone(),
    )
}

/// Runs the checker off the async worker threads: a grammar engine is CPU work.
async fn rule_findings(env: &WorkshopEnv, text: &str) -> Vec<String> {
    let grammar = env.grammar.clone();
    let owned = text.to_owned();
    match tokio::task::spawn_blocking(move || grammar.findings(&owned)).await {
        Ok(findings) => findings,
        Err(error) => {
            tracing::warn!(%error, "the rule-based checker failed");
            Vec::new()
        }
    }
}

fn response_id(session_id: i64, turn_seq: i64) -> String {
    format!("workshop-{session_id}-{turn_seq}")
}

impl Workshop {
    pub async fn start(env: WorkshopEnv, config: WorkshopConfig) -> Result<Self> {
        let stored = env
            .db
            .sessions()
            .create(&NewSession {
                profile_id: config.profile_id,
                kind: storage::SessionKind::Writing,
                unit_id: None,
                activity_id: config.prompt_id.clone(),
                mode: None,
                provider_profile_id: env.provider_profile_id,
                app_version: config.app_version.clone(),
                started_at: (env.clock)(),
            })
            .await?;
        let analyzer = analyzer_for(
            &env,
            config.profile_id,
            stored.id,
            config.level,
            &config.first_language,
        );
        Ok(Self {
            env,
            config,
            session_id: stored.id,
            analyzer,
        })
    }

    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    /// Stores a draft as a turn and returns the rule-based findings. No
    /// provider is involved, so this works offline.
    pub async fn submit_draft(&self, text: &str) -> Result<DraftSubmission> {
        let text = text.trim();
        if text.is_empty() {
            return Err(EngineError::Refused("the draft is empty"));
        }
        let words = text.split_whitespace().count();
        let turn = self
            .env
            .db
            .turns()
            .append(&NewTurn {
                session_id: self.session_id,
                role: TurnRole::Learner,
                input_mode: InputMode::Text,
                text: text.to_owned(),
                stt_text: None,
                edited_by_learner: false,
                speech_ms: None,
                pause_ms: None,
                word_count: Some(i64::try_from(words).unwrap_or(i64::MAX)),
                created_at: (self.env.clock)(),
            })
            .await?;
        let mut findings = rule_findings(&self.env, text).await;
        findings.dedup();
        let below_minimum = self
            .config
            .task
            .min_words
            .is_some_and(|min| words < usize::try_from(min).unwrap_or(usize::MAX));
        Ok(DraftSubmission {
            turn_id: turn.id,
            turn_seq: turn.seq,
            text: text.to_owned(),
            words,
            rule_findings: findings,
            below_minimum,
        })
    }

    /// Layers two and three for a stored draft. Without a reachable provider
    /// the draft is queued and the returned status says so; only a cancelled
    /// call and storage failures are errors.
    pub async fn analyse_draft(
        &self,
        submission: &DraftSubmission,
        cancel: &CancellationToken,
    ) -> Result<DraftFeedback> {
        let pending = PendingDraft {
            kind: PENDING_KIND.to_owned(),
            profile_id: self.config.profile_id,
            session_id: self.session_id,
            turn_id: submission.turn_id,
            turn_seq: submission.turn_seq,
            text: submission.text.clone(),
            level: self.config.level,
            first_language: self.config.first_language.clone(),
            response_id: response_id(self.session_id, submission.turn_seq),
            rubric: self.config.rubric.clone(),
            task: self.config.task.clone(),
        };
        let mut feedback = DraftFeedback {
            turn_id: submission.turn_id,
            turn_seq: submission.turn_seq,
            status: DraftStatus::Analysed,
            analysis: None,
            comparison: None,
            rubric: None,
        };

        match run_errors(&self.env, &self.analyzer, &pending, cancel).await {
            Ok((record, comparison)) => {
                feedback.analysis = Some(record);
                feedback.comparison = comparison;
            }
            Err(EngineError::Llm(LlmError::Cancelled)) => {
                return Err(EngineError::Llm(LlmError::Cancelled));
            }
            Err(EngineError::Llm(_) | EngineError::Output(_)) => {
                enqueue_pending(&self.env, &pending).await?;
                feedback.status = DraftStatus::Pending;
                return Ok(feedback);
            }
            Err(other) => return Err(other),
        }

        if pending.rubric.is_some() {
            match run_rubric(&self.env, &pending, cancel).await {
                Ok(run) => {
                    write_scores(&self.env, &pending, &run, &[]).await?;
                    feedback.rubric = Some(run.result);
                }
                Err(error @ EngineError::Llm(LlmError::Cancelled)) => {
                    enqueue_pending(&self.env, &pending).await?;
                    return Err(error);
                }
                Err(EngineError::Llm(_) | EngineError::Output(_)) => {
                    enqueue_pending(&self.env, &pending).await?;
                    feedback.status = DraftStatus::RubricPending;
                }
                Err(other) => return Err(other),
            }
        }
        Ok(feedback)
    }

    /// Ends the session. Drafts still waiting for a provider stay in the
    /// pending queue and are finished by [`process_pending_drafts`] later.
    pub async fn finish(&self, aborted: bool) -> Result<()> {
        let turns = self.env.db.turns().list(self.session_id).await?;
        let drafts = turns.iter().filter(|t| t.role == TurnRole::Learner).count();
        let waiting = self
            .env
            .db
            .pending_scoring()
            .oldest(1000)
            .await?
            .iter()
            .filter(|p| {
                p.payload.get("session_id").and_then(Value::as_i64) == Some(self.session_id)
            })
            .count();
        let summary = json!({ "drafts": drafts, "awaiting_feedback": waiting });
        let status = if aborted {
            SessionStatus::Aborted
        } else {
            SessionStatus::Completed
        };
        self.env
            .db
            .sessions()
            .finish(self.session_id, status, &(self.env.clock)(), Some(&summary))
            .await?;
        Ok(())
    }
}

/// The errors of the nearest earlier analysed draft of the session.
async fn earlier_errors(
    db: &Database,
    session_id: i64,
    before_seq: i64,
) -> Result<Option<Vec<DraftError>>> {
    let turns = db.turns().list(session_id).await?;
    for turn in turns
        .iter()
        .rev()
        .filter(|t| t.role == TurnRole::Learner && t.seq < before_seq)
    {
        if let Some(stored) = db.analysis().get(turn.id).await?
            && let Ok(parsed) = serde_json::from_value::<RawAnalysis>(stored.analysis)
        {
            let errors = parsed
                .turns
                .into_iter()
                .flat_map(|t| t.errors)
                .map(to_draft_error)
                .collect();
            return Ok(Some(errors));
        }
    }
    Ok(None)
}

fn to_draft_error(error: ErrorFinding) -> DraftError {
    DraftError {
        quote: error.quote,
        category: error.category,
    }
}

/// Layer two for one draft, and its comparison with the earlier one.
async fn run_errors(
    env: &WorkshopEnv,
    analyzer: &TurnAnalyzer,
    pending: &PendingDraft,
    cancel: &CancellationToken,
) -> Result<(AnalysedRecord, Option<DraftComparison>)> {
    let record = analyzer
        .analyse_now(
            TurnToAnalyse {
                turn_id: pending.turn_id,
                turn_seq: pending.turn_seq,
                input_mode: AnalysisInput::Text,
                tutor_before: String::new(),
                learner_text: pending.text.clone(),
                tutor_reply: String::new(),
            },
            cancel,
        )
        .await?;
    let earlier = earlier_errors(&env.db, pending.session_id, pending.turn_seq).await?;
    let comparison = earlier.map(|first| {
        let second: Vec<DraftError> = record
            .analysis
            .errors
            .iter()
            .cloned()
            .map(to_draft_error)
            .collect();
        compare_drafts(&first, &pending.text, &second)
    });
    Ok((record, comparison))
}

/// The scorer's view of the workshop's environment.
fn scorer_env(env: &WorkshopEnv) -> ScorerEnv {
    ScorerEnv {
        client: env.client.clone(),
        db: env.db.clone(),
        clock: env.clock.clone(),
        model: env.model.clone(),
        provider_profile_id: env.provider_profile_id,
        grammar: Some(env.grammar.clone()),
        word_levels: env.word_levels.clone(),
        provider_qualified: env.provider_qualified,
    }
}

/// The subject of a draft's attempts: workshop work is practice.
fn subject_of(pending: &PendingDraft) -> Subject {
    Subject::new(
        pending.profile_id,
        Some(pending.session_id),
        None,
        "writing_workshop",
        "writing_workshop",
        pending.level,
        "writing",
        Origin::FreeMode,
        pending.response_id.clone(),
    )
}

/// Layer three: one run, and one rerun when the evidence of a dimension did not
/// hold (X1). The run is the shared one of the rubric scorer.
async fn run_rubric(
    env: &WorkshopEnv,
    pending: &PendingDraft,
    cancel: &CancellationToken,
) -> Result<CheckedRun> {
    let Some(rubric) = &pending.rubric else {
        return Err(EngineError::Refused("no rubric is set for this task"));
    };
    let scorer = scorer_env(env);
    let request = ScoreRequest {
        subject: subject_of(pending),
        rubric: rubric.clone(),
        task: pending.task.clone(),
        response: pending.text.clone(),
        input_mode: RubricInputMode::Text,
        runs: Runs::One,
        first_language: pending.first_language.clone(),
        timing: None,
    };
    let findings = grammar_findings(&scorer, &pending.text, RubricInputMode::Text).await;
    let (run, _) = checked_run(&scorer, &request, findings, cancel).await?;
    Ok(run)
}

fn scorer_version(env: &WorkshopEnv, rubric: &WorkshopRubric) -> String {
    rubric_scorer_version(rubric, Some(&env.model))
}

/// Stores the rubric result: fills in the waiting attempts when there are some,
/// stores new ones otherwise, with the evidence.
async fn write_scores(
    env: &WorkshopEnv,
    pending: &PendingDraft,
    run: &CheckedRun,
    existing: &[Attempt],
) -> Result<()> {
    let Some(rubric) = &pending.rubric else {
        return Ok(());
    };
    let recorder = EvidenceRecorder::new(env.db.clone(), env.clock.clone());
    let subject = subject_of(pending);
    let outcome = RubricOutcome::join(
        std::slice::from_ref(run),
        env.provider_qualified,
        pending.level,
    );
    let meta = RubricMeta {
        rubric,
        model: Some(&env.model),
        input_mode: "text",
        response_text: &pending.text,
        metrics: serde_json::Value::Null,
        provider_qualified: env.provider_qualified,
        ladder_level: None,
    };
    if existing.is_empty() {
        recorder.record_rubric(&subject, &meta, &outcome).await?;
    } else {
        recorder
            .complete_rubric(existing, &subject, &meta, &outcome)
            .await?;
    }
    Ok(())
}

/// Queues a draft: placeholder attempts (one per rubric dimension, or one when
/// there is no rubric) with the status `pending_llm`, and the queue entry on the
/// first of them. Calling it twice for the same draft does nothing the second time.
async fn enqueue_pending(env: &WorkshopEnv, pending: &PendingDraft) -> Result<()> {
    if !env
        .db
        .attempts()
        .by_response(&pending.response_id)
        .await?
        .is_empty()
    {
        // The rubric attempts of a draft whose layer three failed after layer
        // two was stored may already exist only if a queue entry does too.
        return Ok(());
    }
    let version = pending.rubric.as_ref().map_or_else(
        || "writing_workshop/1".to_owned(),
        |r| scorer_version(env, r),
    );
    let dimensions: Vec<&str> = match &pending.rubric {
        Some(rubric) => rubric
            .dimensions
            .iter()
            .map(|d| d.dimension.as_str())
            .collect(),
        None => vec!["draft_feedback"],
    };
    let payload =
        serde_json::to_value(pending).map_err(|_| EngineError::Output("pending draft"))?;
    EvidenceRecorder::new(env.db.clone(), env.clock.clone())
        .queue(
            &subject_of(pending),
            &version,
            &dimensions,
            &pending.text,
            &payload,
        )
        .await?;
    Ok(())
}

/// A draft whose feedback was finished from the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletedDraft {
    pub session_id: i64,
    pub feedback: DraftFeedback,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingReport {
    pub completed: Vec<CompletedDraft>,
    /// Entries still waiting.
    pub remaining: usize,
}

/// Finishes the feedback of drafts that waited for a provider, oldest first.
/// It stops at the first provider failure, so a provider that is still away is
/// asked once, not once per draft. A draft whose errors were already stored keeps
/// them, and only its rubric is run.
pub async fn process_pending_drafts(
    env: &WorkshopEnv,
    cancel: &CancellationToken,
) -> Result<PendingReport> {
    let entries = env.db.pending_scoring().oldest(PENDING_BATCH).await?;
    let mut completed = Vec::new();
    let mut remaining = entries.len();
    for entry in entries {
        let Ok(pending) = serde_json::from_value::<PendingDraft>(entry.payload.clone()) else {
            continue;
        };
        if pending.kind != PENDING_KIND {
            continue;
        }
        let analyzer = analyzer_for(
            env,
            pending.profile_id,
            pending.session_id,
            pending.level,
            &pending.first_language,
        );
        let mut feedback = DraftFeedback {
            turn_id: pending.turn_id,
            turn_seq: pending.turn_seq,
            status: DraftStatus::Analysed,
            analysis: None,
            comparison: None,
            rubric: None,
        };
        let outcome: Result<()> = async {
            if env.db.analysis().get(pending.turn_id).await?.is_none() {
                let (record, comparison) = run_errors(env, &analyzer, &pending, cancel).await?;
                feedback.analysis = Some(record);
                feedback.comparison = comparison;
            }
            if pending.rubric.is_some() {
                let run = run_rubric(env, &pending, cancel).await?;
                let rows = env.db.attempts().by_response(&pending.response_id).await?;
                write_scores(env, &pending, &run, &rows).await?;
                feedback.rubric = Some(run.result);
            } else {
                let rows = env.db.attempts().by_response(&pending.response_id).await?;
                EvidenceRecorder::new(env.db.clone(), env.clock.clone())
                    .finish_unscored(&rows, "The draft had no rubric, so there are no bands.")
                    .await?;
            }
            Ok(())
        }
        .await;
        match outcome {
            Ok(()) => {
                env.db.pending_scoring().remove(entry.id).await?;
                remaining -= 1;
                completed.push(CompletedDraft {
                    session_id: pending.session_id,
                    feedback,
                });
            }
            Err(EngineError::Llm(LlmError::Cancelled)) => {
                return Err(EngineError::Llm(LlmError::Cancelled));
            }
            Err(EngineError::Llm(_) | EngineError::Output(_)) => {
                env.db.pending_scoring().record_try(entry.id).await?;
                break;
            }
            Err(other) => return Err(other),
        }
    }
    Ok(PendingReport {
        completed,
        remaining,
    })
}
