//! Speech outside a conversation: speaking a stored text, listing the audio
//! devices, and the microphone test of the first-run wizard.
//!
//! A request never carries text to speak. [`speak`] takes a reference to
//! something the server stored (a tutor turn, a generated reading text, the audio
//! lines of an activity of the running unit) and reads the text from there.
//!
//! The speech output is a [`crate::voice::VoiceLoop`] without a microphone: the
//! synthesiser's worker, the playback queue and the stop that cancels them are the
//! ones a conversation uses. It is started on the first request and kept for the
//! next one, because loading a synthesiser takes time. A conversation that
//! starts, an audio test and the end of the program close it, since the devices
//! belong to them.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use audio_io::{
    AudioSession, DeviceId, DeviceInfo, DeviceRegistry, Direction, FrameSink, Route, SessionConfig,
};
use speech::PcmChunk;
use tutor_engine::{
    Channel, FeedbackMode, Focus, NoProvider, Phase, SentenceChunker, TurnState, TutorContext,
};

use super::env::{engine_error, missing_engine};
use super::manager::{Shared, lock};
use crate::api::{
    AudioDeviceView, AudioDevices, AudioTestReport, AudioTestRequest, EngineId, EngineModelInfo,
    EngineState, EngineView, Feature, ServerEvent, SpeakAccepted, SpeakRequest,
};
use crate::core::AppCore;
use crate::engines::PartStatus;
use crate::error::{CoreError, CoreResult};
use crate::voice::{
    AudioMode, Scenario, SystemLoopClock, VoiceConfig, VoiceEvent, VoiceHandle, VoiceLoop,
    VoiceParts,
};

/// The shortest and the longest microphone test, in milliseconds.
pub(crate) const MIN_TEST_MS: u32 = 500;
pub(crate) const MAX_TEST_MS: u32 = 5_000;

/// How often the level of a microphone test is published.
const LEVEL_EVERY: Duration = Duration::from_millis(100);

/// A recording of the test holds at most this many 16 kHz samples (the longest
/// test), so memory stays bounded whatever the device does.
const MAX_TEST_SAMPLES: usize = MAX_TEST_MS as usize * 16;

/// Below this peak the microphone is called silent.
const SILENT_PEAK: f32 = 0.01;

