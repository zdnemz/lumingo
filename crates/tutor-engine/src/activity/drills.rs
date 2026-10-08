//! The pronunciation drills of a unit: `read_aloud`, `minimal_pairs` in say mode
//! and `shadowing`.
//!
//! A drill knows its reference text, so each recording is scored against it by
//! the [`DrillScorer`] seam, on a blocking thread. One attempt is stored per
//! clip: a pair, a line or the whole text. When the activity's scoring is `none`
//! nothing is scored and one unscored attempt records that the learner did it.
//! When no engine is available, or it fails, each clip is stored as unscored with
//! the reason: a missing engine is never a score of zero.

use curriculum::{Activity, MinimalPairsMode, Scoring};
use pron_engine::UtteranceReport;
use speech::CancelFlag;
use storage::{AttemptStatus, PronMode, Scorer};
use tokio_util::sync::CancellationToken;

use super::drill::{DrillError, DrillReport, DrillRequest, DrillScorer};
use super::error::ActivityError;
use super::player::{ActivityResult, ResultOutcome, UnitPlayer, UnscoredReason};
use super::present::audio_of;
use super::response::{Clip, Response, check_clips};
use crate::error::{EngineError, Result};
use crate::evidence::{PRON_CONFIDENCE_CAP, PronRecord};

/// One clip's worth of a drill: what is to be said, and the sounds it targets.
struct Job {
    reference_text: String,
    focus: Vec<String>,
}

fn jobs_of(unit: &curriculum::Unit, activity: &Activity) -> Result<(Scoring, Vec<Job>)> {
    Ok(match activity {
        Activity::ReadAloud(a) => (
            a.scoring,
            vec![Job {
                reference_text: a.text.clone(),
                focus: a.focus_phonemes.clone(),
            }],
        ),
        Activity::MinimalPairs(a) if a.mode == MinimalPairsMode::SayBoth => (
            a.scoring,
            a.pairs
                .iter()
                .map(|p| Job {
                    reference_text: format!("{} {}", p.a, p.b),
                    focus: Vec::new(),
                })
                .collect(),
        ),
        Activity::Shadowing(a) => (
            a.scoring,
            audio_of(unit, activity)?
                .into_iter()
                .map(|line| Job {
                    reference_text: line.text,
                    focus: Vec::new(),
                })
                .collect(),
        ),
        _ => {
            return Err(ActivityError::WrongKind {
                expected: "a pronunciation drill",
            }
            .into());
        }
    })
}

/// Runs the scorer on a blocking thread and stops waiting when the call is
/// cancelled, after telling the scorer to stop.
async fn score_blocking(
    scorer: std::sync::Arc<dyn DrillScorer>,
    request: DrillRequest,
    cancel: &CancellationToken,
) -> Result<std::result::Result<DrillReport, DrillError>> {
    let flag = CancelFlag::new();
    let worker_flag = flag.clone();
    let mut worker = tokio::task::spawn_blocking(move || scorer.score(&request, &worker_flag));
    tokio::select! {
        done = &mut worker => match done {
            Ok(result) => Ok(result),
            Err(_) => Ok(Err(DrillError::Failed("the analysis thread stopped".to_owned()))),
        },
        () = cancel.cancelled() => {
            flag.cancel();
            // The worker checks the flag between units of work and ends soon.
            let _ = worker.await;
            Err(EngineError::Cancelled)
        }
    }
}

