//! A unit run: a lesson, a checkpoint or a drill run, played with
//! `tutor_engine::UnitPlayer`.
//!
//! * `lesson` plays every activity of the unit in its order.
//! * `checkpoint` plays only the activities the unit's checkpoint names.
//! * `drill` plays only the pronunciation drills of the unit.
//!
//! Some activities cannot be done with what this program has, and the run says
//! so instead of showing a button that fails: [`blocker`] names what is missing
//! for each, and `next_activity` lists them apart from the one to do next. A run
//! never scores what it could not measure and never invents an answer for it.
//!
//! A roleplay inside a unit is a text conversation: `roleplay_start` makes the
//! tutor speak first, the learner's lines go to the text route, and
//! `roleplay_finish` ends it and scores it when the activity has a rubric.

use std::sync::Arc;

use async_trait::async_trait;
use curriculum::{Activity, MinimalPairsMode, Scoring, Unit};
use storage::SessionStatus;
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    EngineError, Response, RoleplayRun, UnitConfig, UnitEnv, UnitPlayer, ensure_indexed,
};

use super::convert::{activity_result, presentation, unit_summary};
use super::emit::Emitter;
use super::env::{Env, engine_error};
use super::manager::{Ended, Run, Shared};
use super::text::{TextDriver, checked_message, recover_roleplay};
use crate::api::{
    ActiveSessionView, ActivityAnswer, ActivityOutcomeView, ActivityUnavailable, CheckpointRowView,
    FeedbackView, NextActivity, PronFindingsView, SessionChannel, SessionKind, SessionLife,
    StartSessionRequest, SubmitActivityResponse, TurnAccepted, TurnPhase, UnitStatus,
    UnitSummaryView,
};
use crate::error::{CoreError, CoreResult};

pub(crate) struct UnitRun {
    emit: Emitter,
    kind: SessionKind,
    unit: Unit,
    player: tokio::sync::Mutex<UnitPlayer>,
    /// The activities of this run, in the order they are offered.
    ids: Vec<String>,
    driver: TextDriver,
    roleplay: Arc<tokio::sync::Mutex<Option<RoleplayRun>>>,
    /// Speech output is configured, so audio items can be heard.
    speech_out: bool,
    session_token: CancellationToken,
}

fn storage_kind(kind: SessionKind) -> storage::SessionKind {
    match kind {
        SessionKind::Checkpoint => storage::SessionKind::Checkpoint,
        SessionKind::Drill => storage::SessionKind::Drill,
        _ => storage::SessionKind::Lesson,
    }
}

fn is_drill(activity: &Activity) -> bool {
    match activity {
        Activity::ReadAloud(_) | Activity::Shadowing(_) => true,
        Activity::MinimalPairs(a) => a.mode == MinimalPairsMode::SayBoth,
        _ => false,
    }
}

/// The activities a kind of run offers, in order.
fn scope(kind: SessionKind, unit: &Unit) -> Vec<String> {
    match kind {
        SessionKind::Checkpoint => unit.checkpoint.activity_ids.clone(),
        SessionKind::Drill => unit
            .activities
            .iter()
            .filter(|a| is_drill(a))
            .map(|a| a.id().to_owned())
            .collect(),
        _ => unit.activities.iter().map(|a| a.id().to_owned()).collect(),
    }
}

/// What stops this program from running an activity, if anything. A sentence for
/// the learner.
fn blocker(activity: &Activity, speech_out: bool) -> Option<&'static str> {
    const PRON: &str = "a pronunciation drill that is scored needs a recording of the learner and a phoneme model; this server has neither";
    const SPOKEN: &str = "a spoken answer needs one recording of the learner to go to the speech recogniser; this server listens only during a voice conversation";
    const AUDIO: &str =
        "this activity is played aloud and needs speech output, which is not available";
    let needs_audio = match activity {
        Activity::Dictation(_) | Activity::ListeningSet(_) | Activity::Shadowing(_) => true,
        Activity::Mcq(a) => a.audio_text.is_some(),
        Activity::MinimalPairs(a) => a.mode == MinimalPairsMode::ListenChoose,
        _ => false,
    };
    match activity {
        Activity::GuidedSpeaking(_) => return Some(SPOKEN),
        Activity::ReadAloud(a) if a.scoring == Scoring::Pron => return Some(PRON),
        Activity::Shadowing(a) if a.scoring == Scoring::Pron => return Some(PRON),
        Activity::MinimalPairs(a)
            if a.mode == MinimalPairsMode::SayBoth && a.scoring == Scoring::Pron =>
        {
            return Some(PRON);
        }
        _ => {}
    }
    (needs_audio && !speech_out).then_some(AUDIO)
}

