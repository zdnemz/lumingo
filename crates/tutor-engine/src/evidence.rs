//! Evidence recording (S5-01): the one place that writes attempts and evidence.
//!
//! Every scorer goes through [`EvidenceRecorder`]: the deterministic scorers of
//! the activity runtime, the rubric scorer, the pronunciation engine, and the
//! free modes. That gives the rules of `docs/ASSESSMENT_SPEC.md` section 3 one
//! home.
//!
//! * One attempt row per scored dimension of one response. The rows of a response
//!   share a `response_id`, so a four-dimension essay is one observation.
//! * `scorer` and `scorer_version` are on every row. The first row of a response
//!   also gets a `metric` evidence row with the versions in full: the answer
//!   normaliser (`norm/1`), the scoring algorithm, the contract for a rubric, and
//!   the id, version and model checksum of every speech engine that took part.
//!   Quotes and reasons are per dimension, on that dimension's row. The response
//!   text is evidence too, kept on the first row, and goes with the session.
//! * `counts_toward_estimate` is decided here and nowhere else: only authored
//!   work counts, only a scored or waiting attempt counts, and an attempt below
//!   the confidence floor does not. Generated work, the free modes and free-speech
//!   pronunciation hints never count. Work that is authored but practice (a replay
//!   of an item the learner already answered) is marked with
//!   [`Subject::never_counts`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use assessment_engine::{Level, NORM_VERSION, Origin};
use pron_engine::{PhonemeResult, UtteranceReport, WordResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use speech::EngineInfo;
use storage::{
    Attempt, AttemptOrigin, AttemptStatus, Database, EvidenceKind, NewAttempt, NewEvidence,
    NewPronResult, PronMode, ScoreUpdate, Scorer, Timestamp,
};

use crate::error::{EngineError, Result};
use crate::rubric::{
    DimensionStatus, RUBRIC_ALGORITHM_VERSION, RUBRIC_SCORE_VERSION, RubricOutcome, WorkshopRubric,
};
use crate::support::{Clock, storage_level};

/// Attempts below this confidence are stored and never count (spec section 5.4).
pub const CONFIDENCE_FLOOR: f64 = 0.3;

/// A pronunciation drill's confidence never exceeds this (spec section 7.2).
pub const PRON_CONFIDENCE_CAP: f64 = 0.6;

/// The version of the evidence layout written by this recorder.
pub const RECORDER_VERSION: &str = "evidence/1";

/// The version of the pronunciation scoring path: how a report becomes an
/// attempt, a confidence and result rows.
pub const PRON_ALGORITHM_VERSION: &str = "pron_gop/1";

/// What kind of speech engine took part in a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineRole {
    /// Speech recognition produced the transcript that was scored.
    Stt,
    /// Speech synthesis produced the audio the learner heard.
    Tts,
    /// The pronunciation engine scored the audio.
    Pron,
}

/// A speech engine that took part in a response, as stored with its evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStamp {
    pub role: EngineRole,
    pub id: String,
    pub version: String,
    /// SHA-256 of the model files, when the engine loads a model.
    pub model_checksum: Option<String>,
}

impl EngineStamp {
    pub fn new(role: EngineRole, info: &EngineInfo) -> Self {
        Self {
            role,
            id: info.id.clone(),
            version: info.version.clone(),
            model_checksum: info.model_checksum.clone(),
        }
    }
}

/// Who answered what: everything an attempt row carries besides the score.
#[derive(Debug, Clone, PartialEq)]
pub struct Subject {
    pub profile_id: i64,
    pub session_id: Option<i64>,
    pub unit_id: Option<String>,
    pub activity_id: String,
    pub activity_type: String,
    /// The level of the unit or block the item belongs to, from authored
    /// content and never from a model.
    pub level: Level,
    /// `listening`, `reading`, `speaking`, `writing`, or a supporting dimension
    /// (`grammar`, `vocabulary`, `pronunciation`).
    pub skill: String,
    pub origin: Origin,
    /// Shared by every dimension row of this response.
    pub response_id: String,
    /// Speech engines that took part, for the metric evidence.
    pub engines: Vec<EngineStamp>,
    eligible: bool,
}

