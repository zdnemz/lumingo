//! A voice conversation: the voice loop of this crate, driven by the API.
//!
//! The loop runs on its own threads and tasks (see `crate::voice`); this file
//! starts it with the engines that loaded, forwards what it says to the event bus
//! and carries the learner's commands to it. Two small tasks belong to the run:
//!
//! * the forwarder turns [`VoiceEvent`]s into server events, and
//! * the level ticker reads the microphone level every 100 ms, so `MicLevel` is
//!   published at most ten times a second whatever the audio rate is. It sends
//!   nothing while the level is silent, so a conversation without a microphone
//!   adds no events.
//!
//! Both tasks start in [`Run::begin`], after the manager has published
//! `SessionState` `active`, so no event of the session comes before it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use storage::SessionStatus;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use tutor_engine::{Channel, Focus, TutorContext};

use super::convert::{self, phase_parts};
use super::emit::Emitter;
use super::env::{Env, assessment_level, missing_engine};
use super::manager::{Ended, Run, STOP_WAIT, Shared, lock};
use super::text::{chat_topic, checked_message, feedback_mode, stored_summary};
use crate::api::{
    ActiveSessionView, EditEffect, EngineId, EngineModelInfo, EngineState, EngineView,
    FeedbackView, ServerEvent, SessionChannel, SessionKind, SessionLife, StartSessionRequest,
    TurnAccepted, TurnPhase,
};
use crate::engines::PartStatus;
use crate::error::{CoreError, CoreResult};
use crate::voice::{
    AudioMode, EditEffect as VoiceEdit, Recording, Scenario, SystemLoopClock, TurnOutcome,
    VoiceConfig, VoiceError, VoiceEvent, VoiceHandle, VoiceLoop, VoiceParts,
};

/// How often the microphone level is published.
const LEVEL_EVERY: Duration = Duration::from_millis(100);

/// A level below this is silence for the level meter.
const SILENT: f32 = 0.005;

pub(crate) struct VoiceRun {
    emit: Emitter,
    handle: VoiceHandle,
    voice: tokio::sync::Mutex<Option<VoiceLoop>>,
    env: Env,
    speaks: bool,
    session_token: CancellationToken,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// What the loop says, from before the first turn. [`Run::begin`] hands it to
    /// the forwarder, so nothing the session publishes comes before its
    /// `SessionState` `active`.
    events: Mutex<Option<tokio::sync::broadcast::Receiver<VoiceEvent>>>,
    bus: Arc<crate::events::EventBus>,
}

/// The prompt context of a conversation outside a unit.
fn scenario_of(env: &Env, request: &StartSessionRequest, shared: &Shared) -> CoreResult<Scenario> {
    let level = request
        .level
        .map_or(assessment_engine::Level::A1, assessment_level);
    let mode = feedback_mode(request);
    let (focus, scenario, tutor_role, learner_role, goals) = match chat_topic(shared, request)? {
        tutor_engine::ChatTopic::Bank(topic) => (
            Focus::Topic(topic.title.en.clone()),
            topic.scenario.en.clone(),
            topic.tutor_role.clone(),
            topic.learner_role.clone(),
            topic.goals.clone(),
        ),
        tutor_engine::ChatTopic::Typed(text) => (
            Focus::Topic(tutor_engine::clean_topic(&text)),
            "an open conversation about this topic".to_owned(),
            "a friendly conversation partner".to_owned(),
            "a conversation partner".to_owned(),
            vec![
                "keep the conversation going for several turns".to_owned(),
                "give and ask for opinions or details".to_owned(),
            ],
        ),
        tutor_engine::ChatTopic::Unit(_) => {
            return Err(CoreError::InvalidInput(
                "a unit conversation is started with a unit".to_owned(),
            ));
        }
    };
    Ok(Scenario {
        context: TutorContext {
            channel: Channel::Voice,
            level,
            first_language: env.first_language.clone(),
            focus,
            scenario,
            tutor_role,
            learner_role,
            goals,
            target_language: Vec::new(),
            mode,
            pronunciation_findings: false,
        },
        unit_id: None,
        activity_id: None,
        objectives: Vec::new(),
        target_language: Vec::new(),
        max_turns: None,
    })
}

fn voice_error(error: VoiceError) -> CoreError {
    match error {
        VoiceError::NoSuchTurn => CoreError::NotFound { what: "turn" },
        VoiceError::NotSpoken | VoiceError::NotRecording => {
            CoreError::InvalidInput(error.to_string())
        }
        VoiceError::Stopped => CoreError::Conflict("the voice session has stopped".to_owned()),
        VoiceError::EngineUnavailable { engine, reason } => CoreError::unavailable(
            Some(crate::api::Feature::Speech),
            format!("the {engine} engine is not available: {reason}"),
        ),
        VoiceError::EngineLoadTimeout { engine, seconds } => CoreError::unavailable(
            Some(crate::api::Feature::Speech),
            format!("the {engine} engine did not load within {seconds} s"),
        ),
        VoiceError::Storage(error) => CoreError::Storage(error),
        other => CoreError::Conflict(other.to_string()),
    }
}

