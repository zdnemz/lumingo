//! One audio session: the streams, the capture worker and the playback queue,
//! started and stopped as a unit.
//!
//! # Threads and ownership
//!
//! * The backend owns the device callback threads. A stream is an owned value;
//!   dropping it ends its threads before `drop` returns.
//! * The capture worker (this module) reads the capture ring and runs the
//!   conversion, framing and gate. It hands frames to the caller's sink.
//! * Workers added with [`AudioSession::spawn_worker`] (VAD, STT, TTS in later
//!   stages) share the session's [`CancelFlag`] and are joined when the session
//!   stops, so nothing outlives a stopped session.
//!
//! # Queues
//!
//! * Capture ring: `capture_buffer` of device audio (2 s by default). When full,
//!   whole callback blocks are dropped and counted.
//! * Playback queue: `playback_capacity` of device audio (30 s by default). When
//!   full, `enqueue` fails with `QueueFull`.
//! * Event channel: `event_capacity` events (32 by default). When full, the new
//!   event is dropped and counted in [`SessionStats::events_dropped`], because the
//!   sender may be a device thread that must never wait.
//!
//! # Start, stop, cancel
//!
//! * [`AudioSession::cancel`] sets the shared flag and flushes playback. Workers
//!   leave their loops on their own. It does not wait.
//! * [`AudioSession::stop`] (and `Drop`) cancels, then joins every worker and
//!   closes every stream.
//! * Stopping speech without ending the session is `playback().stop()`.

use std::io;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use speech::{CancelFlag, PcmChunk, SPEECH_SAMPLE_RATE};

use crate::capture::CapturePath;
use crate::device::{
    AudioBackend, AudioStream, Choice, DeviceError, DeviceInfo, DeviceRegistry, Direction,
    ResolvedDevice, StreamErrorCallback,
};
use crate::format::StreamFormat;
use crate::gate::{GateCounters, GateStats, MicGate, PlaybackActivity, PushToTalk, Route};
use crate::playback::{
    EnqueueOutcome, PlaybackError, PlaybackQueue, PlaybackSource, PlaybackStats, playback_queue,
};
use crate::resample::{CaptureConverter, ResampleError};
use crate::ring::{CaptureConsumer, CaptureCounters, CaptureSnapshot, capture_ring};
use crate::sync::lock;

/// Called on the capture worker for every 16 kHz frame, with the gate's decision.
/// It must not block for long: a slow sink makes the capture ring fill up.
pub type FrameSink = Box<dyn FnMut(&[f32], Route) + Send + 'static>;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Samples per frame handed to the sink, at 16 kHz. 512 is 32 ms.
    pub frame_len: usize,
    pub capture_buffer: Duration,
    pub playback_capacity: Duration,
    /// How long the capture worker sleeps when there is nothing to read.
    pub poll_interval: Duration,
    pub event_capacity: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            frame_len: 512,
            capture_buffer: Duration::from_secs(2),
            playback_capacity: Duration::from_secs(30),
            poll_interval: Duration::from_millis(5),
            event_capacity: 32,
        }
    }
}