impl UnitPlayer {
    pub(super) async fn submit_drill(
        &mut self,
        activity: &Activity,
        response: Response,
        cancel: &CancellationToken,
    ) -> Result<ActivityResult> {
        let (scoring, jobs) = jobs_of(&self.unit, activity)?;
        let skill = "pronunciation";

        if scoring != Scoring::Pron {
            // Practice with nothing to score. The response may be a completion or
            // clips; no audio is kept either way.
            if !matches!(response, Response::Done | Response::Clips(_)) {
                return Err(ActivityError::WrongKind {
                    expected: "a completion or recorded clips",
                }
                .into());
            }
            let subject = self.subject(activity, skill);
            let response_id = subject.response_id.clone();
            self.recorder
                .record_unscored(
                    &subject,
                    Scorer::Deterministic,
                    "drill_completion/1",
                    "completion",
                    AttemptStatus::Insufficient,
                    "This drill has no scorer: it is practice.",
                    None,
                    serde_json::json!({ "clips": jobs.len() }),
                )
                .await?;
            return self
                .finish_activity(
                    activity,
                    skill,
                    None,
                    None,
                    ResultOutcome::Unscored(UnscoredReason::NoScorer),
                    vec![response_id],
                )
                .await;
        }

        let Response::Clips(clips) = response else {
            return Err(ActivityError::WrongKind {
                expected: "recorded clips",
            }
            .into());
        };
        check_clips(&clips)?;
        if clips.len() != jobs.len() {
            return Err(ActivityError::WrongLength {
                expected: jobs.len(),
                got: clips.len(),
            }
            .into());
        }
        self.score_clips(activity, skill, &jobs, clips, cancel)
            .await
    }

    async fn score_clips(
        &mut self,
        activity: &Activity,
        skill: &str,
        jobs: &[Job],
        clips: Vec<Clip>,
        cancel: &CancellationToken,
    ) -> Result<ActivityResult> {
        let mut response_ids = Vec::with_capacity(jobs.len());
        let mut reports: Vec<UtteranceReport> = Vec::new();
        let mut scores: Vec<f64> = Vec::new();
        let mut unscored: Option<String> = None;
        for (job, clip) in jobs.iter().zip(clips) {
            let subject = self.subject(activity, skill);
            response_ids.push(subject.response_id.clone());
            let reference = clip
                .says
                .clone()
                .unwrap_or_else(|| job.reference_text.clone());
            let Some(scorer) = self.env.drill.clone() else {
                let why = "no pronunciation engine is available in this run".to_owned();
                self.store_unscored_clip(&subject, &reference, &why).await?;
                unscored.get_or_insert(why);
                continue;
            };
            let request = DrillRequest {
                reference_text: reference.clone(),
                focus: job.focus.clone(),
                samples: clip.samples,
            };
            match score_blocking(scorer.clone(), request, cancel).await? {
                Ok(done) => {
                    let record = PronRecord {
                        report: &done.report,
                        engine: scorer.engine(),
                        threshold_set_version: scorer.threshold_set_version(),
                        reference_text: &reference,
                        mode: PronMode::Drill,
                        frame_ms: done.frame_ms,
                    };
                    self.recorder.record_pron(&subject, &record).await?;
                    if let Some(score) = done.report.utterance_score {
                        scores.push(f64::from(score));
                    } else {
                        unscored.get_or_insert_with(|| {
                            "the engine produced no score for this recording".to_owned()
                        });
                    }
                    reports.push(done.report);
                }
                Err(DrillError::Cancelled) => return Err(EngineError::Cancelled),
                Err(error) => {
                    let why = error.to_string();
                    self.store_unscored_clip(&subject, &reference, &why).await?;
                    unscored.get_or_insert(why);
                }
            }
        }
        let all_scored = unscored.is_none() && scores.len() == jobs.len();
        let score = all_scored.then(|| scores.iter().sum::<f64>() / scores.len() as f64);
        let outcome = match (&unscored, reports.is_empty()) {
            (Some(why), true) => {
                ResultOutcome::Unscored(UnscoredReason::EngineUnavailable { why: why.clone() })
            }
            _ => ResultOutcome::Pron(reports),
        };
        self.finish_activity(
            activity,
            skill,
            score,
            score.map(|_| PRON_CONFIDENCE_CAP),
            outcome,
            response_ids,
        )
        .await
    }

    async fn store_unscored_clip(
        &self,
        subject: &crate::evidence::Subject,
        reference: &str,
        why: &str,
    ) -> Result<()> {
        self.recorder
            .record_unscored(
                subject,
                Scorer::PronEngine,
                "pron_engine/unavailable",
                "pronunciation",
                AttemptStatus::Insufficient,
                &format!("Not scored: {why}."),
                Some(reference),
                serde_json::json!({ "kind": "reference_text" }),
            )
            .await?;
        Ok(())
    }
}