/// The speech output outside a conversation, started on first use.
#[derive(Default)]
pub(crate) struct StandaloneSpeaker {
    inner: tokio::sync::Mutex<Option<VoiceLoop>>,
    forwarder: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl StandaloneSpeaker {
    /// Closes the speech output and frees the devices. Nothing happens when it
    /// was not started.
    pub(crate) async fn close(&self) {
        let voice = self.inner.lock().await.take();
        if let Some(voice) = voice {
            let _ = voice.finish(true).await;
        }
        let task = lock(&self.forwarder).take();
        if let Some(task) = task {
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
    }

    /// Stops what is being spoken. Nothing is spoken when it was not started.
    pub(crate) async fn stop(&self) -> CoreResult<()> {
        let handle = self.inner.lock().await.as_ref().map(VoiceLoop::handle);
        match handle {
            Some(handle) => handle
                .stop_speaking()
                .await
                .map_err(|_| CoreError::Conflict("the speech output has stopped".to_owned())),
            None => Ok(()),
        }
    }

    async fn handle(&self, shared: &Arc<Shared>) -> CoreResult<VoiceHandle> {
        let mut inner = self.inner.lock().await;
        if let Some(voice) = inner.as_ref() {
            return Ok(voice.handle());
        }
        let registry = shared
            .engines
            .audio()
            .map_err(|reason| {
                missing_engine(&format!("speech output needs audio devices: {reason}"))
            })?
            .clone();
        let source = shared
            .engines
            .speech()
            .map_err(|reason| {
                missing_engine(&format!("speech output needs a synthesiser: {reason}"))
            })?
            .clone();
        let tts = {
            let source = Arc::clone(&source);
            tokio::task::spawn_blocking(move || {
                if let PartStatus::Missing(reason) = source.status().tts {
                    return Err(crate::engines::EngineProblem(reason));
                }
                source.tts()
            })
            .await
            .map_err(|_| CoreError::Internal("loading the synthesiser did not finish".to_owned()))?
        }
        .map_err(|problem| {
            CoreError::unavailable(
                Some(Feature::Speech),
                format!("speech output needs a synthesiser: {}", problem.0),
            )
        })?;
        shared.set_engine(engine_view(
            EngineId::Tts,
            EngineState::Loading,
            "loading the model",
            None,
        ));
        let voice = VoiceLoop::start(VoiceParts {
            llm: Arc::new(NoProvider),
            clock: Arc::new(SystemLoopClock::new()),
            scenario: idle_scenario(),
            config: VoiceConfig::default(),
            audio: AudioMode::PlaybackOnly(registry),
            listen: None,
            tts: Some(tts),
            recording: None,
        })
        .await
        .map_err(|error| {
            shared.set_engine(engine_view(
                EngineId::Tts,
                EngineState::Failed,
                &error.to_string(),
                None,
            ));
            CoreError::unavailable(
                Some(Feature::Speech),
                format!("speech output could not start: {error}"),
            )
        })?;
        let handle = voice.handle();
        let info = handle.engines().tts;
        shared.set_engine(engine_view(
            EngineId::Tts,
            EngineState::Ready,
            "the synthesiser loaded",
            info.as_ref(),
        ));
        // A failure of the synthesiser or of the speakers is told to the screen.
        let mut events = handle.subscribe();
        let bus = Arc::clone(&shared.bus);
        let forwarder = tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                match event {
                    VoiceEvent::Closed => return,
                    VoiceEvent::EngineError { message, .. } => {
                        bus.publish(|seq| ServerEvent::Error {
                            seq,
                            session_id: None,
                            code: crate::api::ErrorCode::NotAvailable,
                            message: format!("speech output failed: {message}"),
                        });
                    }
                    _ => {}
                }
            }
        });
        *lock(&self.forwarder) = Some(forwarder);
        *inner = Some(voice);
        Ok(handle)
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

/// The scenario of a loop that never asks the model anything.
fn idle_scenario() -> Scenario {
    Scenario {
        context: TutorContext {
            channel: Channel::Voice,
            level: assessment_engine::Level::A1,
            first_language: "English".to_owned(),
            focus: Focus::Topic("speech output".to_owned()),
            scenario: "the tutor reads a stored text aloud".to_owned(),
            tutor_role: "a reader".to_owned(),
            learner_role: "a listener".to_owned(),
            goals: Vec::new(),
            target_language: Vec::new(),
            mode: FeedbackMode::Fluency,
            pronunciation_findings: false,
        },
        unit_id: None,
        activity_id: None,
        objectives: Vec::new(),
        target_language: Vec::new(),
        max_turns: None,
    }
}

/// Where a text is spoken.
pub(crate) enum SpeakRoute {
    /// By the running voice conversation.
    Through(VoiceHandle),
    /// The running conversation does not speak, and it holds the devices.
    Blocked,
    /// By the speech output of its own.
    Standalone,
}

/// The sentences of `text`, as the speech output will cut them.
pub(crate) fn sentence_count(text: &str) -> u32 {
    let mut chunker = SentenceChunker::new();
    let mut count = chunker.push(text).len();
    if chunker.finish().is_some() {
        count += 1;
    }
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Reads the text a request refers to. The audio lines of an activity come from
/// the running session, which counts the play.
pub(crate) async fn stored_text(core: &AppCore, request: &SpeakRequest) -> CoreResult<String> {
    match request {
        SpeakRequest::Turn {
            session_id,
            turn_seq,
        } => {
            let turns = core.db.turns().list(*session_id).await?;
            let turn = turns
                .into_iter()
                .find(|t| t.seq == *turn_seq)
                .ok_or(CoreError::NotFound { what: "turn" })?;
            if turn.role != storage::TurnRole::Tutor {
                return Err(CoreError::InvalidInput(
                    "only the tutor's words can be spoken".to_owned(),
                ));
            }
            Ok(turn.text)
        }
        SpeakRequest::Reading {
            session_id,
            content_id,
        } => {
            let texts = tutor_engine::list_generated_readings(&core.db, *session_id)
                .await
                .map_err(engine_error)?;
            texts
                .into_iter()
                .find(|t| t.content_id == *content_id)
                .map(|t| t.reading.passage)
                .ok_or(CoreError::NotFound {
                    what: "reading text",
                })
        }
        SpeakRequest::Activity { .. } => Err(CoreError::InvalidInput(
            "the audio of an activity is read from its session".to_owned(),
        )),
    }
}

/// Speaks the text that `fetch` reads. The speech output is looked at first: a
/// request it cannot take now is refused as busy before the text is read, so a
/// refusal does not count as a play of a listening item.
pub(crate) async fn speak(
    shared: &Arc<Shared>,
    speaker: &StandaloneSpeaker,
    route: SpeakRoute,
    fetch: impl std::future::Future<Output = CoreResult<String>>,
) -> CoreResult<SpeakAccepted> {
    let handle = match route {
        SpeakRoute::Through(handle) => handle,
        SpeakRoute::Blocked => {
            return Err(CoreError::Conflict(
                "the voice conversation does not speak its replies, so no sound can be played now"
                    .to_owned(),
            ));
        }
        SpeakRoute::Standalone => speaker.handle(shared).await?,
    };
    // It speaks while it listens and has nothing else to do; otherwise the request
    // is refused, not queued.
    if !matches!(
        handle.phase(),
        Phase::Active {
            turn: TurnState::Listening
        }
    ) {
        return Err(CoreError::Busy);
    }
    let text = fetch.await?;
    if text.trim().is_empty() {
        return Err(CoreError::InvalidInput(
            "there is nothing to speak".to_owned(),
        ));
    }
    let sentences = sentence_count(&text);
    let taken = handle
        .say(text)
        .await
        .map_err(|_| CoreError::Conflict("the speech output has stopped".to_owned()))?;
    if taken {
        Ok(SpeakAccepted { sentences })
    } else {
        // The phase changed between the look and the text: refused, not queued.
        Err(CoreError::Busy)
    }
}

fn device_view(info: &DeviceInfo, selected: Option<&audio_io::DeviceRef>) -> AudioDeviceView {
    AudioDeviceView {
        id: info.id.0.clone(),
        name: info.name.clone(),
        is_default: info.is_default,
        is_selected: selected.is_some_and(|s| s.id == info.id),
        sample_rate: info.format.sample_rate,
        channels: u32::from(info.format.channels),
    }
}

fn audio_registry(shared: &Shared) -> CoreResult<Arc<DeviceRegistry>> {
    shared.engines.audio().map(Arc::clone).map_err(|reason| {
        CoreError::unavailable(
            Some(Feature::Speech),
            format!("audio devices are not available: {reason}"),
        )
    })
}

/// The microphones and speakers, and which ones the learner picked.
pub(crate) async fn audio_devices(shared: &Shared) -> CoreResult<AudioDevices> {
    let registry = audio_registry(shared)?;
    tokio::task::spawn_blocking(move || {
        let list = |direction| -> CoreResult<Vec<AudioDeviceView>> {
            let remembered = registry.remembered(direction);
            registry
                .list(direction)
                .map(|devices| {
                    devices
                        .iter()
                        .map(|d| device_view(d, remembered.as_ref()))
                        .collect()
                })
                .map_err(|error| {
                    CoreError::unavailable(
                        Some(Feature::Speech),
                        format!("the audio devices could not be listed: {error}"),
                    )
                })
        };
        Ok(AudioDevices {
            inputs: list(Direction::Input)?,
            outputs: list(Direction::Output)?,
        })
    })
    .await
    .map_err(|_| CoreError::Internal("listing the audio devices did not finish".to_owned()))?
}

/// Records for the length of the test and reports what the microphone heard.
pub(crate) async fn audio_test(
    shared: &Arc<Shared>,
    request: AudioTestRequest,
) -> CoreResult<AudioTestReport> {
    if !(MIN_TEST_MS..=MAX_TEST_MS).contains(&request.duration_ms) {
        return Err(CoreError::InvalidInput(format!(
            "the test lasts between {MIN_TEST_MS} and {MAX_TEST_MS} ms"
        )));
    }
    let registry = audio_registry(shared)?;
    let before = (
        registry.remembered(Direction::Input),
        registry.remembered(Direction::Output),
    );
    let selection = {
        let registry = Arc::clone(&registry);
        let input = request.input_id.clone();
        let output = request.output_id.clone();
        tokio::task::spawn_blocking(move || -> Result<(), audio_io::DeviceError> {
            if let Some(id) = input {
                registry.select(Direction::Input, Some(&DeviceId(id)))?;
            }
            if let Some(id) = output {
                registry.select(Direction::Output, Some(&DeviceId(id)))?;
            }
            Ok(())
        })
        .await
        .map_err(|_| CoreError::Internal("choosing the devices did not finish".to_owned()))?
    };
    if let Err(error) = selection {
        return Err(CoreError::InvalidInput(error.to_string()));
    }
    let outcome = run_test(shared, &registry, &request).await;
    // A device chosen only for this test is not kept unless the learner said so.
    if !request.remember || outcome.is_err() {
        let registry = Arc::clone(&registry);
        let restored = tokio::task::spawn_blocking(move || {
            let _ = registry.select(Direction::Input, before.0.as_ref().map(|d| &d.id));
            let _ = registry.select(Direction::Output, before.1.as_ref().map(|d| &d.id));
        })
        .await;
        if restored.is_err() {
            tracing::warn!("restoring the device choice did not finish");
        }
    }
    match &outcome {
        Ok(report) if report.silent => shared.set_engine(engine_view(
            EngineId::AudioInput,
            EngineState::Failed,
            "the microphone test heard almost nothing; it may be muted or the wrong one",
            None,
        )),
        Ok(_) => shared.set_engine(engine_view(
            EngineId::AudioInput,
            EngineState::Ready,
            "the microphone test heard the input",
            None,
        )),
        Err(error) => shared.set_engine(engine_view(
            EngineId::AudioInput,
            EngineState::Failed,
            &error.to_string(),
            None,
        )),
    }
    outcome
}

async fn run_test(
    shared: &Arc<Shared>,
    registry: &Arc<DeviceRegistry>,
    request: &AudioTestRequest,
) -> CoreResult<AudioTestReport> {
    let recorded = Arc::new(Mutex::new(Vec::<f32>::new()));
    let level = crate::voice::LevelMeter::default();
    let sink: FrameSink = {
        let recorded = Arc::clone(&recorded);
        let level = level.clone();
        Box::new(move |frame, route| {
            if route == Route::Dropped {
                return;
            }
            level.record(frame);
            let mut samples = lock(&recorded);
            if samples.len() + frame.len() <= MAX_TEST_SAMPLES {
                samples.extend_from_slice(frame);
            }
        })
    };
    let session = {
        let registry = Arc::clone(registry);
        tokio::task::spawn_blocking(move || {
            AudioSession::start(registry, SessionConfig::default(), sink)
        })
        .await
        .map_err(|_| CoreError::Internal("opening the audio devices did not finish".to_owned()))?
        .map_err(|error| {
            CoreError::unavailable(
                Some(Feature::Speech),
                format!("the audio devices could not be opened: {error}"),
            )
        })?
    };
    let input_device = {
        let registry = Arc::clone(registry);
        tokio::task::spawn_blocking(move || {
            registry
                .resolve(Direction::Input)
                .map(|resolved| resolved.info.name)
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    };

    // Record, and show the level while it runs: at most ten events a second.
    let started = tokio::time::Instant::now();
    let length = Duration::from_millis(u64::from(request.duration_ms));
    // The first tick is one period away, so no window of a second holds eleven.
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + LEVEL_EVERY, LEVEL_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while started.elapsed() < length {
        tick.tick().await;
        let value = level.take();
        shared
            .bus
            .publish(|seq| ServerEvent::MicLevel { seq, level: value });
    }

    let samples = lock(&recorded).clone();
    let mut played_back = false;
    let mut output_device = None;
    if request.play_back && !samples.is_empty() {
        let playback = session.playback();
        let chunk = PcmChunk {
            samples: samples.clone(),
            sample_rate: speech::SPEECH_SAMPLE_RATE,
        };
        if playback.enqueue(&chunk).is_ok() {
            playback.finish_turn();
            let deadline = tokio::time::Instant::now() + length + Duration::from_secs(3);
            while playback.is_active() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            played_back = true;
            let registry = Arc::clone(registry);
            output_device = tokio::task::spawn_blocking(move || {
                registry
                    .resolve(Direction::Output)
                    .ok()
                    .map(|resolved| resolved.info.name)
            })
            .await
            .ok()
            .flatten();
        }
    }
    let stopped = tokio::task::spawn_blocking(move || session.stop()).await;
    if stopped.is_err() {
        tracing::warn!("closing the audio devices did not finish");
    }

    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, s| peak.max(s.abs()))
        .min(1.0);
    let rms = if samples.is_empty() {
        0.0
    } else {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    };
    Ok(AudioTestReport {
        input_device,
        output_device,
        recorded_ms: u32::try_from(samples.len() / 16).unwrap_or(u32::MAX),
        peak,
        rms,
        silent: peak < SILENT_PEAK,
        clipped: samples.iter().any(|s| s.abs() >= 0.999),
        played_back,
    })
}