fn engine_view(
    id: EngineId,
    state: EngineState,
    detail: &str,
    info: Option<&speech::EngineInfo>,
) -> EngineView {
    EngineView {
        id,
        state,
        detail: detail.to_owned(),
        model: info.map(|i| EngineModelInfo {
            id: i.id.clone(),
            version: i.version.clone(),
            model_checksum: i.model_checksum.clone(),
        }),
    }
}

/// The engines a conversation loads: the synthesiser only when replies are spoken.
fn loading_ids(speaks: bool) -> Vec<EngineId> {
    let mut ids = vec![EngineId::Vad, EngineId::Stt];
    if speaks {
        ids.push(EngineId::Tts);
    }
    ids
}

pub(crate) async fn start(
    shared: &Arc<Shared>,
    env: Env,
    request: &StartSessionRequest,
) -> CoreResult<Arc<VoiceRun>> {
    let speaks = request.speak.unwrap_or(true);
    let registry = shared
        .engines
        .audio()
        .map_err(|reason| {
            missing_engine(&format!(
                "a voice conversation needs a microphone and speakers: {reason}"
            ))
        })?
        .clone();
    let source = shared
        .engines
        .speech()
        .map_err(|reason| {
            missing_engine(&format!(
                "a voice conversation needs speech engines: {reason}"
            ))
        })?
        .clone();
    let looked = {
        let source = Arc::clone(&source);
        tokio::task::spawn_blocking(move || source.status())
            .await
            .map_err(|_| {
                CoreError::Internal("looking at the speech engines did not finish".to_owned())
            })?
    };
    for (part, name) in [(&looked.vad, "VAD"), (&looked.stt, "speech recognition")] {
        if let PartStatus::Missing(reason) = part {
            return Err(missing_engine(&format!(
                "a voice conversation needs {name}: {reason}"
            )));
        }
    }
    if speaks && let PartStatus::Missing(reason) = &looked.tts {
        return Err(missing_engine(&format!(
            "spoken replies need speech synthesis: {reason}; start the conversation with speak off to read the replies"
        )));
    }

    // The scenario: a unit's roleplay, or a topic.
    let (scenario, link_unit) = match &request.unit_id {
        Some(unit_id) => {
            let unit = env.core.unit(unit_id).await?.unit;
            let scenario = Scenario::from_unit(
                &unit,
                request.activity_id.as_deref(),
                request.mode.map(|m| match m {
                    crate::api::FeedbackMode::Fluency => tutor_engine::FeedbackMode::Fluency,
                    crate::api::FeedbackMode::Accuracy => tutor_engine::FeedbackMode::Accuracy,
                }),
                &env.first_language,
            )
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
            (scenario, true)
        }
        None => (scenario_of(&env, request, shared)?, false),
    };

    // Loading a VAD reads a model file: not on the runtime.
    for id in loading_ids(speaks) {
        shared.set_engine(engine_view(
            id,
            EngineState::Loading,
            "loading the model",
            None,
        ));
    }
    let loaded = {
        let source = Arc::clone(&source);
        tokio::task::spawn_blocking(move || {
            let listen = source.listen()?;
            let tts = if speaks { Some(source.tts()?) } else { None };
            Ok::<_, crate::engines::EngineProblem>((listen, tts))
        })
        .await
        .map_err(|_| CoreError::Internal("loading the speech engines did not finish".to_owned()))?
    };
    let (listen, tts) = match loaded {
        Ok(parts) => parts,
        Err(problem) => {
            for id in loading_ids(speaks) {
                shared.set_engine(engine_view(id, EngineState::Failed, &problem.0, None));
            }
            return Err(CoreError::unavailable(
                Some(crate::api::Feature::Speech),
                problem.0,
            ));
        }
    };

    let audio = AudioMode::Full(registry);
    let recording = Recording {
        db: env.db.clone(),
        profile_id: env.profile_id,
        provider_profile_id: env.provider_profile_id,
        model: env.model.clone(),
        app_version: env.app_version.clone(),
        link_unit,
        clock: env.clock.clone(),
    };
    let voice = match VoiceLoop::start(VoiceParts {
        llm: env.llm.clone(),
        clock: Arc::new(SystemLoopClock::new()),
        scenario,
        config: VoiceConfig::default(),
        audio,
        listen: Some(listen),
        tts,
        recording: Some(recording),
    })
    .await
    {
        Ok(voice) => voice,
        Err(error) => {
            let detail = error.to_string();
            for id in loading_ids(speaks) {
                shared.set_engine(engine_view(id, EngineState::Failed, &detail, None));
            }
            return Err(voice_error(error));
        }
    };
    let handle = voice.handle();
    let infos = handle.engines();
    shared.set_engine(engine_view(
        EngineId::Vad,
        EngineState::Ready,
        "the voice activity detector loaded",
        None,
    ));
    shared.set_engine(engine_view(
        EngineId::Stt,
        EngineState::Ready,
        "the recogniser loaded",
        infos.stt.as_ref(),
    ));
    if speaks {
        shared.set_engine(engine_view(
            EngineId::Tts,
            EngineState::Ready,
            "the synthesiser loaded",
            infos.tts.as_ref(),
        ));
    }

    let session_id = handle
        .session_id()
        .ok_or_else(|| CoreError::Internal("the voice session was not stored".to_owned()))?;
    let emit = Emitter::new(
        Arc::clone(&shared.bus),
        ActiveSessionView {
            id: session_id,
            kind: SessionKind::Conversation,
            unit_id: request.unit_id.clone(),
            channel: SessionChannel::Voice,
            life: SessionLife::Active,
            turn_state: Some(TurnPhase::Listening),
            fault: None,
            turns_completed: 0,
            turn: 0,
            recent: Vec::new(),
            pending_reply: None,
            speaking: speaks,
            activity_id: None,
        },
    );
    let session_token = shared.shutdown.child_token();
    let run = Arc::new(VoiceRun {
        emit: emit.clone(),
        handle: handle.clone(),
        voice: tokio::sync::Mutex::new(Some(voice)),
        env,
        speaks,
        session_token,
        tasks: Mutex::new(Vec::new()),
        events: Mutex::new(Some(handle.subscribe())),
        bus: Arc::clone(&shared.bus),
    });
    Ok(run)
}

