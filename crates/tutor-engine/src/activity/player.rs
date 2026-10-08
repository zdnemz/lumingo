//! Playing a unit: present each activity, take the learner's response, score it,
//! store the rows, and decide the unit checkpoint from what was stored.
//!
//! [`UnitPlayer`] is the one object the command line, the server and the tests
//! drive. It owns no engine: the language model, the pronunciation engine and
//! the rubrics come in through [`UnitEnv`]. Deterministic activities are scored
//! here. Productive ones go to the rubric scorer, or wait in the pending queue
//! when no provider answers. Drills go to the [`DrillScorer`]. Every response is
//! written by the evidence recorder, and the checkpoint is read back from those
//! rows, so it can be recomputed after the backlog has been scored.

use std::collections::HashMap;
use std::sync::Arc;

use assessment_engine::{CheckpointItem, CheckpointOutcome, Origin, evaluate_checkpoint};
use curriculum::validate::{GrammarCheck, WordLevels};
use curriculum::{Activity, ActivityType, Unit};
use llm_client::LlmClient;
use pron_engine::UtteranceReport;
use serde::Serialize;
use serde_json::json;
use storage::{
    AttemptStatus, Database, NewSession, ObjectiveMastery, SessionStatus, Timestamp, UnitProgress,
    UnitStatus,
};
use tokio_util::sync::CancellationToken;

use super::drill::DrillScorer;
use super::error::ActivityError;
use super::feedback::Feedback;
use super::present::{AudioLine, Presentation, audio_of, present};
use super::replay::PlayCounter;
use super::response::Response;
use super::score::{evidence_skill, score_deterministic};
use crate::chat::ChatDeps;
use crate::error::{EngineError, Result};
use crate::evidence::{EngineRole, EngineStamp, EvidenceRecorder, Recorded, Subject};
use crate::review::update_mastery;
use crate::rubric::{RubricCatalog, RubricOutcome, RubricScorer, ScorerEnv};
use crate::support::{Clock, assessment_level, storage_level};

/// What a unit run works with. Cheap to clone.
#[derive(Clone)]
pub struct UnitEnv {
    /// The language model. [`crate::NoProvider`] when none is configured.
    pub client: Arc<dyn LlmClient>,
    pub db: Database,
    pub clock: Clock,
    /// The model's name, stored with every rubric score. A placeholder such as
    /// `none` when there is no provider.
    pub model: String,
    pub provider_profile_id: Option<i64>,
    /// Whether the provider passed scorer qualification.
    pub provider_qualified: bool,
    /// The rule-based checker for X5. Without one X5 is not evaluated.
    pub grammar: Option<Arc<dyn GrammarCheck + Send + Sync>>,
    /// Word levels for X4 and the vocabulary profile. Optional.
    pub word_levels: Option<Arc<WordLevels>>,
    /// The authored rubrics, by id.
    pub rubrics: Arc<RubricCatalog>,
    /// The pronunciation engine. Without one a drill is stored as unscored.
    pub drill: Option<Arc<dyn DrillScorer>>,
    /// The speech synthesiser the learner hears, recorded as evidence on the
    /// activities that play audio.
    pub tts: Option<speech::EngineInfo>,
}

impl UnitEnv {
    pub(super) fn scorer_env(&self) -> ScorerEnv {
        ScorerEnv {
            client: self.client.clone(),
            db: self.db.clone(),
            clock: self.clock.clone(),
            model: self.model.clone(),
            provider_profile_id: self.provider_profile_id,
            grammar: self.grammar.clone(),
            word_levels: self.word_levels.clone(),
            provider_qualified: self.provider_qualified,
        }
    }