pub(crate) async fn start(
    shared: &Arc<Shared>,
    env: Env,
    request: &StartSessionRequest,
) -> CoreResult<Arc<UnitRun>> {
    let unit_id = request.unit_id.as_deref().ok_or_else(|| {
        CoreError::InvalidInput("a lesson, checkpoint or drill run needs a unit".to_owned())
    })?;
    let unit = env.core.unit(unit_id).await?.unit;
    let ids = scope(request.kind, &unit);
    if ids.is_empty() {
        return Err(CoreError::InvalidInput(match request.kind {
            SessionKind::Drill => "the unit has no pronunciation drills".to_owned(),
            _ => "the unit has no activities for this kind of run".to_owned(),
        }));
    }
    let speech_out = {
        let source = shared.engines.speech().ok().cloned();
        let audio = shared.engines.audio().is_ok();
        match source {
            Some(source) if audio => tokio::task::spawn_blocking(move || {
                source.status().tts == crate::engines::PartStatus::Configured
            })
            .await
            .unwrap_or(false),
            _ => false,
        }
    };
    if request.kind == SessionKind::Drill
        && ids.iter().all(|id| {
            unit.activities
                .iter()
                .find(|a| a.id() == id)
                .is_some_and(|a| blocker(a, speech_out).is_some())
        })
    {
        return Err(CoreError::unavailable(
            None,
            "the pronunciation drills of this unit are scored from a recording by a phoneme model; this server has neither a recorder for the learner's clip nor the model",
        ));
    }

    let tts = shared.engine_model(crate::api::EngineId::Tts);
    let unit_env = UnitEnv {
        client: env.llm.clone(),
        db: env.db.clone(),
        clock: env.clock.clone(),
        model: env.model.clone(),
        provider_profile_id: env.provider_profile_id,
        provider_qualified: env.provider_qualified,
        grammar: shared.catalogs.grammar.clone(),
        word_levels: shared.catalogs.word_levels.clone(),
        rubrics: Arc::clone(&shared.catalogs.rubrics),
        drill: shared.engines.drill().cloned(),
        tts,
    };
    // The unit file was indexed when the program started; a unit that was not
    // (the index could not be written) gets its row here, once.
    let detail_checksum = unit_checksum(&unit);
    ensure_indexed(&env.db, &unit, &detail_checksum, (env.clock)())
        .await
        .map_err(engine_error)?;
    let player = UnitPlayer::start_as(
        unit_env,
        UnitConfig {
            profile_id: env.profile_id,
            first_language: env.first_language.clone(),
            app_version: env.app_version.clone(),
        },
        unit.clone(),
        storage_kind(request.kind),
    )
    .await
    .map_err(engine_error)?;
    let session_token = shared.shutdown.child_token();
    let emit = Emitter::new(
        Arc::clone(&shared.bus),
        ActiveSessionView {
            id: player.session_id(),
            kind: request.kind,
            unit_id: Some(unit.id.clone()),
            channel: SessionChannel::Text,
            life: SessionLife::Active,
            turn_state: Some(TurnPhase::Waiting),
            fault: None,
            turns_completed: 0,
            turn: 0,
            recent: Vec::new(),
            pending_reply: None,
            speaking: speech_out,
            activity_id: ids.first().cloned(),
        },
    );
    let driver = TextDriver::new(emit.clone(), env.db.clone(), session_token.clone());
    Ok(Arc::new(UnitRun {
        emit,
        kind: request.kind,
        unit,
        player: tokio::sync::Mutex::new(player),
        ids,
        driver,
        roleplay: Arc::new(tokio::sync::Mutex::new(None)),
        speech_out,
        session_token,
    }))
}

/// A checksum for the unit when the index has no row for it.
fn unit_checksum(unit: &Unit) -> String {
    let text = serde_json::to_string(unit).unwrap_or_default();
    curriculum::sha256_hex(text.as_bytes())
}