/// Publishes the microphone level, at most ten times a second, until the session
/// ends. A level that is silent and was silent at the last tick is not published,
/// so a conversation without a microphone, or a quiet room, sends nothing.
async fn level_ticker(
    handle: VoiceHandle,
    bus: Arc<crate::events::EventBus>,
    stop: CancellationToken,
) {
    // The first tick is one period away, so no window of a second holds eleven.
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + LEVEL_EVERY, LEVEL_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last = 0.0_f32;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = tick.tick() => {
                let level = handle.mic_level();
                if level < SILENT && last < SILENT {
                    continue;
                }
                last = level;
                bus.publish(|seq| ServerEvent::MicLevel { seq, level });
            }
        }
    }
}

/// Turns the loop's events into server events.
async fn forward(
    mut events: tokio::sync::broadcast::Receiver<VoiceEvent>,
    handle: VoiceHandle,
    emit: Emitter,
) {
    loop {
        match events.recv().await {
            Ok(VoiceEvent::Closed) | Err(RecvError::Closed) => return,
            Ok(event) => on_event(&event, &handle, &emit),
            Err(RecvError::Lagged(_)) => {
                // Events were lost: say where the loop is now.
                on_event(&VoiceEvent::State(handle.phase()), &handle, &emit);
            }
        }
    }
}

fn on_event(event: &VoiceEvent, handle: &VoiceHandle, emit: &Emitter) {
    match event {
        VoiceEvent::State(phase) => {
            let (life, turn_state, fault) = phase_parts(*phase);
            match turn_state {
                Some(state) => emit.turn_state(handle.current_turn(), state, None),
                None if life != SessionLife::Ended => {
                    emit.session_state_if_changed(life, fault, None);
                }
                None => {}
            }
        }
        VoiceEvent::Heard { turn, text, voice } => {
            let source = if *voice {
                SessionChannel::Voice
            } else {
                SessionChannel::Text
            };
            emit.learner_line(*turn, text, source, None, false);
        }
        VoiceEvent::Recorded {
            turn,
            learner_seq,
            text,
        } => emit.learner_line(
            *turn,
            text,
            SessionChannel::Voice,
            Some(*learner_seq),
            false,
        ),
        VoiceEvent::TutorSentence { turn, index, text } => {
            emit.sentence_spoken(*turn, *index, text);
        }
        VoiceEvent::Latency(latency) => {
            let record = latency.record();
            emit.latency(
                latency.turn,
                [
                    record.endpointing_wait_ms,
                    record.stt_finalise_ms,
                    record.llm_first_sentence_ms,
                    record.tts_first_sentence_ms,
                    record.output_start_ms,
                ],
                record.sum_ms,
                record.complete,
            );
        }
        VoiceEvent::TurnEnded { turn, outcome } => {
            emit.set_turns_completed(handle.turns_completed());
            let reply = emit.take_pending_reply();
            if let Some(text) = reply.filter(|_| {
                matches!(
                    outcome,
                    TurnOutcome::Replied | TurnOutcome::Fallback | TurnOutcome::Stopped
                )
            }) {
                emit.tutor_line(*turn, &text, None);
            }
        }
        VoiceEvent::ProviderUnavailable { message } => {
            emit.clear_pending_reply();
            emit.session_state(
                SessionLife::ProviderUnavailable,
                None,
                Some(message.clone()),
            );
        }
        VoiceEvent::EngineError { fault, message } => {
            emit.session_state(
                SessionLife::EngineError,
                Some(convert::fault_view(*fault)),
                Some(message.clone()),
            );
        }
        VoiceEvent::Analysed(report) => {
            for record in &report.analysed {
                emit.analysis(convert::analysis(record));
            }
        }
        VoiceEvent::SpeechStarted | VoiceEvent::Stopped(_) | VoiceEvent::Closed => {}
    }
}