    pub(super) fn chat_deps(&self) -> ChatDeps {
        ChatDeps {
            client: self.client.clone(),
            db: self.db.clone(),
            clock: self.clock.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnitConfig {
    pub profile_id: i64,
    /// The first language written in English ("Indonesian").
    pub first_language: String,
    pub app_version: String,
}

/// Why a response has no score and never will without a change on the learner's
/// side or the project's. None of these is a low score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum UnscoredReason {
    /// The activity's scoring is `none`: practice with nothing to score.
    NoScorer,
    /// The unit names a rubric the catalog does not have.
    RubricMissing { rubric_id: String },
    /// There is no pronunciation engine in this run, or it failed.
    EngineUnavailable { why: String },
    /// A scored roleplay needs the interaction rubric of the unit's level.
    InteractionRubricMissing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResultOutcome {
    /// An objective item: the feedback data.
    Deterministic(Box<Feedback>),
    /// A productive response the model scored.
    Rubric(Box<RubricOutcome>),
    /// A productive response waiting for a provider.
    Queued,
    /// A pronunciation drill: one report per recorded clip.
    Pron(Vec<UtteranceReport>),
    /// Stored, with nothing to score.
    Unscored(UnscoredReason),
}

/// What happened to one activity.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityResult {
    pub activity_id: String,
    pub activity_type: ActivityType,
    /// The skill the attempt rows are filed under.
    pub skill: String,
    /// 0 to 1 when there is a score: the activity's score, or the mean over a
    /// response's dimensions or a drill's clips. `None` while the response waits
    /// for a provider, needs review or has no scorer.
    pub score: Option<f64>,
    pub confidence: Option<f64>,
    pub outcome: ResultOutcome,
    /// The ids of the responses stored, one per submission or, for a drill, per clip.
    pub response_ids: Vec<String>,
}

/// What was played of an audio item.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioPlay {
    pub lines: Vec<AudioLine>,
    /// Plays so far, this one included.
    pub plays: u8,
    /// Replays left, or `None` when the item has no limit.
    pub replays_left: Option<u8>,
}

/// One checkpoint activity as it stands in the stored rows.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointRow {
    pub activity_id: String,
    /// The activity has a stored response.
    pub answered: bool,
    /// The response's score when every dimension is scored.
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointReport {
    pub rows: Vec<CheckpointRow>,
    pub outcome: CheckpointOutcome,
    pub pass_mark: f64,
    /// Activities of the checkpoint with no response yet.
    pub unanswered: Vec<String>,
}

/// What `finish` leaves behind.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitSummary {
    pub checkpoint: CheckpointReport,
    pub status: UnitStatus,
    pub answered: usize,
}

pub struct UnitPlayer {
    pub(super) env: UnitEnv,
    pub(super) config: UnitConfig,
    pub(super) unit: Unit,
    pub(super) session_id: i64,
    pub(super) recorder: EvidenceRecorder,
    pub(super) scorer: Arc<RubricScorer>,
    plays: HashMap<String, PlayCounter>,
    results: HashMap<String, ActivityResult>,
    order: Vec<String>,
}

/// A unit must be in the curriculum index before a session can point at it:
/// the application indexes every unit at start-up, and a tool that plays one
/// loose file indexes it here. A unit that is already indexed is left alone.
pub async fn ensure_indexed(
    db: &Database,
    unit: &Unit,
    file_sha256: &str,
    now: Timestamp,
) -> Result<()> {
    if db.curriculum().unit(&unit.id).await?.is_some() {
        return Ok(());
    }
    db.curriculum()
        .install(&storage::NewCurriculumVersion {
            content_version: format!("{}-{}", unit.id, &file_sha256[..file_sha256.len().min(12)]),
            schema_version: unit.schema_version.clone(),
            manifest_sha256: file_sha256.to_owned(),
            installed_at: now,
            units: vec![storage::NewUnit {
                id: unit.id.clone(),
                level: storage_level(assessment_level(unit.level)),
                sequence: i64::from(unit.sequence),
                title_en: unit.title.en.clone(),
                file_sha256: file_sha256.to_owned(),
                objectives: unit
                    .objectives
                    .iter()
                    .map(|o| storage::NewObjective {
                        id: format!("{}/{}", unit.id, o.id),
                        skill: serde_json::to_value(o.skill)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default(),
                        can_do_en: o.can_do.en.clone(),
                    })
                    .collect(),
            }],
        })
        .await?;
    Ok(())
}

impl UnitPlayer {
    /// Starts a run of `unit`: stores a lesson session and marks the unit in
    /// progress unless it is already passed. The unit must be in the curriculum
    /// index (see [`ensure_indexed`]).
    pub async fn start(env: UnitEnv, config: UnitConfig, unit: Unit) -> Result<Self> {
        Self::start_as(env, config, unit, storage::SessionKind::Lesson).await
    }

