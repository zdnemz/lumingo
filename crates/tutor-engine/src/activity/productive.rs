//! The productive activities of a unit: `guided_speaking`, `guided_writing` and
//! `mediation`.
//!
//! A response is a transcript or a text. It goes to the rubric scorer with the
//! unit's rubric and the activity's task; when no provider answers it is stored
//! as `pending_llm` and queued, and [`crate::RubricScorer::score_pending_backlog`]
//! scores it later. A model never sets a level: what comes back is bands per
//! dimension, checked by X1 to X7.

use assessment_engine::{TimingMetrics, timing_metrics, word_count};
use curriculum::{Activity, GuidedProduction, Mediation};
use storage::AttemptStatus;
use storage::Scorer;
use tokio_util::sync::CancellationToken;

use super::error::ActivityError;
use super::player::{ActivityResult, ResultOutcome, UnitPlayer, UnscoredReason};
use super::response::{Response, SpokenResponse, check_text};
use super::score::evidence_skill;
use crate::error::{EngineError, Result};
use crate::evidence::{EngineRole, EngineStamp};
use crate::rubric::{
    InputMode, RubricOutcome, Runs, ScoreRequest, ScoreResult, WorkshopRubric, WorkshopTask,
};

/// The task the rubric scorer is told about, from a guided production.
fn production_task(a: &GuidedProduction) -> WorkshopTask {
    WorkshopTask {
        prompt: a.prompt.en.clone(),
        content_points: a.content_points.clone(),
        min_words: Some(a.min_words),
    }
}

/// A mediation task names its source text, because the scorer has to judge the
/// relay against it.
fn mediation_task(a: &Mediation) -> WorkshopTask {
    WorkshopTask {
        prompt: format!("{}\n\nSource text:\n{}", a.task.en, a.source_text),
        content_points: Vec::new(),
        min_words: None,
    }
}

/// What the learner handed in for a productive activity, read.
struct Given {
    text: String,
    voice: bool,
    spoken: Option<SpokenResponse>,
}

fn read_response(activity: &Activity, response: Response) -> Result<Given> {
    let wrong = |expected| Err(ActivityError::WrongKind { expected }.into());
    let given = match (activity, response) {
        (Activity::GuidedSpeaking(_), Response::Spoken(s)) => Given {
            text: s.transcript.clone(),
            voice: true,
            spoken: Some(s),
        },
        (Activity::GuidedSpeaking(_), _) => return wrong("a spoken transcript"),
        (Activity::GuidedWriting(_), Response::Text(text)) => Given {
            text,
            voice: false,
            spoken: None,
        },
        (Activity::GuidedWriting(_), _) => return wrong("text"),
        (Activity::Mediation(_), Response::Spoken(s)) => Given {
            text: s.transcript.clone(),
            voice: true,
            spoken: Some(s),
        },
        (Activity::Mediation(_), Response::Text(text)) => Given {
            text,
            voice: false,
            spoken: None,
        },
        (Activity::Mediation(_), _) => return wrong("text or a spoken transcript"),
        _ => return wrong("a productive activity"),
    };
    if given.text.trim().is_empty() {
        return Err(ActivityError::Empty.into());
    }
    check_text(&given.text)?;
    Ok(given)
}

/// The mean of the dimensions and the confidence, when the outcome is complete.
pub(super) fn outcome_scores(outcome: &RubricOutcome) -> (Option<f64>, Option<f64>) {
    match outcome.mean_normalized() {
        Some(mean) => (Some(mean), Some(outcome.confidence)),
        None => (None, None),
    }
}

impl UnitPlayer {
    pub(super) async fn submit_production(
        &mut self,
        activity: &Activity,
        response: Response,
        cancel: &CancellationToken,
    ) -> Result<ActivityResult> {
        let given = read_response(activity, response)?;
        let (rubric_id, task) = match activity {
            Activity::GuidedSpeaking(a) | Activity::GuidedWriting(a) => {
                (a.rubric_id.clone(), production_task(a))
            }
            Activity::Mediation(a) => (a.rubric_id.clone(), mediation_task(a)),
            _ => {
                return Err(ActivityError::WrongKind {
                    expected: "a productive activity",
                }
                .into());
            }
        };
        let timing: Option<TimingMetrics> = match &given.spoken {
            Some(spoken) if !spoken.spans.is_empty() => Some(
                timing_metrics(&spoken.spans, word_count(&given.text))
                    .map_err(|_| EngineError::Refused("the voiced spans are malformed"))?,
            ),
            _ => None,
        };
        // The text is stored with the session first, so a failure later cannot lose it.
        self.store_learner_turn(&given.text, given.voice, timing.as_ref())
            .await?;

        let skill = evidence_skill(activity, given.voice);
        let mut subject = self.subject(activity, skill);
        if let Some(stt) = given.spoken.as_ref().and_then(|s| s.stt.as_ref()) {
            subject = subject.with_engine(EngineStamp::new(EngineRole::Stt, stt));
        }
        let response_id = subject.response_id.clone();

        let Some(catalog_rubric) = self.env.rubrics.get(&rubric_id) else {
            let reason = format!("The rubric {rubric_id} is not in the catalog.");
            self.recorder
                .record_unscored(
                    &subject,
                    Scorer::RubricLlm,
                    "rubric_missing/1",
                    "overall",
                    AttemptStatus::Insufficient,
                    &reason,
                    Some(&given.text),
                    serde_json::json!({ "rubric_id": rubric_id }),
                )
                .await?;
            return self
                .finish_activity(
                    activity,
                    skill,
                    None,
                    None,
                    ResultOutcome::Unscored(UnscoredReason::RubricMissing { rubric_id }),
                    vec![response_id],
                )
                .await;
        };
        let rubric: WorkshopRubric = catalog_rubric.rubric.clone();
        let request = ScoreRequest {
            subject,
            rubric,
            task,
            response: given.text.clone(),
            input_mode: if given.voice {
                InputMode::Voice
            } else {
                InputMode::Text
            },
            runs: if self.in_checkpoint(activity.id()) {
                Runs::Two
            } else {
                Runs::One
            },
            first_language: self.config.first_language.clone(),
            timing,
        };
        match self.scorer.score(request, cancel).await? {
            ScoreResult::Scored { outcome, .. } => {
                let (score, confidence) = outcome_scores(&outcome);
                self.finish_activity(
                    activity,
                    skill,
                    score,
                    confidence,
                    ResultOutcome::Rubric(outcome),
                    vec![response_id],
                )
                .await
            }
            ScoreResult::Queued { .. } => {
                self.finish_activity(
                    activity,
                    skill,
                    None,
                    None,
                    ResultOutcome::Queued,
                    vec![response_id],
                )
                .await
            }
        }
    }
}