impl Subject {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        profile_id: i64,
        session_id: Option<i64>,
        unit_id: Option<String>,
        activity_id: impl Into<String>,
        activity_type: impl Into<String>,
        level: Level,
        skill: impl Into<String>,
        origin: Origin,
        response_id: impl Into<String>,
    ) -> Self {
        Self {
            profile_id,
            session_id,
            unit_id,
            activity_id: activity_id.into(),
            activity_type: activity_type.into(),
            level,
            skill: skill.into(),
            origin,
            response_id: response_id.into(),
            engines: Vec::new(),
            eligible: origin == Origin::Authored,
        }
    }

    #[must_use]
    pub fn with_engine(mut self, stamp: EngineStamp) -> Self {
        self.engines.push(stamp);
        self
    }

    /// Marks authored work that is practice and must not count, such as the
    /// replay of an item the learner already answered in the unit.
    #[must_use]
    pub fn never_counts(mut self) -> Self {
        self.eligible = false;
        self
    }

    /// Whether this subject may count at all, before the status and confidence
    /// of the score are known.
    pub fn may_count(&self) -> bool {
        self.eligible && self.origin == Origin::Authored
    }
}

fn storage_origin(origin: Origin) -> AttemptOrigin {
    match origin {
        Origin::Authored => AttemptOrigin::Authored,
        Origin::Generated => AttemptOrigin::Generated,
        Origin::FreeMode => AttemptOrigin::FreeMode,
    }
}

/// The rule for `counts_toward_estimate` (spec section 3).
pub fn counts_toward_estimate(
    may_count: bool,
    status: AttemptStatus,
    confidence: Option<f64>,
) -> bool {
    may_count
        && matches!(status, AttemptStatus::Scored | AttemptStatus::PendingLlm)
        && confidence.is_none_or(|c| c >= CONFIDENCE_FLOOR)
}

/// The rows written for one response.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub response_id: String,
    pub attempts: Vec<Attempt>,
}

impl Recorded {
    /// The first attempt, which carries the response text and the metric evidence.
    pub fn first(&self) -> Option<&Attempt> {
        self.attempts.first()
    }
}

/// A score from a deterministic scorer.
#[derive(Debug, Clone, PartialEq)]
pub struct DeterministicRecord {
    pub normalized: f64,
    pub raw: f64,
    pub max: f64,
    /// The scoring rule and its version, for example `gap_fill/1`.
    pub algorithm: &'static str,
    /// What the learner submitted, in readable form.
    pub response_text: Option<String>,
    /// Counts and flags of this score, stored as metric data.
    pub details: Value,
    /// One-line notes, such as a spelling note, stored as scorer reasons.
    pub notes: Vec<String>,
}

/// What the rubric rows need to know besides the outcome.
#[derive(Debug, Clone)]
pub struct RubricMeta<'a> {
    pub rubric: &'a WorkshopRubric,
    /// The model that scored, or `None` while the response waits for one.
    pub model: Option<&'a str>,
    /// `text` or `voice`.
    pub input_mode: &'a str,
    pub response_text: &'a str,
    /// Word counts, vocabulary profile and timing of the response, stored as
    /// metric data next to the scores.
    pub metrics: Value,
    /// Whether the provider passed scorer qualification.
    pub provider_qualified: bool,
    /// The ladder level of the structured call, 1 to 4.
    pub ladder_level: Option<u8>,
}

/// The version stored with a rubric attempt: rubric id and version, contract
/// version, model name (spec section 3).
pub fn rubric_scorer_version(rubric: &WorkshopRubric, model: Option<&str>) -> String {
    format!(
        "{}/{}+{RUBRIC_SCORE_VERSION}+{}",
        rubric.id,
        rubric.version,
        model.unwrap_or("unscored")
    )
}

/// A pronunciation report to store.
#[derive(Debug, Clone)]
pub struct PronRecord<'a> {
    pub report: &'a UtteranceReport,
    pub engine: EngineInfo,
    /// The version of the threshold set the report was produced with.
    pub threshold_set_version: String,
    pub reference_text: &'a str,
    pub mode: PronMode,
    /// Milliseconds per posterior frame, to turn aligned frames into times.
    pub frame_ms: u32,
}