    /// Like [`UnitPlayer::start`], storing the run as a session of `kind`: a
    /// lesson, a checkpoint run or a drill run all play activities of a unit.
    pub async fn start_as(
        env: UnitEnv,
        config: UnitConfig,
        unit: Unit,
        kind: storage::SessionKind,
    ) -> Result<Self> {
        let now = (env.clock)();
        let session = env
            .db
            .sessions()
            .create(&NewSession {
                profile_id: config.profile_id,
                kind,
                unit_id: Some(unit.id.clone()),
                activity_id: None,
                mode: None,
                provider_profile_id: env.provider_profile_id,
                app_version: config.app_version.clone(),
                started_at: now,
            })
            .await?;
        let existing = env
            .db
            .unit_progress()
            .get(config.profile_id, &unit.id)
            .await?;
        if existing
            .as_ref()
            .is_none_or(|p| p.status != UnitStatus::Passed)
        {
            env.db
                .unit_progress()
                .set(&UnitProgress {
                    profile_id: config.profile_id,
                    unit_id: unit.id.clone(),
                    status: UnitStatus::InProgress,
                    best_checkpoint: existing.and_then(|p| p.best_checkpoint),
                    updated_at: now,
                })
                .await?;
        }
        let recorder = EvidenceRecorder::new(env.db.clone(), env.clock.clone());
        let scorer = Arc::new(RubricScorer::new(env.scorer_env()));
        let plays = unit
            .activities
            .iter()
            .map(|a| (a.id().to_owned(), PlayCounter::for_activity(a)))
            .collect();
        let order = unit.activities.iter().map(|a| a.id().to_owned()).collect();
        Ok(Self {
            env,
            config,
            unit,
            session_id: session.id,
            recorder,
            scorer,
            plays,
            results: HashMap::new(),
            order,
        })
    }

    pub fn unit(&self) -> &Unit {
        &self.unit
    }

    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    /// The rubric scorer of this run, for scoring the backlog.
    pub fn scorer(&self) -> &Arc<RubricScorer> {
        &self.scorer
    }

    pub fn recorder(&self) -> &EvidenceRecorder {
        &self.recorder
    }

    /// The ids of the activities in the order the unit lists them.
    pub fn activity_ids(&self) -> &[String] {
        &self.order
    }

    /// The result stored for an activity, once it is answered.
    pub fn result(&self, id: &str) -> Option<&ActivityResult> {
        self.results.get(id)
    }

    pub fn answered(&self) -> usize {
        self.results.len()
    }

    pub(super) fn activity(&self, id: &str) -> Result<&Activity> {
        self.unit
            .activities
            .iter()
            .find(|a| a.id() == id)
            .ok_or_else(|| ActivityError::UnknownActivity(id.to_owned()).into())
    }

    /// What the learner is shown for an activity.
    pub fn present(&self, id: &str) -> Result<Presentation> {
        Ok(present(&self.unit, self.activity(id)?)?)
    }

    /// Plays the audio of an item. A listening set refuses a play past its first
    /// one and its `replays_allowed`; other audio items are not limited.
    pub fn play_audio(&mut self, id: &str) -> Result<AudioPlay> {
        let lines = audio_of(&self.unit, self.activity(id)?)?;
        let counter = self
            .plays
            .get_mut(id)
            .ok_or_else(|| ActivityError::UnknownActivity(id.to_owned()))?;
        let plays = counter.play()?;
        Ok(AudioPlay {
            lines,
            plays,
            replays_left: counter.replays_left(),
        })
    }

    pub(super) fn check_open(&self, id: &str) -> Result<()> {
        if self.results.contains_key(id) {
            return Err(ActivityError::AlreadyAnswered(id.to_owned()).into());
        }
        Ok(())
    }