/// Things the rest of the program needs to hear about. Delivered through
/// [`AudioSession::try_event`] and [`AudioSession::wait_event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioEvent {
    /// A device went away under a running stream. Call
    /// [`AudioSession::recover`] to continue on another device.
    DeviceLost {
        direction: Direction,
        device: String,
    },
    /// A stream failed for another reason.
    StreamError {
        direction: Direction,
        message: String,
    },
    /// `recover` reopened a stream.
    Recovered {
        direction: Direction,
        device: String,
        choice: Choice,
    },
    /// The capture worker could not convert a block and dropped it.
    ConversionFailed { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Device(#[from] DeviceError),
    #[error(transparent)]
    Playback(#[from] PlaybackError),
    #[error(transparent)]
    Resample(#[from] ResampleError),
    #[error("invalid audio session settings: {0}")]
    Settings(&'static str),
    #[error("could not start a thread: {0}")]
    Spawn(#[from] io::Error),
    #[error("the session was cancelled or stopped")]
    Stopped,
}

/// The producer side of the playback queue as the session exposes it. It keeps
/// working across [`AudioSession::recover`]: the queue behind it is replaced when
/// the output device changes, and audio queued for the old device is dropped.
#[derive(Clone)]
pub struct PlaybackHandle {
    current: Arc<Mutex<PlaybackQueue>>,
    retired: Arc<Mutex<PlaybackStats>>,
}

impl std::fmt::Debug for PlaybackHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackHandle").finish_non_exhaustive()
    }
}

impl PlaybackHandle {
    fn new(queue: PlaybackQueue) -> Self {
        Self {
            current: Arc::new(Mutex::new(queue)),
            retired: Arc::new(Mutex::new(PlaybackStats::default())),
        }
    }

    fn queue(&self) -> PlaybackQueue {
        lock(&self.current).clone()
    }

    fn replace(&self, queue: PlaybackQueue) {
        let old = std::mem::replace(&mut *lock(&self.current), queue);
        old.stop();
        let stats = old.stats();
        let mut retired = lock(&self.retired);
        retired.underruns += stats.underruns;
        retired.samples_played += stats.samples_played;
    }

    /// See [`PlaybackQueue::enqueue`].
    pub fn enqueue(&self, chunk: &PcmChunk) -> Result<EnqueueOutcome, PlaybackError> {
        self.queue().enqueue(chunk)
    }

    /// See [`PlaybackQueue::finish_turn`].
    pub fn finish_turn(&self) {
        self.queue().finish_turn();
    }

    /// See [`PlaybackQueue::stop`].
    pub fn stop(&self) {
        self.queue().stop();
    }

    pub fn is_active(&self) -> bool {
        self.queue().is_active()
    }

    pub fn queued(&self) -> Duration {
        self.queue().queued()
    }

    pub fn format(&self) -> StreamFormat {
        self.queue().format()
    }

    /// Counters over the whole session, including queues replaced by `recover`.
    pub fn stats(&self) -> PlaybackStats {
        let mut stats = self.queue().stats();
        let retired = *lock(&self.retired);
        stats.underruns += retired.underruns;
        stats.samples_played += retired.samples_played;
        stats
    }
}

impl PlaybackActivity for PlaybackHandle {
    fn is_playing(&self) -> bool {
        self.is_active()
    }
}

/// Counters for the whole session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionStats {
    pub capture: CaptureSnapshot,
    pub gate: GateStats,
    pub playback: PlaybackStats,
    pub events_dropped: u64,
}

/// What stopping a session did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopReport {
    /// Worker threads joined, the capture worker included.
    pub workers_joined: usize,
    /// Workers that ended by panicking.
    pub worker_panics: usize,
}

struct Worker {
    name: String,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    fn wake(&self) {
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
    }
}

struct CaptureInput {
    consumer: CaptureConsumer,
    converter: CaptureConverter,
}

type Mailbox = Arc<Mutex<Option<CaptureInput>>>;

struct EventLink {
    tx: SyncSender<AudioEvent>,
    dropped: Arc<AtomicU64>,
}

impl EventLink {
    fn send(&self, event: AudioEvent) {
        if self.tx.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn stream_errors(
        &self,
        direction: Direction,
        name: String,
        flush_on_loss: Option<PlaybackHandle>,
    ) -> StreamErrorCallback {
        let link = Self {
            tx: self.tx.clone(),
            dropped: Arc::clone(&self.dropped),
        };
        Arc::new(move |error| {
            let event = match error {
                DeviceError::Lost { .. } => {
                    // A lost output no longer drains the queue; flush it so the
                    // microphone gate does not stay closed behind audio that
                    // will never play.
                    if let Some(playback) = &flush_on_loss {
                        playback.stop();
                    }
                    AudioEvent::DeviceLost {
                        direction,
                        device: name.clone(),
                    }
                }
                other => AudioEvent::StreamError {
                    direction,
                    message: other.to_string(),
                },
            };
            link.send(event);
        })
    }
}

struct OpenInput {
    stream: Box<dyn AudioStream>,
    input: CaptureInput,
    counters: CaptureCounters,
}

fn open_input(
    backend: &dyn AudioBackend,
    device: &DeviceInfo,
    capture_buffer: Duration,
    on_error: StreamErrorCallback,
) -> Result<OpenInput, SessionError> {
    let samples = device.format.samples_per_second() * capture_buffer.as_millis() as u64 / 1000;
    let capacity = usize::try_from(samples)
        .ok()
        .and_then(NonZeroUsize::new)
        .ok_or(SessionError::Settings("the capture buffer holds no audio"))?;
    let converter = CaptureConverter::new(device.format.sample_rate, device.format.channels)?;
    let (mut producer, consumer, counters) = capture_ring(capacity);
    // The callback only copies into the ring; overflow is counted inside `push`.
    let stream = backend.open_input(
        device,
        Box::new(move |block| {
            let _ = producer.push(block);
        }),
        on_error,
    )?;
    Ok(OpenInput {
        stream,
        input: CaptureInput {
            consumer,
            converter,
        },
        counters,
    })
}

fn open_output(
    backend: &dyn AudioBackend,
    device: &DeviceInfo,
    mut source: PlaybackSource,
    on_error: StreamErrorCallback,
) -> Result<Box<dyn AudioStream>, DeviceError> {
    backend.open_output(device, Box::new(move |out| source.fill(out)), on_error)
}

/// A running session. See the module documentation.
pub struct AudioSession {
    registry: Arc<DeviceRegistry>,
    config: SessionConfig,
    cancel: CancelFlag,
    push_to_talk: PushToTalk,
    playback: PlaybackHandle,
    mailbox: Mailbox,
    events: EventLink,
    events_rx: Receiver<AudioEvent>,
    gate_counters: GateCounters,
    capture_counters: Vec<CaptureCounters>,
    input_stream: Option<Box<dyn AudioStream>>,
    output_stream: Option<Box<dyn AudioStream>>,
    input_device: DeviceInfo,
    output_device: DeviceInfo,
    workers: Vec<Worker>,
    stopped: bool,
}

impl std::fmt::Debug for AudioSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioSession")
            .field("input", &self.input_device.name)
            .field("output", &self.output_device.name)
            .finish_non_exhaustive()
    }
}

