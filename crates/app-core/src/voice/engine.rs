//! The voice loop itself: wiring, the orchestrator task and the handle.
//!
//! See the [module documentation](super) for the threads, the queues and the
//! rules. This file holds the parts that connect them.

use std::collections::VecDeque;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use audio_io::{
    AudioEvent, AudioSession, AudioStream, DeviceRegistry, Direction, PushToTalk, Route,
    SessionConfig, SessionStats, playback_queue,
};
use llm_client::{ChatMessage, LlmClient, Role, TextRequest};
use speech::{
    CancelFlag, SttEngine, SttError, SttEvent, SttJob, SttSubmitError, SttWorker, SttWorkerStats,
    TtsEngine, TtsError, TtsWorker, TtsWorkerStats, UtteranceSegmenter, Vad,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle as TaskHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    Channel, EngineFault, Event, FALLBACK_LINE, InputMode as AnalysisInput, Phase, TurnState,
    bounded_history, reply_limits, system_prompt, user_message,
};

use super::clock::LoopClock;
use super::config::{Scenario, VoiceConfig};
use super::error::{VoiceError, VoiceResult};
use super::event::{EventSink, StopCause, TurnOutcome, VoiceEvent};
use super::latency::{LatencySummary, Stamps, TurnLatency};
use super::listen::{FrameMsg, ListenState, frame_sink, spawn_listener};
use super::msg::{Command, Counters, Inbound, Inbox, ListenEvent, TtsLifecycle, UtteranceMsg};
use super::phase::PhaseCell;
use super::port::PlaybackPort;
use super::record::{RecordMsg, RecordSummary, Recorder, Recording, close_session};
use super::speaker::Speaker;
use super::turn::{Plan, SpeechOut, TurnEnv, TurnReport, TurnRun, run as run_turn};

/// Builds the recogniser on the STT worker's thread, so a slow model load does
/// not hold the caller.
pub type SttLoad = Box<dyn FnOnce() -> Result<Box<dyn SttEngine>, SttError> + Send>;
/// Builds the synthesiser on the TTS worker's thread.
pub type TtsLoad = Box<dyn FnOnce() -> Result<Box<dyn TtsEngine>, TtsError> + Send>;

/// The audio devices the loop opens.
#[derive(Clone)]
pub enum AudioMode {
    /// No devices. Input is typed or fed with [`VoiceHandle::feed_audio`], and a
    /// reply is shown and not played.
    None,
    /// Only the output device, for typed input with spoken replies. The
    /// microphone is not opened.
    PlaybackOnly(Arc<DeviceRegistry>),
    /// Microphone and speakers, with the half-duplex gate between them.
    Full(Arc<DeviceRegistry>),
}

/// What listening needs.
pub struct ListenParts {
    pub vad: Box<dyn Vad>,
    pub stt: SttLoad,
}

/// Everything the loop is built from. All of it is a trait object or a loader,
/// so the loop never names a vendor.
pub struct VoiceParts {
    pub llm: Arc<dyn LlmClient>,
    pub clock: Arc<dyn LoopClock>,
    pub scenario: Scenario,
    pub config: VoiceConfig,
    pub audio: AudioMode,
    /// A VAD and a recogniser. Without them only typed input works.
    pub listen: Option<ListenParts>,
    /// A synthesiser. Without it replies are shown and not spoken.
    pub tts: Option<TtsLoad>,
    /// Where to store the session and its analysis. `None` stores nothing.
    pub recording: Option<Recording>,
}

/// Counters of the whole run, including everything that was dropped on purpose.
#[derive(Debug, Clone, Default)]
pub struct VoiceStats {
    /// Frames lost because the listener was behind.
    pub frames_dropped: u64,
    /// Messages lost because the orchestrator's inbox was full.
    pub inbox_dropped: u64,
    /// Utterances and typed messages that could not wait for their turn.
    pub utterances_dropped: u64,
    pub notes_dropped: u64,
    /// Tutor audio that arrived after its turn was over and was not played.
    pub stale_audio: u64,
    pub recorder_dropped: u64,
    pub ptt_dropped: u64,
    pub audio: Option<SessionStats>,
    pub stt: Option<SttWorkerStats>,
    pub tts: Option<TtsWorkerStats>,
}

/// What a finished session leaves behind.
#[derive(Debug, Clone)]
pub struct VoiceSummary {
    pub turns_completed: u32,
    pub latencies: Vec<TurnLatency>,
    pub latency: LatencySummary,
    pub stats: VoiceStats,
    pub recording: Option<RecordSummary>,
}

/// The things that must be stopped, in the order `finish` stops them.
struct Resources {
    session: Option<AudioSession>,
    output_stream: Option<Box<dyn AudioStream>>,
    stt: Option<Arc<SttWorker>>,
    tts: Option<Arc<TtsWorker>>,
    listener: Option<JoinHandle<()>>,
}