/// Writes attempts and evidence. Cheap to clone.
#[derive(Clone)]
pub struct EvidenceRecorder {
    db: Database,
    clock: Clock,
    counter: Arc<AtomicU64>,
}

fn attempt_row(
    subject: &Subject,
    dimension: &str,
    scorer: Scorer,
    scorer_version: &str,
    now: Timestamp,
) -> NewAttempt {
    NewAttempt {
        profile_id: subject.profile_id,
        session_id: subject.session_id,
        unit_id: subject.unit_id.clone(),
        activity_id: subject.activity_id.clone(),
        activity_type: subject.activity_type.clone(),
        response_id: subject.response_id.clone(),
        origin: storage_origin(subject.origin),
        level: storage_level(subject.level),
        skill: subject.skill.clone(),
        dimension: dimension.to_owned(),
        scorer,
        scorer_version: scorer_version.to_owned(),
        raw_score: None,
        max_score: None,
        normalized: None,
        confidence: None,
        status: AttemptStatus::Insufficient,
        counts_toward_estimate: false,
        created_at: now,
    }
}

fn engines_json(engines: &[EngineStamp]) -> Value {
    Value::Array(
        engines
            .iter()
            .map(|e| {
                json!({
                    "role": e.role,
                    "id": e.id,
                    "version": e.version,
                    "model_checksum": e.model_checksum,
                })
            })
            .collect(),
    )
}

/// The metric evidence that makes a score traceable: who scored, with which
/// versions, with which engines.
fn provenance(
    scorer: Scorer,
    scorer_version: &str,
    algorithm: &str,
    engines: &[EngineStamp],
    details: Value,
) -> Value {
    json!({
        "recorder": RECORDER_VERSION,
        "scorer": scorer.as_str(),
        "scorer_version": scorer_version,
        "norm_version": NORM_VERSION,
        "algorithm_version": algorithm,
        "engines": engines_json(engines),
        "details": details,
    })
}

impl EvidenceRecorder {
    pub fn new(db: Database, clock: Clock) -> Self {
        Self {
            db,
            clock,
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    /// An id for one submitted response: unique within the process, and across
    /// runs through the timestamp. Retrying an activity makes a new response.
    pub fn new_response_id(&self, activity_id: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{activity_id}@{}#{n}", (self.clock)())
    }

    async fn evidence(
        &self,
        attempt_id: i64,
        kind: EvidenceKind,
        content: Option<String>,
        data: Option<Value>,
        now: Timestamp,
    ) -> Result<()> {
        self.db
            .evidence()
            .add(&NewEvidence {
                attempt_id,
                kind,
                content,
                data,
                created_at: now,
            })
            .await?;
        Ok(())
    }

    /// Stores a deterministic score: one attempt, scored, confidence 1.0.
    pub async fn record_deterministic(
        &self,
        subject: &Subject,
        record: &DeterministicRecord,
    ) -> Result<Recorded> {
        if !(0.0..=1.0).contains(&record.normalized) {
            return Err(EngineError::Refused("a score is between 0 and 1"));
        }
        let now = (self.clock)();
        let mut row = attempt_row(subject, "overall", Scorer::Deterministic, NORM_VERSION, now);
        row.raw_score = Some(record.raw);
        row.max_score = Some(record.max);
        row.normalized = Some(record.normalized);
        row.confidence = Some(1.0);
        row.status = AttemptStatus::Scored;
        row.counts_toward_estimate =
            counts_toward_estimate(subject.may_count(), row.status, row.confidence);
        let stored = self.db.attempts().insert_response(&[row]).await?;
        let Some(first) = stored.first() else {
            return Err(EngineError::Refused("no attempt was stored"));
        };
        if let Some(text) = &record.response_text {
            self.evidence(
                first.id,
                EvidenceKind::ResponseText,
                Some(text.clone()),
                None,
                now,
            )
            .await?;
        }
        self.evidence(
            first.id,
            EvidenceKind::Metric,
            None,
            Some(provenance(
                Scorer::Deterministic,
                NORM_VERSION,
                record.algorithm,
                &subject.engines,
                record.details.clone(),
            )),
            now,
        )
        .await?;
        for note in &record.notes {
            self.evidence(
                first.id,
                EvidenceKind::ScorerReason,
                Some(note.clone()),
                None,
                now,
            )
            .await?;
        }
        Ok(Recorded {
            response_id: subject.response_id.clone(),
            attempts: stored,
        })
    }

    /// Stores a response that has no score and says why: a task that has no
    /// scorer (a shadowing run, a roleplay with scoring none), or one that could
    /// not be scored (no rubric, no pronunciation engine). The row has status
    /// `insufficient` or `rejected`, no score and never counts, so the response
    /// stays traceable without pretending to be evidence.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_unscored(
        &self,
        subject: &Subject,
        scorer: Scorer,
        scorer_version: &str,
        dimension: &str,
        status: AttemptStatus,
        reason: &str,
        response_text: Option<&str>,
        details: Value,
    ) -> Result<Recorded> {
        if !matches!(
            status,
            AttemptStatus::Insufficient | AttemptStatus::Rejected
        ) {
            return Err(EngineError::Refused(
                "an unscored response is insufficient or rejected",
            ));
        }
        let now = (self.clock)();
        let mut row = attempt_row(subject, dimension, scorer, scorer_version, now);
        row.status = status;
        let stored = self.db.attempts().insert_response(&[row]).await?;
        let Some(first) = stored.first() else {
            return Err(EngineError::Refused("no attempt was stored"));
        };
        if let Some(text) = response_text {
            self.evidence(
                first.id,
                EvidenceKind::ResponseText,
                Some(text.to_owned()),
                None,
                now,
            )
            .await?;
        }
        self.evidence(
            first.id,
            EvidenceKind::Metric,
            None,
            Some(provenance(
                scorer,
                scorer_version,
                "unscored/1",
                &subject.engines,
                details,
            )),
            now,
        )
        .await?;
        self.evidence(
            first.id,
            EvidenceKind::ScorerReason,
            Some(reason.to_owned()),
            None,
            now,
        )
        .await?;
        Ok(Recorded {
            response_id: subject.response_id.clone(),
            attempts: stored,
        })
    }

    fn dimension_names(rubric: &WorkshopRubric) -> Vec<&'static str> {
        rubric
            .dimensions
            .iter()
            .map(|d| d.dimension.as_str())
            .collect()
    }