    /// A subject for one response to `activity`, authored and counting.
    pub(super) fn subject(&self, activity: &Activity, skill: &str) -> Subject {
        let common = activity.common();
        let mut subject = Subject::new(
            self.config.profile_id,
            Some(self.session_id),
            Some(self.unit.id.clone()),
            common.id,
            common.activity_type.as_str(),
            assessment_level(self.unit.level),
            skill,
            Origin::Authored,
            self.recorder.new_response_id(common.id),
        );
        let has_audio = matches!(activity, Activity::Dictation(_) | Activity::ListeningSet(_))
            || matches!(activity, Activity::Mcq(a) if a.audio_text.is_some())
            || matches!(activity, Activity::MinimalPairs(a) if a.mode == curriculum::MinimalPairsMode::ListenChoose);
        if has_audio && let Some(tts) = &self.env.tts {
            subject = subject.with_engine(EngineStamp::new(EngineRole::Tts, tts));
        }
        subject
    }

    /// Whether the unit's checkpoint includes this activity: its productive
    /// responses are scored in two runs (assessment spec 5.2).
    pub(super) fn in_checkpoint(&self, id: &str) -> bool {
        self.unit.checkpoint.activity_ids.iter().any(|c| c == id)
    }

    /// Records the result of an activity and updates the mastery of its objectives.
    pub(super) async fn finish_activity(
        &mut self,
        activity: &Activity,
        skill: &str,
        score: Option<f64>,
        confidence: Option<f64>,
        outcome: ResultOutcome,
        response_ids: Vec<String>,
    ) -> Result<ActivityResult> {
        let common = activity.common();
        if let Some(score) = score {
            self.update_mastery_of(common.objective_ids, score).await?;
        }
        let result = ActivityResult {
            activity_id: common.id.to_owned(),
            activity_type: common.activity_type,
            skill: skill.to_owned(),
            score,
            confidence,
            outcome,
            response_ids,
        };
        self.results
            .insert(result.activity_id.clone(), result.clone());
        Ok(result)
    }

    /// Objective mastery is a moving average of the results of its activities
    /// (`<unit id>/<objective id>`).
    async fn update_mastery_of(&self, objective_ids: &[String], score: f64) -> Result<()> {
        let now = (self.env.clock)();
        for objective in objective_ids {
            let key = format!("{}/{objective}", self.unit.id);
            let previous = self
                .env
                .db
                .mastery()
                .get(self.config.profile_id, &key)
                .await?;
            self.env
                .db
                .mastery()
                .set(&ObjectiveMastery {
                    profile_id: self.config.profile_id,
                    objective_id: key,
                    mastery: update_mastery(previous.as_ref().map(|m| m.mastery), score),
                    attempts: previous.map_or(0, |m| m.attempts) + 1,
                    last_attempt_at: Some(now),
                })
                .await?;
        }
        Ok(())
    }

    /// Takes the response to an activity, scores it and stores it.
    ///
    /// Objective activities are scored here. A productive activity goes to the
    /// rubric scorer, in two runs when it is part of the checkpoint, or waits in
    /// the pending queue when the provider cannot be reached. A drill goes to the
    /// pronunciation engine. An activity is answered once per run: a second
    /// response is refused, so repeating an item cannot add evidence.
    pub async fn submit(
        &mut self,
        id: &str,
        response: Response,
        cancel: &CancellationToken,
    ) -> Result<ActivityResult> {
        self.check_open(id)?;
        let activity = self.activity(id)?.clone();
        match &activity {
            Activity::Mcq(_)
            | Activity::GapFill(_)
            | Activity::Reorder(_)
            | Activity::Match(_)
            | Activity::Dictation(_)
            | Activity::ReadingSet(_)
            | Activity::ListeningSet(_)
            | Activity::ErrorCorrection(_) => self.submit_deterministic(&activity, &response).await,
            Activity::MinimalPairs(a) if a.mode == curriculum::MinimalPairsMode::ListenChoose => {
                self.submit_deterministic(&activity, &response).await
            }
            Activity::GuidedSpeaking(_) | Activity::GuidedWriting(_) | Activity::Mediation(_) => {
                self.submit_production(&activity, response, cancel).await
            }
            Activity::ReadAloud(_) | Activity::MinimalPairs(_) | Activity::Shadowing(_) => {
                self.submit_drill(&activity, response, cancel).await
            }
            Activity::Roleplay(_) => Err(ActivityError::WrongKind {
                expected: "a roleplay run: use start_roleplay and finish_roleplay",
            }
            .into()),
        }
    }