struct Shared {
    inbox: Inbox,
    events: EventSink,
    cell: Arc<PhaseCell>,
    counters: Arc<Counters>,
    config: Arc<VoiceConfig>,
    session_token: CancellationToken,
    listener_cancel: CancelFlag,
    push_to_talk: Option<PushToTalk>,
    frames: Option<SyncSender<FrameMsg>>,
    resources: Mutex<Option<Resources>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A running voice session. Dropping it without calling [`finish`](Self::finish)
/// cancels the session; the workers stop when the last handle is gone.
pub struct VoiceLoop {
    handle: VoiceHandle,
    task: Option<TaskHandle<Output>>,
}

struct Output {
    latencies: Vec<TurnLatency>,
    recording: Option<RecordSummary>,
}

/// A cheap handle for the program around the loop: commands in, events out.
#[derive(Clone)]
pub struct VoiceHandle {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for VoiceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceHandle").finish_non_exhaustive()
    }
}

impl VoiceHandle {
    /// Events from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<VoiceEvent> {
        self.shared.events.subscribe()
    }

    pub fn phase(&self) -> Phase {
        self.shared.cell.phase()
    }

    pub fn turns_completed(&self) -> u32 {
        self.shared.cell.turns_completed()
    }

    async fn command(&self, command: Command) -> VoiceResult<()> {
        if self.shared.inbox.send(Inbound::Command(command)).await {
            Ok(())
        } else {
            Err(VoiceError::Stopped)
        }
    }

    /// The tutor speaks first. Call it once, before the learner's first turn.
    pub async fn open(&self) -> VoiceResult<()> {
        self.command(Command::Open).await
    }

    /// Sends a typed message as the learner's turn.
    pub async fn send_text(&self, text: impl Into<String>) -> VoiceResult<()> {
        self.command(Command::Text(text.into())).await
    }

    /// Stops the tutor, whether it is thinking or speaking, and goes back to listening.
    pub async fn stop_speaking(&self) -> VoiceResult<()> {
        self.command(Command::Stop).await
    }

    pub async fn pause(&self) -> VoiceResult<()> {
        self.command(Command::Pause).await
    }

    /// Leaves `Paused`, or `ProviderUnavailable` when the provider is back.
    pub async fn resume(&self) -> VoiceResult<()> {
        self.command(Command::Resume).await
    }

    /// The learner pressed or released push-to-talk. Without a microphone this
    /// does nothing.
    pub fn push_to_talk(&self, pressed: bool) {
        if let Some(ptt) = &self.shared.push_to_talk {
            ptt.set(pressed);
        }
    }

    /// Feeds 16 kHz mono audio to the listener as if it came from the
    /// microphone, bypassing the microphone gate. The audio is cut into frames
    /// and a short last frame is padded with silence. When the listener's queue
    /// is full this waits one frame length and tries again: the caller sets the
    /// pace, so real-time pacing is the caller's choice.
    pub async fn feed_audio(&self, samples: &[f32]) -> VoiceResult<()> {
        let Some(frames) = &self.shared.frames else {
            return Err(VoiceError::Missing("a VAD and a recogniser to feed audio"));
        };
        let len = self.shared.config.frame_len;
        let wait = Duration::from_micros(len as u64 * 1_000_000 / 16_000);
        for chunk in samples.chunks(len) {
            let mut frame = chunk.to_vec();
            frame.resize(len, 0.0);
            let mut message = FrameMsg {
                frame,
                route: Route::Vad,
            };
            loop {
                match frames.try_send(message) {
                    Ok(()) => break,
                    Err(TrySendError::Disconnected(_)) => return Err(VoiceError::Stopped),
                    Err(TrySendError::Full(back)) => {
                        message = back;
                        tokio::time::sleep(wait).await;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> VoiceStats {
        let c = &self.shared.counters;
        let mut stats = VoiceStats {
            frames_dropped: Counters::get(&c.frames_dropped),
            inbox_dropped: Counters::get(&c.inbox_dropped),
            utterances_dropped: Counters::get(&c.utterances_dropped),
            notes_dropped: Counters::get(&c.notes_dropped),
            stale_audio: Counters::get(&c.stale_audio),
            recorder_dropped: Counters::get(&c.recorder_dropped),
            ptt_dropped: Counters::get(&c.ptt_dropped),
            ..VoiceStats::default()
        };
        if let Some(resources) = lock(&self.shared.resources).as_ref() {
            stats.audio = resources.session.as_ref().map(AudioSession::stats);
            stats.stt = resources.stt.as_ref().map(|w| w.stats());
            stats.tts = resources.tts.as_ref().map(|w| w.stats());
        }
        stats
    }
}

impl VoiceLoop {
    pub fn handle(&self) -> VoiceHandle {
        self.handle.clone()
    }

    /// Wires everything, starts the workers and waits until the models have
    /// loaded (at most `VoiceConfig::model_load_timeout`). Nothing is left
    /// running when this fails.
    pub async fn start(parts: VoiceParts) -> VoiceResult<Self> {
        let VoiceParts {
            llm,
            clock,
            scenario,
            config,
            audio,
            listen,
            tts,
            recording,
        } = parts;
        config.validate()?;
        match (&audio, &listen, &tts) {
            (AudioMode::Full(_), None, _) => {
                return Err(VoiceError::Missing(
                    "a VAD and a recogniser to listen to the microphone",
                ));
            }
            (AudioMode::None, _, Some(_)) => {
                return Err(VoiceError::Missing("an output device to speak"));
            }
            (AudioMode::PlaybackOnly(_), _, None) => {
                return Err(VoiceError::Missing("a synthesiser to play"));
            }
            _ => {}
        }
        let config = Arc::new(config);
        let counters = Arc::new(Counters::default());
        let events = EventSink::new(config.event_capacity);
        let cell = Arc::new(PhaseCell::new(events.clone()));
        let (inbox_tx, inbox_rx) = mpsc::channel(config.inbox);
        let inbox = Inbox::new(inbox_tx, Arc::clone(&counters));
        let session_token = CancellationToken::new();
        let listener_cancel = CancelFlag::new();

        let mut resources = Resources {
            session: None,
            output_stream: None,
            stt: None,
            tts: None,
            listener: None,
        };
        let mut push_to_talk = None;
        let mut frames_tx = None;
        let mut playback: Option<Arc<dyn PlaybackPort>> = None;

        // Listening: the VAD thread and the recogniser.
        if let Some(listen) = listen {
            let (tx, rx) = sync_channel(config.frame_queue);
            let segmenter = UtteranceSegmenter::new(config.endpoint.clone())?;
            let state = ListenState::new(
                listen.vad,
                segmenter,
                Arc::clone(&clock),
                Arc::clone(&counters),
            );
            resources.listener = Some(
                spawn_listener(state, rx, inbox.clone(), listener_cancel.clone()).map_err(
                    |source| speech::WorkerError::Spawn {
                        name: "vad",
                        source,
                    },
                )?,
            );
            let stt_inbox = inbox.clone();
            resources.stt = Some(Arc::new(SttWorker::spawn_with_sink(
                config.stt.clone(),
                listen.stt,
                move |event| {
                    stt_inbox.post(Inbound::Stt(event));
                    Ok(())
                },
            )?));
            frames_tx = Some(tx);
        }

        // Audio devices.
        match &audio {
            AudioMode::None => {}
            AudioMode::Full(registry) => {
                let Some(tx) = frames_tx.clone() else {
                    return Err(VoiceError::Missing("a listener"));
                };
                let session = AudioSession::start(
                    Arc::clone(registry),
                    SessionConfig {
                        frame_len: config.frame_len,
                        ..SessionConfig::default()
                    },
                    frame_sink(tx, Arc::clone(&counters)),
                )?;
                playback = Some(Arc::new(session.playback()));
                push_to_talk = Some(session.push_to_talk());
                resources.session = Some(session);
            }
            AudioMode::PlaybackOnly(registry) => {
                let device = registry.resolve(Direction::Output)?;
                let (queue, mut source) = playback_queue(
                    device.info.format,
                    SessionConfig::default().playback_capacity,
                )?;
                let stream = registry.backend().open_output(
                    &device.info,
                    Box::new(move |out| source.fill(out)),
                    Arc::new(|error| tracing::warn!(%error, "the output stream failed")),
                )?;
                resources.output_stream = Some(stream);
                playback = Some(Arc::new(queue));
            }
        }

        // Speech output.
        let mut speaker = None;
        let mut speech = None;
        if let (Some(load), Some(playback)) = (tts, playback.clone()) {
            let sp = Arc::new(Speaker::new(
                Arc::clone(&playback),
                Arc::clone(&clock),
                inbox.clone(),
                Arc::clone(&counters),
            ));
            let worker = Arc::new(TtsWorker::spawn_with_sink(
                config.tts.clone(),
                load,
                sp.sink(),
            )?);
            resources.tts = Some(Arc::clone(&worker));
            speech = Some(SpeechOut {
                tts: worker,
                playback,
            });
            speaker = Some(sp);
        }

        let recorder = match recording.clone() {
            Some(recording) => Some(
                Recorder::start(
                    recording,
                    &scenario,
                    Arc::clone(&llm),
                    config.recorder_queue,
                    Arc::clone(&counters),
                )
                .await?,
            ),
            None => None,
        };

        let waits_for_stt = resources.stt.is_some();
        let waits_for_tts = resources.tts.is_some();
        let (ready_tx, ready_rx) = oneshot::channel();
        let limits = reply_limits(scenario.context.level, Channel::Voice);
        let system = Arc::new(system_prompt(&scenario.context));
        let shared = Arc::new(Shared {
            inbox: inbox.clone(),
            events: events.clone(),
            cell: Arc::clone(&cell),
            counters: Arc::clone(&counters),
            config: Arc::clone(&config),
            session_token: session_token.clone(),
            listener_cancel,
            push_to_talk,
            frames: frames_tx,
            resources: Mutex::new(Some(resources)),
        });
        let orchestrator = Orchestrator {
            inbox_rx,
            inbox,
            env: TurnEnv {
                llm,
                clock,
                cell,
                events,
                speech,
                config: Arc::clone(&config),
            },
            shared: Arc::clone(&shared),
            speaker,
            system,
            max_tokens: u32::from(limits.max_tokens),
            history: Vec::new(),
            pending_stt: None,
            queued: VecDeque::new(),
            active: None,
            recorder,
            recording,
            latencies: Vec::new(),
            empty_streak: 0,
            listen_failed: false,
            ready: Some(Readiness {
                stt: !waits_for_stt,
                tts: !waits_for_tts,
                tx: ready_tx,
            }),
        };
        let handle = VoiceHandle {
            shared: Arc::clone(&shared),
        };
        let mut voice = Self {
            handle,
            task: Some(tokio::spawn(orchestrator.run())),
        };

        // Without workers there is nothing to wait for and the orchestrator
        // answers at once.
        let loaded = tokio::time::timeout(config.model_load_timeout, ready_rx).await;
        let failure = match loaded {
            Ok(Ok(Ok(()))) => None,
            Ok(Ok(Err(error))) => Some(error),
            Ok(Err(_)) => Some(VoiceError::Stopped),
            Err(_) => Some(VoiceError::EngineLoadTimeout {
                engine: "speech",
                seconds: config.model_load_timeout.as_secs(),
            }),
        };
        if let Some(error) = failure {
            voice.shutdown(true).await;
            return Err(error);
        }
        Ok(voice)
    }

    /// Ends the session. Stops the tutor and flushes playback, stores what is
    /// queued, runs the last analysis (unless `cancelled`), joins the workers
    /// and closes the streams. Returns the latency of every turn and the counters.
    pub async fn finish(mut self, cancelled: bool) -> VoiceSummary {
        self.shutdown(cancelled).await
    }

    async fn shutdown(&mut self, cancelled: bool) -> VoiceSummary {
        let shared = Arc::clone(&self.handle.shared);
        let (done_tx, done_rx) = oneshot::channel();
        let accepted = shared
            .inbox
            .send(Inbound::Command(Command::Finish {
                cancelled,
                done: done_tx,
            }))
            .await;
        if accepted {
            let _ = done_rx.await;
        } else {
            shared.session_token.cancel();
        }
        let output = match self.task.take() {
            Some(task) => task.await.ok(),
            None => None,
        };
        let stats = self.handle.stats();
        let resources = lock(&shared.resources).take();
        shared.listener_cancel.cancel();
        if let Some(resources) = resources {
            // Joining threads can take as long as an engine call, so it never
            // runs on a runtime worker.
            let _ = tokio::task::spawn_blocking(move || stop_resources(resources)).await;
        }
        shared.events.publish(VoiceEvent::Closed);
        let (latencies, recording) = match output {
            Some(Output {
                latencies,
                recording,
            }) => (latencies, recording),
            None => (Vec::new(), None),
        };
        VoiceSummary {
            turns_completed: shared.cell.turns_completed(),
            latency: LatencySummary::of(&latencies),
            latencies,
            stats,
            recording,
        }
    }
}

impl Drop for VoiceLoop {
    fn drop(&mut self) {
        // A session that was not finished still has to stop its turn.
        self.handle.shared.session_token.cancel();
    }
}

fn stop_resources(mut resources: Resources) {
    // The session first: it joins the capture worker, which owns the frame sink.
    if let Some(session) = resources.session.take() {
        let report = session.stop();
        if report.worker_panics > 0 {
            tracing::error!(panics = report.worker_panics, "an audio worker panicked");
        }
    }
    drop(resources.output_stream.take());
    if let Some(listener) = resources.listener.take()
        && listener.join().is_err()
    {
        tracing::error!("the listener thread panicked");
    }
    // The last handle to a worker stops it and joins its thread.
    drop(resources.stt.take());
    drop(resources.tts.take());
}

struct Readiness {
    stt: bool,
    tts: bool,
    tx: oneshot::Sender<VoiceResult<()>>,
}

struct PendingStt {
    epoch: u64,
    turn: u64,
    stamps: Stamps,
    flag: CancelFlag,
    deadline: Instant,
    speech_ms: i64,
}

struct Active {
    epoch: u64,
    token: CancellationToken,
    task: TaskHandle<()>,
    learner_text: String,
    voice: bool,
    speech_ms: Option<i64>,
    opening: bool,
    canned: bool,
}

enum Queued {
    Utterance(UtteranceMsg),
    Text(String),
}

struct Orchestrator {
    inbox_rx: mpsc::Receiver<Inbound>,
    inbox: Inbox,
    env: TurnEnv,
    shared: Arc<Shared>,
    speaker: Option<Arc<Speaker>>,
    system: Arc<String>,
    max_tokens: u32,
    /// The messages the model sees, oldest first. Learner messages are stored
    /// without notes, as in the text chat. Bounded to twice the window.
    history: Vec<ChatMessage>,
    pending_stt: Option<PendingStt>,
    queued: VecDeque<Queued>,
    active: Option<Active>,
    recorder: Option<Recorder>,
    recording: Option<Recording>,
    latencies: Vec<TurnLatency>,
    /// Empty replies in a row. The first gets the authored line; the second is
    /// treated as a provider failure.
    empty_streak: u32,
    listen_failed: bool,
    ready: Option<Readiness>,
}

/// How often the orchestrator looks for audio device events.
const AUDIO_POLL: Duration = Duration::from_millis(250);

/// The messages for one request: the bounded history with consecutive messages
/// of one role joined, the first one a learner message (a provider rejects a
/// conversation that opens with the assistant), then the current message.
pub(crate) fn build_messages(history: &[ChatMessage], current: ChatMessage) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::new();
    for message in bounded_history(history) {
        match out.last_mut() {
            Some(last) if last.role == message.role => {
                last.content.push('\n');
                last.content.push_str(&message.content);
            }
            _ => out.push(message),
        }
    }
    while out.first().is_some_and(|m| m.role == Role::Assistant) {
        out.remove(0);
    }
    match out.last_mut() {
        Some(last) if last.role == Role::User => {
            last.content.push('\n');
            last.content.push_str(&current.content);
        }
        _ => out.push(current),
    }
    out
}

impl Orchestrator {
    fn events(&self) -> &EventSink {
        &self.env.events
    }

    fn cell(&self) -> &PhaseCell {
        &self.env.cell
    }

    fn playback(&self) -> Option<&Arc<dyn PlaybackPort>> {
        self.env.speech.as_ref().map(|s| &s.playback)
    }

    async fn run(mut self) -> Output {
        self.check_ready();
        let mut audio_tick = tokio::time::interval(AUDIO_POLL);
        audio_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let token = self.shared.session_token.clone();
        let mut cancelled = false;
        let mut finish_done: Option<oneshot::Sender<()>> = None;
        loop {
            let stt_deadline = self.pending_stt.as_ref().map(|p| p.deadline);
            tokio::select! {
                biased;
                () = token.cancelled() => {
                    cancelled = true;
                    break;
                }
                message = self.inbox_rx.recv() => {
                    let Some(message) = message else { break };
                    match message {
                        Inbound::Command(Command::Finish { cancelled: c, done }) => {
                            cancelled = c;
                            finish_done = Some(done);
                            break;
                        }
                        other => self.handle(other).await,
                    }
                }
                () = async {
                    match stt_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                } => self.stt_timed_out().await,
                _ = audio_tick.tick(), if self.has_session() => self.poll_audio().await,
            }
        }
        let output = self.close(cancelled).await;
        if let Some(done) = finish_done {
            let _ = done.send(());
        }
        output
    }

    fn has_session(&self) -> bool {
        lock(&self.shared.resources)
            .as_ref()
            .is_some_and(|r| r.session.is_some())
    }

    fn check_ready(&mut self) {
        let Some(ready) = &self.ready else {
            return;
        };
        if ready.stt && ready.tts {
            if let Some(ready) = self.ready.take() {
                let _ = ready.tx.send(Ok(()));
            }
        }
    }

    fn fail_ready(&mut self, engine: &'static str, reason: String) {
        if let Some(ready) = self.ready.take() {
            let _ = ready
                .tx
                .send(Err(VoiceError::EngineUnavailable { engine, reason }));
        }
    }

    async fn handle(&mut self, message: Inbound) {
        match message {
            Inbound::Listen(event) => self.on_listen(event).await,
            Inbound::Stt(event) => self.on_stt(event).await,
            Inbound::Tts(life) => match life {
                TtsLifecycle::Ready => {
                    if let Some(ready) = self.ready.as_mut() {
                        ready.tts = true;
                    }
                    self.check_ready();
                }
                TtsLifecycle::LoadFailed(error) => {
                    self.fail_ready("speech synthesis", error.to_string());
                }
                TtsLifecycle::Stopped => {}
            },
            Inbound::Command(command) => self.on_command(command).await,
            Inbound::Turn(report) => self.on_report(report).await,
        }
    }

    // ---- listening -------------------------------------------------------

    async fn on_listen(&mut self, event: ListenEvent) {
        match event {
            ListenEvent::SpeechStarted => {
                self.events().publish(VoiceEvent::SpeechStarted);
                // The learner speaks while the tutor is still thinking: the
                // reply they are about to be given no longer fits.
                if matches!(
                    self.cell().phase(),
                    Phase::Active {
                        turn: TurnState::Thinking
                    }
                ) {
                    self.abort_turn(Some(StopCause::Speech));
                }
            }
            ListenEvent::PushToTalkStarted => {
                self.abort_turn(Some(StopCause::PushToTalk));
            }
            ListenEvent::Utterance(utterance) => self.on_utterance(utterance).await,
            ListenEvent::Failed(message) => {
                // A failing VAD fails on every frame; report it once.
                if !std::mem::replace(&mut self.listen_failed, true) {
                    self.abort_turn(None);
                    self.engine_error(EngineFault::Microphone, message, false);
                }
            }
        }
    }

    async fn on_utterance(&mut self, utterance: UtteranceMsg) {
        match self.cell().phase() {
            Phase::Active {
                turn: TurnState::Listening,
            } => self.begin_voice_turn(utterance),
            Phase::Active { .. } => self.enqueue(Queued::Utterance(utterance)),
            Phase::Paused
            | Phase::ProviderUnavailable
            | Phase::EngineError { .. }
            | Phase::Ended { .. } => Counters::bump(&self.shared.counters.utterances_dropped),
        }
    }

    fn enqueue(&mut self, item: Queued) {
        if self.queued.len() < self.shared.config.utterance_backlog {
            self.queued.push_back(item);
        } else {
            Counters::bump(&self.shared.counters.utterances_dropped);
        }
    }

    fn begin_voice_turn(&mut self, utterance: UtteranceMsg) {
        let (epoch, turn) = self.cell().begin_turn();
        let _ = self.cell().apply(Event::UtteranceEnded);
        let stamps = Stamps {
            last_speech: Some(utterance.last_speech),
            utterance_ended: Some(utterance.ended),
            ..Stamps::default()
        };
        let speech_ms = i64::try_from(utterance.samples.len() / 16).unwrap_or(i64::MAX);
        let flag = CancelFlag::new();
        let submitted = match &self.shared_stt() {
            Some(stt) => stt.submit(SttJob {
                utterance_id: epoch,
                samples: utterance.samples,
                cancel: flag.clone(),
            }),
            None => {
                self.engine_error(
                    EngineFault::SpeechRecognition,
                    "no speech recognition engine is running".to_owned(),
                    true,
                );
                return;
            }
        };
        match submitted {
            Ok(()) => {
                self.pending_stt = Some(PendingStt {
                    epoch,
                    turn,
                    stamps,
                    flag,
                    deadline: Instant::now() + self.shared.config.stt_timeout,
                    speech_ms,
                });
            }
            Err(SttSubmitError::Full(_)) => self.engine_error(
                EngineFault::SpeechRecognition,
                "speech recognition is still busy with earlier speech".to_owned(),
                true,
            ),
            Err(SttSubmitError::Stopped(_)) => self.engine_error(
                EngineFault::SpeechRecognition,
                "speech recognition has stopped".to_owned(),
                true,
            ),
        }
    }

    fn shared_stt(&self) -> Option<Arc<SttWorker>> {
        lock(&self.shared.resources)
            .as_ref()
            .and_then(|r| r.stt.clone())
    }

    async fn on_stt(&mut self, event: SttEvent) {
        match event {
            SttEvent::Ready { .. } => {
                if let Some(ready) = self.ready.as_mut() {
                    ready.stt = true;
                }
                self.check_ready();
            }
            SttEvent::LoadFailed { error } => {
                self.fail_ready("speech recognition", error.to_string());
            }
            SttEvent::Transcript {
                utterance_id,
                transcript,
                ..
            } => {
                let Some(pending) = self.pending_stt.take_if(|p| p.epoch == utterance_id) else {
                    return;
                };
                self.on_transcript(pending, transcript.text);
            }
            SttEvent::Cancelled { utterance_id } => {
                // Cancelled by the loop (stop, push-to-talk, timeout), which has
                // already dealt with the turn.
                let _ = self.pending_stt.take_if(|p| p.epoch == utterance_id);
            }
            SttEvent::Failed {
                utterance_id,
                error,
            } => {
                if self
                    .pending_stt
                    .take_if(|p| p.epoch == utterance_id)
                    .is_some()
                {
                    self.engine_error(EngineFault::SpeechRecognition, error.to_string(), true);
                }
            }
            SttEvent::Stopped => {}
        }
    }

    async fn stt_timed_out(&mut self) {
        let Some(pending) = self.pending_stt.take() else {
            return;
        };
        pending.flag.cancel();
        if let Some(stt) = self.shared_stt() {
            stt.cancel_current();
        }
        self.engine_error(
            EngineFault::SpeechRecognition,
            "speech recognition took too long".to_owned(),
            true,
        );
    }

    fn on_transcript(&mut self, pending: PendingStt, text: String) {
        let mut stamps = pending.stamps;
        stamps.transcript_ready = Some(self.env.clock.now());
        let _ = self.cell().apply(Event::TranscriptReady);
        let text = text.trim().to_owned();
        self.events().publish(VoiceEvent::Heard {
            turn: pending.turn,
            text: text.clone(),
        });
        self.start_turn(
            pending.epoch,
            pending.turn,
            stamps,
            text,
            true,
            Some(pending.speech_ms),
            false,
        );
    }

    // ---- commands --------------------------------------------------------

    async fn on_command(&mut self, command: Command) {
        match command {
            Command::Text(text) => {
                let text = text.trim().to_owned();
                if text.is_empty() {
                    return;
                }
                match self.cell().phase() {
                    Phase::Active {
                        turn: TurnState::Listening,
                    } => self.begin_text_turn(text),
                    Phase::Active { .. } => self.enqueue(Queued::Text(text)),
                    _ => Counters::bump(&self.shared.counters.utterances_dropped),
                }
            }
            Command::Open => {
                if matches!(
                    self.cell().phase(),
                    Phase::Active {
                        turn: TurnState::Listening
                    }
                ) {
                    self.begin_opening();
                }
            }
            Command::Stop => self.abort_turn(Some(StopCause::Command)),
            Command::Pause => {
                self.abort_turn(None);
                self.queued.clear();
                let _ = self.cell().apply(Event::Pause);
            }
            Command::Resume => {
                let event = match self.cell().phase() {
                    Phase::ProviderUnavailable => Event::ProviderRecovered,
                    _ => Event::Resume,
                };
                self.empty_streak = 0;
                if self.cell().apply(event).is_ok() {
                    self.drain_queue();
                }
            }
            // Handled by the run loop, which ends the session.
            Command::Finish { done, .. } => {
                let _ = done.send(());
            }
        }
    }

    fn begin_text_turn(&mut self, text: String) {
        let (epoch, turn) = self.cell().begin_turn();
        let _ = self.cell().apply(Event::TextSent);
        let stamps = Stamps {
            transcript_ready: Some(self.env.clock.now()),
            ..Stamps::default()
        };
        self.events().publish(VoiceEvent::Heard {
            turn,
            text: text.clone(),
        });
        self.start_turn(epoch, turn, stamps, text, false, None, false);
    }

    fn begin_opening(&mut self) {
        let (epoch, turn) = self.cell().begin_turn();
        let _ = self.cell().apply(Event::TextSent);
        let stamps = Stamps {
            transcript_ready: Some(self.env.clock.now()),
            ..Stamps::default()
        };
        let instruction = self.shared.config.opening_instruction.clone();
        self.start_turn(epoch, turn, stamps, instruction, false, None, true);
    }

    // ---- turns -----------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn start_turn(
        &mut self,
        epoch: u64,
        turn: u64,
        stamps: Stamps,
        learner_text: String,
        voice: bool,
        speech_ms: Option<i64>,
        opening: bool,
    ) {
        let canned = learner_text.is_empty() && !opening;
        let plan = if canned {
            Plan::Canned(FALLBACK_LINE.to_owned())
        } else {
            let notes = self
                .recorder
                .as_ref()
                .map(Recorder::notes)
                .unwrap_or_default();
            let (current, stored) = if opening {
                (
                    ChatMessage::user(learner_text.clone()),
                    ChatMessage::user(learner_text.clone()),
                )
            } else {
                (
                    ChatMessage::user(user_message(&learner_text, &notes, &[])),
                    ChatMessage::user(user_message(&learner_text, &[], &[])),
                )
            };
            let messages = build_messages(&self.history, current);
            self.push_history(stored);
            Plan::Model(
                TextRequest::new((*self.system).clone(), messages, self.max_tokens)
                    .with_temperature(0.7),
            )
        };
        let token = self.shared.session_token.child_token();
        let notes = match &self.speaker {
            Some(speaker) => speaker.begin_turn(epoch),
            None => mpsc::channel(1).1,
        };
        let run = TurnRun {
            epoch,
            turn,
            plan,
            stamps,
            allow_fallback: self.empty_streak == 0,
            token: token.clone(),
            notes,
        };
        let env = self.env.clone();
        let inbox = self.inbox.clone();
        let task = tokio::spawn(async move {
            let report = run_turn(env, run).await;
            inbox.send(Inbound::Turn(report)).await;
        });
        self.active = Some(Active {
            epoch,
            token,
            task,
            learner_text,
            voice,
            speech_ms,
            opening,
            canned,
        });
    }

    fn push_history(&mut self, message: ChatMessage) {
        self.history.push(message);
        let keep = tutor_engine::HISTORY_MESSAGES * 2;
        if self.history.len() > keep {
            let excess = self.history.len() - keep;
            self.history.drain(..excess);
        }
    }

    async fn on_report(&mut self, report: TurnReport) {
        let current = self.cell().epoch();
        if report.epoch != current {
            // A turn that was cancelled earlier. Its text so far still belongs
            // to the conversation.
            if report.outcome == TurnOutcome::Stopped && !report.reply.is_empty() {
                self.push_history(ChatMessage::assistant(report.reply));
            }
            return;
        }
        let Some(mut active) = self.active.take_if(|a| a.epoch == report.epoch) else {
            return;
        };
        let _ = (&mut active.task).await;
        if let Some(latency) = report.latency {
            self.latencies.push(latency);
        }
        match report.outcome {
            TurnOutcome::Replied | TurnOutcome::Fallback => {
                if report.outcome == TurnOutcome::Fallback && !active.canned {
                    self.empty_streak += 1;
                } else if report.outcome == TurnOutcome::Replied {
                    self.empty_streak = 0;
                }
                self.push_history(ChatMessage::assistant(report.reply.clone()));
                self.record(&active, &report.reply);
            }
            TurnOutcome::Stopped => {
                if !report.reply.is_empty() {
                    self.push_history(ChatMessage::assistant(report.reply.clone()));
                }
            }
            TurnOutcome::ProviderUnavailable => {
                self.empty_streak = 0;
                let message = report.message.clone().unwrap_or_default();
                self.events()
                    .publish(VoiceEvent::ProviderUnavailable { message });
            }
            TurnOutcome::EngineError => {
                let fault = report.fault.unwrap_or(EngineFault::Playback);
                let message = report.message.clone().unwrap_or_default();
                self.abort_audio();
                self.engine_error(fault, message, true);
            }
        }
        self.events().publish(VoiceEvent::TurnEnded {
            turn: report.turn,
            outcome: report.outcome,
        });
        self.drain_queue();
    }

    fn record(&self, active: &Active, reply: &str) {
        let Some(recorder) = &self.recorder else {
            return;
        };
        if active.opening {
            recorder.push(RecordMsg::Opening {
                tutor_text: reply.to_owned(),
            });
        } else if !active.canned {
            recorder.push(RecordMsg::Turn {
                learner_text: active.learner_text.clone(),
                input: if active.voice {
                    AnalysisInput::Voice
                } else {
                    AnalysisInput::Text
                },
                speech_ms: active.speech_ms,
                tutor_reply: reply.to_owned(),
            });
        }
    }

    /// Starts the next waiting utterance or message once the learner can be heard.
    fn drain_queue(&mut self) {
        while matches!(
            self.cell().phase(),
            Phase::Active {
                turn: TurnState::Listening
            }
        ) {
            match self.queued.pop_front() {
                Some(Queued::Utterance(utterance)) => self.begin_voice_turn(utterance),
                Some(Queued::Text(text)) => self.begin_text_turn(text),
                None => break,
            }
        }
    }

    /// Stops the tutor: everything that belongs to the current turn is
    /// cancelled, the playback queue is flushed and the session goes back to
    /// listening. The order matters: the epoch changes and the speaker stops
    /// letting audio through before the flush, so a sentence that was being
    /// synthesised cannot play afterwards.
    fn abort_turn(&mut self, cause: Option<StopCause>) {
        let busy = self.active.is_some() || self.pending_stt.is_some();
        if !busy {
            return;
        }
        self.cell().invalidate();
        if let Some(speaker) = &self.speaker {
            speaker.end_turn();
        }
        if let Some(active) = self.active.take() {
            active.token.cancel();
        }
        if let Some(pending) = self.pending_stt.take() {
            pending.flag.cancel();
            if let Some(stt) = self.shared_stt() {
                stt.cancel_current();
            }
        }
        if let Some(playback) = self.playback() {
            playback.stop();
        }
        match self.cell().phase() {
            Phase::Active {
                turn: TurnState::Speaking,
            } => {
                let _ = self.cell().apply(Event::StopSpeaking);
            }
            Phase::Active {
                turn: TurnState::Thinking | TurnState::Transcribing,
            } => {
                // The machine has no "never mind" event for a turn that has not
                // started speaking. A pause and a resume return it to listening
                // without counting a finished turn.
                let _ = self.cell().apply(Event::Pause);
                let _ = self.cell().apply(Event::Resume);
            }
            _ => {}
        }
        if let Some(cause) = cause {
            self.events().publish(VoiceEvent::Stopped(cause));
        }
        self.drain_queue();
    }

    /// Stops sound and speech work without touching the session phase.
    fn abort_audio(&self) {
        if let Some(speaker) = &self.speaker {
            speaker.end_turn();
        }
        if let Some(playback) = self.playback() {
            playback.stop();
        }
    }

    /// Reports an engine fault. With `recover` the session returns to
    /// listening at once, so the next turn can try again.
    fn engine_error(&mut self, fault: EngineFault, message: String, recover: bool) {
        let _ = self.cell().apply(Event::EngineFailed(fault));
        self.events()
            .publish(VoiceEvent::EngineError { fault, message });
        if recover {
            let _ = self.cell().apply(Event::EngineRecovered);
        }
    }

    // ---- audio devices ---------------------------------------------------

    async fn poll_audio(&mut self) {
        let mut events = Vec::new();
        if let Some(session) = lock(&self.shared.resources)
            .as_ref()
            .and_then(|r| r.session.as_ref())
        {
            while let Some(event) = session.try_event() {
                events.push(event);
            }
        }
        for event in events {
            match event {
                AudioEvent::DeviceLost { direction, .. } => self.on_device_lost(direction).await,
                AudioEvent::StreamError { direction, message } => {
                    let fault = fault_of(direction);
                    self.engine_error(fault, message, true);
                }
                AudioEvent::ConversionFailed { .. } | AudioEvent::Recovered { .. } => {}
            }
        }
    }

    async fn on_device_lost(&mut self, direction: Direction) {
        let fault = fault_of(direction);
        self.abort_turn(None);
        self.engine_error(
            fault,
            match direction {
                Direction::Input => "the microphone was lost".to_owned(),
                Direction::Output => "the speaker was lost".to_owned(),
            },
            false,
        );
        let shared = Arc::clone(&self.shared);
        let recovered = tokio::task::spawn_blocking(move || {
            lock(&shared.resources)
                .as_mut()
                .and_then(|r| r.session.as_mut())
                .map(|session| session.recover(direction))
        })
        .await;
        match recovered {
            Ok(Some(Ok(_))) => {
                let _ = self.cell().apply(Event::EngineRecovered);
            }
            _ => self.events().publish(VoiceEvent::EngineError {
                fault,
                message: "no other device could be opened".to_owned(),
            }),
        }
    }

    // ---- shutdown --------------------------------------------------------

    async fn close(mut self, cancelled: bool) -> Output {
        let _ = self.cell().apply(if cancelled {
            Event::Cancel
        } else {
            Event::Finish
        });
        self.cell().invalidate();
        if let Some(speaker) = &self.speaker {
            speaker.end_turn();
        }
        if let Some(active) = self.active.take() {
            active.token.cancel();
            // The turn cancels its speech and returns at its next wait.
            let _ = tokio::time::timeout(Duration::from_secs(2), active.task).await;
        }
        if let Some(pending) = self.pending_stt.take() {
            pending.flag.cancel();
        }
        if let Some(playback) = self.playback() {
            playback.stop();
        }
        let recording = match self.recorder.take() {
            Some(recorder) => {
                let session_id = recorder.session_id();
                let cancel = CancellationToken::new();
                let summary = recorder.finish(cancelled, &cancel).await;
                if let Some(recording) = &self.recording
                    && let Err(error) = close_session(recording, session_id, cancelled).await
                {
                    tracing::warn!(%error, "the stored session could not be closed");
                }
                Some(summary)
            }
            None => None,
        };
        Output {
            latencies: std::mem::take(&mut self.latencies),
            recording,
        }
    }
}

fn fault_of(direction: Direction) -> EngineFault {
    match direction {
        Direction::Input => EngineFault::Microphone,
        Direction::Output => EngineFault::Playback,
    }
}