    /// Queues a productive response for the rubric scorer: one `pending_llm`
    /// attempt per rubric dimension and one entry in the pending queue, on the
    /// first of them. The payload must carry everything the scorer needs.
    ///
    /// Authored responses are stored as counting: the status keeps them out of
    /// every estimate until a score arrives, and [`Self::complete_rubric`] turns
    /// the flag off if the confidence turns out to be below the floor.
    pub async fn queue_rubric(
        &self,
        subject: &Subject,
        meta: &RubricMeta<'_>,
        payload: &Value,
    ) -> Result<Recorded> {
        let now = (self.clock)();
        let version = rubric_scorer_version(meta.rubric, meta.model);
        let rows: Vec<NewAttempt> = Self::dimension_names(meta.rubric)
            .into_iter()
            .map(|dimension| {
                let mut row = attempt_row(subject, dimension, Scorer::RubricLlm, &version, now);
                row.status = AttemptStatus::PendingLlm;
                row.counts_toward_estimate =
                    counts_toward_estimate(subject.may_count(), row.status, None);
                row
            })
            .collect();
        let stored = self.db.attempts().insert_response(&rows).await?;
        let Some(first) = stored.first() else {
            return Err(EngineError::Refused("a rubric has at least one dimension"));
        };
        self.evidence(
            first.id,
            EvidenceKind::ResponseText,
            Some(meta.response_text.to_owned()),
            None,
            now,
        )
        .await?;
        self.db
            .pending_scoring()
            .enqueue(first.id, payload, &now)
            .await?;
        Ok(Recorded {
            response_id: subject.response_id.clone(),
            attempts: stored,
        })
    }