    async fn submit_deterministic(
        &mut self,
        activity: &Activity,
        response: &Response,
    ) -> Result<ActivityResult> {
        let scored = score_deterministic(activity, response)?;
        let skill = evidence_skill(activity, false);
        let subject = self.subject(activity, skill);
        let recorded = self
            .recorder
            .record_deterministic(&subject, &scored.record)
            .await?;
        let normalized = scored.record.normalized;
        self.finish_activity(
            activity,
            skill,
            Some(normalized),
            Some(1.0),
            ResultOutcome::Deterministic(Box::new(scored.feedback)),
            vec![recorded.response_id],
        )
        .await
    }

    /// The checkpoint, read from the stored rows of this run.
    pub async fn checkpoint(&self) -> Result<CheckpointReport> {
        checkpoint_report(
            &self.env.db,
            self.session_id,
            &self.unit,
            &self.unit.checkpoint.activity_ids,
        )
        .await
    }

    /// Ends the run: decides the checkpoint from the stored rows, updates the
    /// unit's progress (see [`settle_unit`]) and closes the session. An
    /// incomplete checkpoint is an error: finish only when every activity of it
    /// has been answered.
    pub async fn finish(&mut self) -> Result<UnitSummary> {
        let now = (self.env.clock)();
        let summary = settle_unit(
            &self.env.db,
            now,
            self.config.profile_id,
            self.session_id,
            &self.unit,
        )
        .await?;
        let record = json!({
            "answered": self.results.len(),
            "checkpoint_mean": summary.checkpoint.outcome.mean,
            "checkpoint_passed": summary.checkpoint.outcome.passed,
            "provisional": summary.checkpoint.outcome.provisional,
        });
        self.env
            .db
            .sessions()
            .finish(
                self.session_id,
                SessionStatus::Completed,
                &now,
                Some(&record),
            )
            .await?;
        Ok(summary)
    }

    /// Ends a practice run (a drill run) as completed. The checkpoint is not
    /// consulted and the unit's progress does not change: a run that only
    /// practised some activities has nothing to settle.
    pub async fn finish_practice(&mut self) -> Result<()> {
        self.env
            .db
            .sessions()
            .finish(
                self.session_id,
                SessionStatus::Completed,
                &(self.env.clock)(),
                Some(&json!({ "answered": self.results.len() })),
            )
            .await?;
        Ok(())
    }

    /// Ends the run early. Nothing about the unit's progress changes.
    pub async fn abort(&mut self) -> Result<()> {
        self.env
            .db
            .sessions()
            .finish(
                self.session_id,
                SessionStatus::Aborted,
                &(self.env.clock)(),
                Some(&json!({ "answered": self.results.len() })),
            )
            .await?;
        Ok(())
    }

    pub(super) async fn store_learner_turn(
        &self,
        text: &str,
        voice: bool,
        timing: Option<&assessment_engine::TimingMetrics>,
    ) -> Result<()> {
        let words = assessment_engine::word_count(text);
        self.env
            .db
            .turns()
            .append(&storage::NewTurn {
                session_id: self.session_id,
                role: storage::TurnRole::Learner,
                input_mode: if voice {
                    storage::InputMode::Voice
                } else {
                    storage::InputMode::Text
                },
                text: text.to_owned(),
                stt_text: None,
                edited_by_learner: false,
                speech_ms: timing.and_then(|t| i64::try_from(t.voiced_ms).ok()),
                pause_ms: timing.and_then(|t| i64::try_from(t.pause_ms).ok()),
                word_count: i64::try_from(words).ok(),
                created_at: (self.env.clock)(),
            })
            .await?;
        Ok(())
    }
}