impl AudioSession {
    /// Opens the selected (or default) devices and starts the capture worker.
    /// Nothing is left running when this fails.
    pub fn start(
        registry: Arc<DeviceRegistry>,
        config: SessionConfig,
        sink: FrameSink,
    ) -> Result<Self, SessionError> {
        if config.frame_len == 0 {
            return Err(SessionError::Settings("frame_len must be at least 1"));
        }
        if config.event_capacity == 0 {
            return Err(SessionError::Settings("event_capacity must be at least 1"));
        }
        let input_choice = registry.resolve(Direction::Input)?;
        let output_choice = registry.resolve(Direction::Output)?;
        let (tx, events_rx) = sync_channel(config.event_capacity);
        let events = EventLink {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
        };
        let cancel = CancelFlag::new();
        let push_to_talk = PushToTalk::new();
        let backend = Arc::clone(registry.backend());

        let (queue, source) = playback_queue(output_choice.info.format, config.playback_capacity)?;
        let playback = PlaybackHandle::new(queue);
        let output_stream = open_output(
            backend.as_ref(),
            &output_choice.info,
            source,
            events.stream_errors(
                Direction::Output,
                output_choice.info.name.clone(),
                Some(playback.clone()),
            ),
        )?;

        let opened = open_input(
            backend.as_ref(),
            &input_choice.info,
            config.capture_buffer,
            events.stream_errors(Direction::Input, input_choice.info.name.clone(), None),
        )?;
        let gate = MicGate::new(
            Arc::new(playback.clone()),
            push_to_talk.clone(),
            SPEECH_SAMPLE_RATE,
        );
        let gate_counters = gate.counters();
        let path = CapturePath::new(
            opened.input.consumer,
            opened.input.converter,
            gate,
            config.frame_len,
        );
        let mailbox: Mailbox = Arc::new(Mutex::new(None));
        let worker = spawn_capture_worker(
            path,
            sink,
            Arc::clone(&mailbox),
            cancel.clone(),
            config.poll_interval,
            EventLink {
                tx: events.tx.clone(),
                dropped: Arc::clone(&events.dropped),
            },
        )?;

        Ok(Self {
            registry,
            config,
            cancel,
            push_to_talk,
            playback,
            mailbox,
            events,
            events_rx,
            gate_counters,
            capture_counters: vec![opened.counters],
            input_stream: Some(opened.stream),
            output_stream: Some(output_stream),
            input_device: input_choice.info,
            output_device: output_choice.info,
            workers: vec![worker],
            stopped: false,
        })
    }