    /// Stores a rubric outcome as new attempts, one per dimension.
    pub async fn record_rubric(
        &self,
        subject: &Subject,
        meta: &RubricMeta<'_>,
        outcome: &RubricOutcome,
    ) -> Result<Recorded> {
        let now = (self.clock)();
        let version = rubric_scorer_version(meta.rubric, meta.model);
        let rows: Vec<NewAttempt> = Self::dimension_names(meta.rubric)
            .into_iter()
            .map(|dimension| {
                let mut row = attempt_row(subject, dimension, Scorer::RubricLlm, &version, now);
                apply_dimension(&mut row, outcome, dimension, subject.may_count());
                row
            })
            .collect();
        let stored = self.db.attempts().insert_response(&rows).await?;
        if let Some(first) = stored.first() {
            self.evidence(
                first.id,
                EvidenceKind::ResponseText,
                Some(meta.response_text.to_owned()),
                None,
                now,
            )
            .await?;
        }
        self.rubric_evidence(&stored, subject, meta, outcome, &version, now)
            .await?;
        Ok(Recorded {
            response_id: subject.response_id.clone(),
            attempts: stored,
        })
    }

    /// Fills in the placeholder attempts of a queued response with the outcome.
    /// The caller removes the queue entry.
    pub async fn complete_rubric(
        &self,
        attempts: &[Attempt],
        subject: &Subject,
        meta: &RubricMeta<'_>,
        outcome: &RubricOutcome,
    ) -> Result<()> {
        let now = (self.clock)();
        let version = rubric_scorer_version(meta.rubric, meta.model);
        for attempt in attempts {
            let mut row = attempt_row(
                subject,
                &attempt.dimension,
                Scorer::RubricLlm,
                &version,
                now,
            );
            apply_dimension(&mut row, outcome, &attempt.dimension, subject.may_count());
            self.db
                .attempts()
                .update_score(
                    attempt.id,
                    &ScoreUpdate {
                        scorer_version: version.clone(),
                        raw_score: row.raw_score,
                        max_score: row.max_score,
                        normalized: row.normalized,
                        confidence: row.confidence,
                        status: row.status,
                    },
                )
                .await?;
            if attempt.counts_toward_estimate != row.counts_toward_estimate {
                self.db
                    .attempts()
                    .set_counts_toward_estimate(attempt.id, row.counts_toward_estimate)
                    .await?;
            }
        }
        self.rubric_evidence(attempts, subject, meta, outcome, &version, now)
            .await
    }