fn response_of(answer: &ActivityAnswer) -> CoreResult<Response> {
    let index = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    Ok(match answer {
        ActivityAnswer::Choice { index: n } => Response::Choice(index(*n)),
        ActivityAnswer::Gaps { answers } => Response::Gaps(answers.clone()),
        ActivityAnswer::Order { tokens } => Response::Order(tokens.clone()),
        ActivityAnswer::Picks { picks } => {
            Response::Picks(picks.iter().map(|p| p.map(index)).collect())
        }
        ActivityAnswer::Text { text } => Response::Text(text.clone()),
        ActivityAnswer::Done => Response::Done,
        ActivityAnswer::RoleplayStart | ActivityAnswer::RoleplayFinish => {
            return Err(CoreError::InvalidInput(
                "a roleplay is answered with its own start and finish".to_owned(),
            ));
        }
    })
}

impl UnitRun {
    fn activity(&self, id: &str) -> CoreResult<&Activity> {
        if !self.ids.iter().any(|known| known == id) {
            return Err(CoreError::InvalidInput(
                "this activity is not part of this run".to_owned(),
            ));
        }
        self.unit
            .activities
            .iter()
            .find(|a| a.id() == id)
            .ok_or(CoreError::NotFound { what: "activity" })
    }

    fn publish_result(
        &self,
        result: &tutor_engine::ActivityResult,
    ) -> crate::api::ActivityResultView {
        let view = activity_result(result);
        if let ActivityOutcomeView::Pron { findings } = &view.outcome {
            for finding in findings {
                let finding: PronFindingsView = finding.clone();
                self.emit.pron(&view.activity_id, finding);
            }
        }
        self.emit.feedback(FeedbackView::Activity {
            result: view.clone(),
        });
        view
    }

    async fn roleplay_start(&self, id: &str) -> CoreResult<SubmitActivityResponse> {
        let mut slot = self.roleplay.lock().await;
        if slot.is_some() {
            return Err(CoreError::Conflict(
                "a roleplay is already running".to_owned(),
            ));
        }
        let run = {
            let mut player = self.player.lock().await;
            player
                .start_roleplay(id)
                .map_err(engine_error)?
                .with_observer(self.driver.observer())
        };
        *slot = Some(run);
        drop(slot);
        let roleplay = Arc::clone(&self.roleplay);
        let begun = self.driver.begin(None, move |on_delta, token| async move {
            let mut slot = roleplay.lock().await;
            let Some(run) = slot.as_mut() else {
                return Err(EngineError::Refused("no roleplay is running"));
            };
            let result = run.open(on_delta, &token).await;
            recover_roleplay(run, result)
        });
        if let Err(error) = begun {
            *self.roleplay.lock().await = None;
            return Err(error);
        }
        self.emit.set_activity(Some(id.to_owned()));
        Ok(SubmitActivityResponse { result: None })
    }

    async fn roleplay_finish(&self) -> CoreResult<SubmitActivityResponse> {
        if self.driver.is_busy() {
            return Err(CoreError::Busy);
        }
        let run = self
            .roleplay
            .lock()
            .await
            .take()
            .ok_or_else(|| CoreError::Conflict("no roleplay is running".to_owned()))?;
        let token = self.session_token.child_token();
        let result = self
            .player
            .lock()
            .await
            .finish_roleplay(run, &token)
            .await
            .map_err(engine_error)?;
        let view = self.publish_result(&result);
        Ok(SubmitActivityResponse { result: Some(view) })
    }
}

#[async_trait]
impl Run for UnitRun {
    fn emitter(&self) -> &Emitter {
        &self.emit
    }

    async fn next_activity(&self) -> CoreResult<NextActivity> {
        let player = self.player.lock().await;
        let mut unavailable = Vec::new();
        let mut next = None;
        let mut answered = 0_u32;
        for id in &self.ids {
            if player.result(id).is_some() {
                answered += 1;
                continue;
            }
            let Some(activity) = self.unit.activities.iter().find(|a| a.id() == id) else {
                continue;
            };
            if let Some(reason) = blocker(activity, self.speech_out) {
                unavailable.push(ActivityUnavailable {
                    activity_id: id.clone(),
                    reason: reason.to_owned(),
                });
                continue;
            }
            if next.is_none() {
                next = Some(player.present(id).map_err(engine_error)?);
            }
        }
        self.emit.set_activity(next.as_ref().map(|p| p.id.clone()));
        Ok(NextActivity {
            session_id: self.emit.id(),
            answered,
            total: u32::try_from(self.ids.len()).unwrap_or(u32::MAX),
            activity: next.as_ref().map(presentation),
            unavailable,
        })
    }