    pub fn playback(&self) -> PlaybackHandle {
        self.playback.clone()
    }

    pub fn push_to_talk(&self) -> PushToTalk {
        self.push_to_talk.clone()
    }

    /// The flag every worker of this session watches. Hand a clone to anything
    /// that should end with the session.
    pub fn cancel_flag(&self) -> CancelFlag {
        self.cancel.clone()
    }

    pub fn input_device(&self) -> &DeviceInfo {
        &self.input_device
    }

    pub fn output_device(&self) -> &DeviceInfo {
        &self.output_device
    }

    /// The next pending event, if any.
    pub fn try_event(&self) -> Option<AudioEvent> {
        self.events_rx.try_recv().ok()
    }

    /// Waits up to `timeout` for the next event.
    pub fn wait_event(&self, timeout: Duration) -> Option<AudioEvent> {
        self.events_rx.recv_timeout(timeout).ok()
    }

    pub fn stats(&self) -> SessionStats {
        let mut capture = CaptureSnapshot::default();
        for counters in &self.capture_counters {
            let snap = counters.snapshot();
            capture.pushed_samples += snap.pushed_samples;
            capture.dropped_samples += snap.dropped_samples;
            capture.overflow_events += snap.overflow_events;
        }
        SessionStats {
            capture,
            gate: self.gate_counters.snapshot(),
            playback: self.playback.stats(),
            events_dropped: self.events.dropped.load(Ordering::Relaxed),
        }
    }

    /// Starts a worker thread tied to this session. `body` receives the session's
    /// cancel flag and must return soon after it is set. The thread is joined by
    /// [`stop`](Self::stop); the session unparks it on cancel, so a worker that
    /// waits with `std::thread::park_timeout` reacts at once.
    pub fn spawn_worker(
        &mut self,
        name: &str,
        body: impl FnOnce(CancelFlag) + Send + 'static,
    ) -> Result<(), SessionError> {
        if self.stopped || self.cancel.is_cancelled() {
            return Err(SessionError::Stopped);
        }
        let flag = self.cancel.clone();
        let handle = std::thread::Builder::new()
            .name(format!("lumingo-{name}"))
            .spawn(move || body(flag))?;
        self.workers.push(Worker {
            name: name.to_owned(),
            handle: Some(handle),
        });
        Ok(())
    }