    async fn rubric_evidence(
        &self,
        attempts: &[Attempt],
        subject: &Subject,
        meta: &RubricMeta<'_>,
        outcome: &RubricOutcome,
        version: &str,
        now: Timestamp,
    ) -> Result<()> {
        if let Some(first) = attempts.first() {
            self.evidence(
                first.id,
                EvidenceKind::Metric,
                None,
                Some(provenance(
                    Scorer::RubricLlm,
                    version,
                    RUBRIC_ALGORITHM_VERSION,
                    &subject.engines,
                    json!({
                        "rubric": format!("{}/{}", meta.rubric.id, meta.rubric.version),
                        "contract_version": RUBRIC_SCORE_VERSION,
                        "model": meta.model,
                        "input_mode": meta.input_mode,
                        "runs": outcome.runs,
                        "runs_agreed": outcome.runs_agreed,
                        "repaired": outcome.repaired,
                        "ladder_level": meta.ladder_level,
                        "provider_qualified": meta.provider_qualified,
                        "confidence": outcome.confidence,
                        "alarms": outcome.alarms,
                        "on_task": outcome.on_task,
                        "metrics": meta.metrics,
                    }),
                )),
                now,
            )
            .await?;
        }
        for attempt in attempts {
            let Some(dimension) = outcome
                .dimensions
                .iter()
                .find(|d| d.dimension.as_str() == attempt.dimension)
            else {
                continue;
            };
            for quote in &dimension.evidence_quotes {
                self.evidence(
                    attempt.id,
                    EvidenceKind::Quote,
                    Some(quote.clone()),
                    None,
                    now,
                )
                .await?;
            }
            if !dimension.reason.trim().is_empty() {
                self.evidence(
                    attempt.id,
                    EvidenceKind::ScorerReason,
                    Some(dimension.reason.clone()),
                    None,
                    now,
                )
                .await?;
            }
            if dimension.runs_disagree {
                self.evidence(
                    attempt.id,
                    EvidenceKind::ScorerReason,
                    Some("The two runs differ by more than one band.".to_owned()),
                    Some(json!({ "run_bands": dimension.run_bands })),
                    now,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Stores a pronunciation report.
    ///
    /// A report with an utterance score becomes one `scored` attempt whose
    /// confidence is capped at [`PRON_CONFIDENCE_CAP`]. A report without one (no
    /// curve configured, nothing aligned, every word unchecked) becomes an
    /// `insufficient` attempt with the reason, never a zero. Free-speech
    /// results are stored but never count. The phoneme results that have a
    /// decision (a threshold applied) go to `pron_results`; a phone no threshold
    /// decided on is not stored as "fine".
    pub async fn record_pron(
        &self,
        subject: &Subject,
        record: &PronRecord<'_>,
    ) -> Result<Recorded> {
        let now = (self.clock)();
        let report = record.report;
        let version = format!(
            "{} {}{}+thresholds {}",
            record.engine.id,
            record.engine.version,
            record
                .engine
                .model_checksum
                .as_deref()
                .map_or(String::new(), |c| format!("+{c}")),
            record.threshold_set_version
        );
        let mut row = attempt_row(subject, "pronunciation", Scorer::PronEngine, &version, now);
        let drill = matches!(record.mode, PronMode::Drill);
        match report.utterance_score {
            Some(score) => {
                row.raw_score = Some(f64::from(score));
                row.max_score = Some(1.0);
                row.normalized = Some(f64::from(score).clamp(0.0, 1.0));
                row.confidence = Some(PRON_CONFIDENCE_CAP);
                row.status = AttemptStatus::Scored;
            }
            None => row.status = AttemptStatus::Insufficient,
        }
        row.counts_toward_estimate =
            drill && counts_toward_estimate(subject.may_count(), row.status, row.confidence);
        let stored = self.db.attempts().insert_response(&[row]).await?;
        let Some(first) = stored.first() else {
            return Err(EngineError::Refused("no attempt was stored"));
        };

        let mut engines = subject.engines.clone();
        engines.push(EngineStamp::new(EngineRole::Pron, &record.engine));
        let focus: Vec<Value> = report
            .focus
            .iter()
            .map(|f| {
                json!({
                    "symbol": f.symbol.as_str(),
                    "occurrences": f.occurrences,
                    "mean_gop": f.mean_gop,
                    "mean_score": f.mean_score,
                    "flagged": f.flagged,
                })
            })
            .collect();
        self.evidence(
            first.id,
            EvidenceKind::Metric,
            None,
            Some(provenance(
                Scorer::PronEngine,
                &version,
                PRON_ALGORITHM_VERSION,
                &engines,
                json!({
                    "mode": record.mode.as_str(),
                    "experimental": report.experimental,
                    "outcome": report.outcome,
                    "utterance_score": report.utterance_score,
                    "words_scored": report.words_scored,
                    "words_not_checked": report.words_not_checked,
                    "scores_calibrated": report.scores_calibrated,
                    "calibration_validated": report.calibration_validated,
                    "threshold_set_version": record.threshold_set_version,
                    "focus": focus,
                    "frames": report.frames,
                }),
            )),
            now,
        )
        .await?;
        self.evidence(
            first.id,
            EvidenceKind::ResponseText,
            Some(record.reference_text.to_owned()),
            Some(json!({ "kind": "reference_text" })),
            now,
        )
        .await?;
        if report.utterance_score.is_none() {
            let reason = match &report.outcome {
                pron_engine::Outcome::Aligned if !report.scores_calibrated => {
                    "No score curve is configured, so no pronunciation score exists."
                }
                pron_engine::Outcome::Aligned => "No word of the reference could be scored.",
                pron_engine::Outcome::NotScored(_) => "The utterance could not be scored.",
            };
            self.evidence(
                first.id,
                EvidenceKind::ScorerReason,
                Some(reason.to_owned()),
                Some(json!({ "outcome": report.outcome })),
                now,
            )
            .await?;
        }
        let results = pron_rows(report, first.id, record);
        if !results.is_empty() {
            self.db.pron_results().add_many(&results).await?;
        }
        Ok(Recorded {
            response_id: subject.response_id.clone(),
            attempts: stored,
        })
    }
}

/// Sets the score, confidence, status and counting flag of one dimension row
/// from the outcome.
fn apply_dimension(
    row: &mut NewAttempt,
    outcome: &RubricOutcome,
    dimension: &str,
    may_count: bool,
) {
    let scored = outcome
        .dimensions
        .iter()
        .find(|d| d.dimension.as_str() == dimension)
        .filter(|d| d.status == DimensionStatus::Scored)
        .and_then(|d| d.band);
    match scored {
        Some(band) => {
            row.raw_score = Some(band);
            row.max_score = Some(4.0);
            row.normalized = Some(band / 4.0);
            row.confidence = Some(outcome.confidence);
            row.status = AttemptStatus::Scored;
        }
        None => {
            row.raw_score = None;
            row.max_score = None;
            row.normalized = None;
            row.confidence = None;
            row.status = AttemptStatus::NeedsReview;
        }
    }
    row.counts_toward_estimate = counts_toward_estimate(may_count, row.status, row.confidence);
}

fn pron_rows(
    report: &UtteranceReport,
    attempt_id: i64,
    record: &PronRecord<'_>,
) -> Vec<NewPronResult> {
    let engine_version = format!("{} {}", record.engine.id, record.engine.version);
    let mut rows = Vec::new();
    for (word_index, word) in report.words.iter().enumerate() {
        let WordResult::Scored { text, phonemes, .. } = word else {
            continue;
        };
        for phoneme in phonemes {
            let PhonemeResult {
                symbol,
                gop,
                flagged,
                heard,
                start_frame,
                end_frame,
                ..
            } = phoneme;
            let Some(flagged) = flagged else {
                continue;
            };
            rows.push(NewPronResult {
                attempt_id: Some(attempt_id),
                turn_id: None,
                mode: record.mode,
                word: text.clone(),
                word_index: i64::try_from(word_index).unwrap_or(i64::MAX),
                phone_expected: symbol.as_str().to_owned(),
                phone_heard: heard.as_ref().map(|h| {
                    h.arpabet
                        .map_or_else(|| h.label.clone(), |a| a.as_str().to_owned())
                }),
                gop: f64::from(*gop),
                flagged: *flagged,
                start_ms: frames_to_ms(*start_frame, record.frame_ms),
                end_ms: frames_to_ms(*end_frame, record.frame_ms),
                engine_version: engine_version.clone(),
            });
        }
    }
    rows
}

fn frames_to_ms(frames: usize, frame_ms: u32) -> i64 {
    i64::try_from(frames)
        .unwrap_or(i64::MAX)
        .saturating_mul(i64::from(frame_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_authored_scored_or_waiting_work_above_the_floor_counts() {
        use AttemptStatus::*;
        assert!(counts_toward_estimate(true, Scored, Some(1.0)));
        assert!(counts_toward_estimate(true, Scored, Some(0.3)));
        assert!(!counts_toward_estimate(true, Scored, Some(0.29)));
        assert!(counts_toward_estimate(true, PendingLlm, None));
        for status in [Insufficient, NeedsReview, Rejected] {
            assert!(!counts_toward_estimate(true, status, None), "{status:?}");
        }
        assert!(!counts_toward_estimate(false, Scored, Some(1.0)));
    }

    #[test]
    fn a_subject_counts_only_when_authored_and_not_marked_as_practice() {
        let make = |origin| {
            Subject::new(
                1,
                None,
                None,
                "a1",
                "mcq",
                Level::A1,
                "grammar",
                origin,
                "r1",
            )
        };
        assert!(make(Origin::Authored).may_count());
        assert!(!make(Origin::Generated).may_count());
        assert!(!make(Origin::FreeMode).may_count());
        assert!(!make(Origin::Authored).never_counts().may_count());
    }

    #[test]
    fn the_scorer_version_names_the_rubric_the_contract_and_the_model() {
        let rubric = WorkshopRubric {
            id: "rubric-a1-spoken-production".into(),
            version: 2,
            dimensions: Vec::new(),
        };
        assert_eq!(
            rubric_scorer_version(&rubric, Some("m-1")),
            "rubric-a1-spoken-production/2+rubric_score/1+m-1"
        );
        assert!(rubric_scorer_version(&rubric, None).ends_with("+unscored"));
    }
}