#[async_trait]
impl Run for VoiceRun {
    fn emitter(&self) -> &Emitter {
        &self.emit
    }

    async fn begin(&self) {
        let receiver = lock(&self.events).take();
        if let Some(receiver) = receiver {
            let forwarder = tokio::spawn(forward(receiver, self.handle.clone(), self.emit.clone()));
            let ticker = tokio::spawn(level_ticker(
                self.handle.clone(),
                Arc::clone(&self.bus),
                self.session_token.clone(),
            ));
            lock(&self.tasks).extend([forwarder, ticker]);
        }
        // The tutor speaks first. A refusal here only means the loop already ended.
        if let Err(error) = self.handle.open().await {
            tracing::warn!(%error, "the opening turn could not be started");
        }
    }

    async fn send_text(&self, text: String) -> CoreResult<TurnAccepted> {
        let text = checked_message(&text)?;
        if self.emit.view().life != SessionLife::Active {
            return Err(CoreError::Conflict(
                "the session is not running: resume it first".to_owned(),
            ));
        }
        self.handle.send_text(text).await.map_err(voice_error)?;
        Ok(TurnAccepted {
            session_id: self.emit.id(),
            turn: self.handle.current_turn() + 1,
        })
    }

    async fn edit_turn(&self, turn: u64, text: String) -> CoreResult<EditEffect> {
        let text = checked_message(&text)?;
        let effect = self
            .handle
            .edit_transcript(turn, text.clone())
            .await
            .map_err(voice_error)?;
        self.emit
            .learner_line(turn, &text, SessionChannel::Voice, None, true);
        Ok(match effect {
            VoiceEdit::BeforeRecording => EditEffect::BeforeRecording,
            VoiceEdit::AnalysedFromEdit => EditEffect::AnalysedFromEdit,
            VoiceEdit::StoredOnly => EditEffect::StoredOnly,
        })
    }

    async fn push_to_talk(&self, pressed: bool) -> CoreResult<()> {
        self.handle.push_to_talk(pressed);
        Ok(())
    }

    async fn stop_speaking(&self) -> CoreResult<()> {
        self.handle.stop_speaking().await.map_err(voice_error)
    }

    async fn pause(&self) -> CoreResult<()> {
        self.handle.pause().await.map_err(voice_error)?;
        // Said here so the answer to the request shows the new state; the loop's own
        // event for it is then a repeat and is not sent.
        self.emit
            .session_state_if_changed(SessionLife::Paused, None, None);
        Ok(())
    }

    async fn resume(&self) -> CoreResult<()> {
        self.handle.resume().await.map_err(voice_error)?;
        self.emit.session_state(SessionLife::Active, None, None);
        Ok(())
    }

    fn speaker(&self) -> Option<VoiceHandle> {
        self.speaks.then(|| self.handle.clone())
    }

    async fn finish(&self, cancel: bool) -> CoreResult<Ended> {
        self.session_token.cancel();
        let voice = self.voice.lock().await.take();
        let session_id = self.emit.id();
        if let Some(voice) = voice {
            // A normal end analyses what is waiting; it may not wait for ever.
            if cancel {
                let _ = voice.finish(true).await;
            } else if tokio::time::timeout(STOP_WAIT, voice.finish(false))
                .await
                .is_err()
            {
                tracing::warn!("the voice session did not end in time");
            }
        }
        let tasks = std::mem::take(&mut *lock(&self.tasks));
        for task in tasks {
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
        if cancel {
            return Ok(Ended {
                status: SessionStatus::Aborted,
                feedback: None,
            });
        }
        let summary = stored_summary(&self.env.db, session_id).await?;
        Ok(Ended {
            status: SessionStatus::Completed,
            feedback: Some(FeedbackView::Conversation { summary }),
        })
    }
}