    /// Reopens a stream on the remembered device, or the default one if that is
    /// gone, without restarting the session. This is the recovery path after
    /// [`AudioEvent::DeviceLost`]. Audio queued for a lost output is dropped.
    pub fn recover(&mut self, direction: Direction) -> Result<ResolvedDevice, SessionError> {
        if self.stopped || self.cancel.is_cancelled() {
            return Err(SessionError::Stopped);
        }
        let backend = Arc::clone(self.registry.backend());
        // The old stream goes first: some systems allow only one stream per device.
        let resolved = match direction {
            Direction::Input => {
                drop(self.input_stream.take());
                let resolved = self.registry.resolve(Direction::Input)?;
                let opened = open_input(
                    backend.as_ref(),
                    &resolved.info,
                    self.config.capture_buffer,
                    self.events
                        .stream_errors(Direction::Input, resolved.info.name.clone(), None),
                )?;
                *lock(&self.mailbox) = Some(opened.input);
                self.capture_counters.push(opened.counters);
                self.input_stream = Some(opened.stream);
                self.input_device = resolved.info.clone();
                resolved
            }
            Direction::Output => {
                drop(self.output_stream.take());
                self.playback.stop();
                let resolved = self.registry.resolve(Direction::Output)?;
                let (queue, source) =
                    playback_queue(resolved.info.format, self.config.playback_capacity)?;
                self.playback.replace(queue);
                let stream = open_output(
                    backend.as_ref(),
                    &resolved.info,
                    source,
                    self.events.stream_errors(
                        Direction::Output,
                        resolved.info.name.clone(),
                        Some(self.playback.clone()),
                    ),
                )?;
                self.output_stream = Some(stream);
                self.output_device = resolved.info.clone();
                resolved
            }
        };
        self.events.send(AudioEvent::Recovered {
            direction,
            device: resolved.info.name.clone(),
            choice: resolved.choice.clone(),
        });
        Ok(resolved)
    }

    /// Cancels the session without waiting: the shared flag is set, playback is
    /// flushed and every worker is woken. Call [`stop`](Self::stop) to wait.
    pub fn cancel(&self) {
        self.cancel.cancel();
        self.playback.stop();
        for worker in &self.workers {
            worker.wake();
        }
    }

    /// Cancels, joins every worker and closes every stream. When this returns,
    /// the session owns no thread and no stream.
    pub fn stop(mut self) -> StopReport {
        self.shutdown()
    }

    fn shutdown(&mut self) -> StopReport {
        self.cancel();
        // Closing the streams first ends the device threads and drops the ring's
        // producer, so the capture worker sees a closed input while it exits.
        drop(self.input_stream.take());
        drop(self.output_stream.take());
        let mut joined = 0;
        let mut panics = 0;
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                if handle.join().is_err() {
                    panics += 1;
                    tracing::error!(worker = %worker.name, "audio worker panicked");
                }
                joined += 1;
            }
        }
        self.workers.clear();
        self.stopped = true;
        StopReport {
            workers_joined: joined,
            worker_panics: panics,
        }
    }
}

impl Drop for AudioSession {
    fn drop(&mut self) {
        if !self.stopped {
            self.shutdown();
        }
    }
}

fn spawn_capture_worker(
    mut path: CapturePath,
    mut sink: FrameSink,
    mailbox: Mailbox,
    cancel: CancelFlag,
    poll: Duration,
    events: EventLink,
) -> Result<Worker, SessionError> {
    let handle = std::thread::Builder::new()
        .name("lumingo-capture".to_owned())
        .spawn(move || {
            while !cancel.is_cancelled() {
                if let Some(next) = lock(&mailbox).take() {
                    path.replace_input(next.consumer, next.converter);
                }
                match path.pump(&mut |frame, route| sink(frame, route)) {
                    Ok(pumped) if pumped.frames > 0 => {}
                    Ok(_) => std::thread::park_timeout(poll),
                    Err(error) => {
                        events.send(AudioEvent::ConversionFailed {
                            message: error.to_string(),
                        });
                        std::thread::park_timeout(poll);
                    }
                }
            }
        })?;
    Ok(Worker {
        name: "capture".to_owned(),
        handle: Some(handle),
    })
}