/// Decides the checkpoint of a run from its stored rows and updates the unit's
/// progress.
///
/// A unit is `passed` when the checkpoint is passed and no item of it is still
/// waiting for a score. A pass that rests on the objective items alone is
/// provisional: the unit stays `in_progress` with its score kept. Once the pending
/// responses are scored, calling this again gives the final decision, because it
/// reads the stored rows and not the run's memory. A unit that was passed stays
/// passed. An incomplete checkpoint is an error.
pub async fn settle_unit(
    db: &Database,
    now: Timestamp,
    profile_id: i64,
    session_id: i64,
    unit: &Unit,
) -> Result<UnitSummary> {
    let report = checkpoint_report(db, session_id, unit, &unit.checkpoint.activity_ids).await?;
    if !report.unanswered.is_empty() {
        return Err(EngineError::Refused(
            "the checkpoint still has activities to answer",
        ));
    }
    let existing = db.unit_progress().get(profile_id, &unit.id).await?;
    let passed = report.outcome.passed && !report.outcome.provisional;
    let status = if passed
        || existing
            .as_ref()
            .is_some_and(|p| p.status == UnitStatus::Passed)
    {
        UnitStatus::Passed
    } else {
        UnitStatus::InProgress
    };
    let previous = existing.and_then(|p| p.best_checkpoint);
    let best = if report.outcome.mean > 0.0 {
        Some(previous.map_or(report.outcome.mean, |old| old.max(report.outcome.mean)))
    } else {
        previous
    };
    db.unit_progress()
        .set(&UnitProgress {
            profile_id,
            unit_id: unit.id.clone(),
            status,
            best_checkpoint: best,
            updated_at: now,
        })
        .await?;
    let answered = report.rows.iter().filter(|r| r.answered).count();
    Ok(UnitSummary {
        checkpoint: report,
        status,
        answered,
    })
}

/// The checkpoint of `unit`, from the attempts stored for `session_id`.
///
/// For each checkpoint activity the latest stored response counts. Its score is
/// the mean of its dimensions when all of them are scored, and `None` while any
/// is waiting for a provider, in review or without a score. The pass decision is
/// the one of the assessment spec (section 10): the mean of the scored items
/// against the pass mark, provisional while an item is unscored.
pub async fn checkpoint_report(
    db: &Database,
    session_id: i64,
    unit: &Unit,
    checkpoint_ids: &[String],
) -> Result<CheckpointReport> {
    let attempts = db.attempts().for_session(session_id).await?;
    let groups = storage::group_by_response(attempts);
    let mut rows = Vec::with_capacity(checkpoint_ids.len());
    let mut unanswered = Vec::new();
    for id in checkpoint_ids {
        // A run answers an activity once, so every response of the session that
        // belongs to the activity is part of that one submission (a drill stores
        // one response per clip).
        let mine: Vec<&storage::ResponseGroup> = groups
            .iter()
            .filter(|g| g.attempts.first().is_some_and(|a| &a.activity_id == id))
            .collect();
        if mine.is_empty() {
            unanswered.push(id.clone());
            rows.push(CheckpointRow {
                activity_id: id.clone(),
                answered: false,
                score: None,
            });
            continue;
        }
        let group_scores: Vec<Option<f64>> = mine
            .iter()
            .map(|group| {
                let all_scored = group
                    .attempts
                    .iter()
                    .all(|a| a.status == AttemptStatus::Scored && a.normalized.is_some());
                all_scored.then(|| {
                    let values: Vec<f64> =
                        group.attempts.iter().filter_map(|a| a.normalized).collect();
                    values.iter().sum::<f64>() / values.len() as f64
                })
            })
            .collect();
        let score = group_scores
            .iter()
            .copied()
            .collect::<Option<Vec<f64>>>()
            .map(|all| all.iter().sum::<f64>() / all.len() as f64);
        rows.push(CheckpointRow {
            activity_id: id.clone(),
            answered: true,
            score,
        });
    }
    let items: Vec<CheckpointItem> = rows
        .iter()
        .filter(|r| r.answered)
        .map(|r| CheckpointItem { score: r.score })
        .collect();
    Ok(CheckpointReport {
        outcome: evaluate_checkpoint(&items, unit.checkpoint.pass_score),
        pass_mark: unit.checkpoint.pass_score,
        rows,
        unanswered,
    })
}

impl ActivityResult {
    /// The rows of the stored response, for callers that want to show them.
    pub async fn recorded(&self, db: &Database) -> Result<Vec<Recorded>> {
        let mut out = Vec::new();
        for response_id in &self.response_ids {
            out.push(Recorded {
                response_id: response_id.clone(),
                attempts: db.attempts().by_response(response_id).await?,
            });
        }
        Ok(out)
    }
}