    async fn submit_activity(
        &self,
        activity_id: String,
        answer: ActivityAnswer,
    ) -> CoreResult<SubmitActivityResponse> {
        let activity = self.activity(&activity_id)?;
        if let Some(reason) = blocker(activity, self.speech_out) {
            return Err(CoreError::unavailable(
                Some(crate::api::Feature::Speech),
                format!("this activity cannot be done here: {reason}"),
            ));
        }
        match answer {
            ActivityAnswer::RoleplayStart => self.roleplay_start(&activity_id).await,
            ActivityAnswer::RoleplayFinish => self.roleplay_finish().await,
            other => {
                let response = response_of(&other)?;
                let token = self.session_token.child_token();
                let result = self
                    .player
                    .lock()
                    .await
                    .submit(&activity_id, response, &token)
                    .await
                    .map_err(engine_error)?;
                let view = self.publish_result(&result);
                Ok(SubmitActivityResponse { result: Some(view) })
            }
        }
    }

    async fn activity_audio(&self, activity_id: &str) -> CoreResult<String> {
        self.activity(activity_id)?;
        let play = self
            .player
            .lock()
            .await
            .play_audio(activity_id)
            .map_err(engine_error)?;
        Ok(play
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(" "))
    }

    async fn send_text(&self, text: String) -> CoreResult<TurnAccepted> {
        let text = checked_message(&text)?;
        if self.roleplay.lock().await.is_none() {
            return Err(CoreError::Conflict(
                "no roleplay is running: answer the roleplay activity with roleplay_start first"
                    .to_owned(),
            ));
        }
        let roleplay = Arc::clone(&self.roleplay);
        let message = text.clone();
        self.driver
            .begin(Some(text), move |on_delta, token| async move {
                let mut slot = roleplay.lock().await;
                let Some(run) = slot.as_mut() else {
                    return Err(EngineError::Refused("no roleplay is running"));
                };
                let result = run.say(&message, on_delta, &token).await;
                recover_roleplay(run, result)
            })
    }

    async fn finish(&self, cancel: bool) -> CoreResult<Ended> {
        self.driver.cancel_running().await;
        let token = self.session_token.child_token();
        let mut player = self.player.lock().await;
        if let Some(run) = self.roleplay.lock().await.take()
            && !cancel
            && let Err(error) = player.finish_roleplay(run, &token).await
        {
            tracing::warn!(%error, "the roleplay could not be finished");
        }
        if cancel {
            player.abort().await.map_err(engine_error)?;
            return Ok(Ended {
                status: SessionStatus::Aborted,
                feedback: None,
            });
        }
        if self.kind == SessionKind::Drill {
            player.finish_practice().await.map_err(engine_error)?;
            return Ok(Ended {
                status: SessionStatus::Completed,
                feedback: None,
            });
        }
        match player.finish().await {
            Ok(summary) => Ok(Ended {
                status: SessionStatus::Completed,
                feedback: Some(FeedbackView::Unit {
                    summary: unit_summary(&self.unit.id, &summary),
                }),
            }),
            // The checkpoint still has activities to answer: the unit's progress
            // does not change, and the run is closed as left unfinished.
            Err(EngineError::Refused(_)) => {
                let report = player.checkpoint().await.map_err(engine_error)?;
                player.abort().await.map_err(engine_error)?;
                Ok(Ended {
                    status: SessionStatus::Aborted,
                    feedback: Some(FeedbackView::Unit {
                        summary: UnitSummaryView {
                            unit_id: self.unit.id.clone(),
                            rows: report
                                .rows
                                .iter()
                                .map(|r| CheckpointRowView {
                                    activity_id: r.activity_id.clone(),
                                    answered: r.answered,
                                    score: r.score,
                                })
                                .collect(),
                            mean: report.outcome.mean,
                            pass_mark: report.pass_mark,
                            passed: false,
                            provisional: report.outcome.provisional,
                            unit_status: UnitStatus::InProgress,
                            answered: u32::try_from(
                                report.rows.iter().filter(|r| r.answered).count(),
                            )
                            .unwrap_or(u32::MAX),
                        },
                    }),
                })
            }
            Err(other) => Err(engine_error(other)),
        }
    }
}
