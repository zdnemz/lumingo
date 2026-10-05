//! Doubles for the voice loop: a manual clock, a scripted language model, fake
//! speech engines and a fake audio backend.
//!
//! This module exists only for this crate's own tests and with the
//! `test-support` feature, which no release build enables. Nothing here pretends
//! to be an engine: every double does exactly what its script says, so a test
//! knows what the loop should have seen.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use audio_io::fake::FakeBackend;
use audio_io::{
    AudioBackend, AudioStream, DeviceError, DeviceInfo, DeviceRegistry, Direction, InputCallback,
    MemoryPrefs, OutputCallback, StreamErrorCallback, StreamFormat,
};
use llm_client::{
    Capabilities, FinishReason, LadderLevel, LlmClient, LlmError, StreamEvent, StreamSummary,
    StructuredOutput, StructuredRequest, TextRequest, TextStream, TimeoutKind, TransportKind,
};
use serde_json::{Value, json};
use speech::{
    CancelFlag, EngineInfo, PcmChunk, SttEngine, SttError, Transcript, TtsEngine, TtsError, Vad,
    VadError,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::clock::LoopClock;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- the clock -------------------------------------------------------------

/// A clock that only moves when a double moves it. Clones share one reading.
#[derive(Debug, Clone, Default)]
pub struct ManualClock {
    micros: Arc<AtomicU64>,
}

impl ManualClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn advance(&self, by: Duration) {
        self.micros.fetch_add(
            u64::try_from(by.as_micros()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
    }
}

impl LoopClock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_micros(self.micros.load(Ordering::SeqCst))
    }
}

// ---- the language model ------------------------------------------------------

/// How a scripted call fails.
#[derive(Debug, Clone, Copy)]
pub enum FailKind {
    /// Nothing listens on the address.
    Unreachable,
    /// HTTP 503.
    Unavailable,
    /// The provider answers 401.
    Auth,
    /// The first token never came.
    Timeout,
}

impl FailKind {
    fn error(self) -> LlmError {
        match self {
            Self::Unreachable => LlmError::Transport(TransportKind::Connect),
            Self::Unavailable => LlmError::Server {
                status: 503,
                message: "unavailable".to_owned(),
            },
            Self::Auth => LlmError::Auth { status: 401 },
            Self::Timeout => LlmError::Timeout(TimeoutKind::FirstToken),
        }
    }
}

/// One streamed reply.
#[derive(Clone)]
pub struct ReplyScript {
    pub deltas: Vec<String>,
    /// The manual clock moves by this much before the first delta: the model's
    /// time to first token.
    pub advance_first: Duration,
    /// After the delta with this index the stream waits for a permit before it
    /// goes on. A test uses it to look at the loop while the stream is open.
    pub hold_after: Option<(usize, Arc<Semaphore>)>,
    pub finish: FinishReason,
}

impl ReplyScript {
    pub fn new(deltas: &[&str]) -> Self {
        Self {
            deltas: deltas.iter().map(|d| (*d).to_owned()).collect(),
            advance_first: Duration::ZERO,
            hold_after: None,
            finish: FinishReason::Stop,
        }
    }

    #[must_use]
    pub fn after(mut self, advance_first: Duration) -> Self {
        self.advance_first = advance_first;
        self
    }

    #[must_use]
    pub fn hold_after(mut self, index: usize, permit: Arc<Semaphore>) -> Self {
        self.hold_after = Some((index, permit));
        self
    }
}

pub enum Step {
    Reply(ReplyScript),
    /// The call fails.
    Fail(FailKind),
    /// The call is accepted and never answers, until it is cancelled.
    Hang,
    /// The stream ends at once with no text and this reason.
    Empty(FinishReason),
}

/// A scripted `LlmClient`. A call with nothing left in the script fails like a
/// provider that is down, so a test cannot pass by accident.
pub struct ScriptedLlm {
    steps: Mutex<VecDeque<Step>>,
    clock: Option<Arc<ManualClock>>,
    seen: Mutex<Vec<TextRequest>>,
    cancelled: Arc<AtomicUsize>,
    structured_delay: Mutex<Duration>,
    structured_calls: AtomicUsize,
}

impl ScriptedLlm {
    pub fn new(steps: Vec<Step>, clock: Option<Arc<ManualClock>>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            clock,
            seen: Mutex::new(Vec::new()),
            cancelled: Arc::new(AtomicUsize::new(0)),
            structured_delay: Mutex::new(Duration::ZERO),
            structured_calls: AtomicUsize::new(0),
        })
    }

    /// Adds a step at the end of the script.
    pub fn push(&self, step: Step) {
        lock(&self.steps).push_back(step);
    }

    /// Makes every structured call (the background analysis) take this long.
    pub fn set_structured_delay(&self, delay: Duration) {
        *lock(&self.structured_delay) = delay;
    }

    pub fn requests(&self) -> Vec<TextRequest> {
        lock(&self.seen).clone()
    }

    pub fn calls(&self) -> usize {
        lock(&self.seen).len()
    }

    /// Streams that ended because their token was cancelled.
    pub fn cancellations(&self) -> usize {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn structured_calls(&self) -> usize {
        self.structured_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmClient for ScriptedLlm {
    async fn stream_text(
        &self,
        request: TextRequest,
        cancel: CancellationToken,
    ) -> Result<TextStream, LlmError> {
        lock(&self.seen).push(request);
        let step = lock(&self.steps).pop_front();
        match step {
            None => Err(LlmError::Transport(TransportKind::Connect)),
            Some(Step::Fail(kind)) => Err(kind.error()),
            Some(Step::Hang) => {
                cancel.cancelled().await;
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                Err(LlmError::Cancelled)
            }
            Some(Step::Empty(finish)) => Ok(TextStream::new(futures_util::stream::iter(vec![Ok(
                StreamEvent::Finished(StreamSummary {
                    finish,
                    usage: None,
                    time_to_first_token: None,
                }),
            )]))),
            Some(Step::Reply(script)) => {
                let counter = Arc::clone(&self.cancelled);
                Ok(TextStream::new(futures_util::stream::unfold(
                    ReplyState {
                        deltas: script.deltas.into(),
                        index: 0,
                        advance_first: script.advance_first,
                        hold_after: script.hold_after,
                        finish: script.finish,
                        clock: self.clock.clone(),
                        cancel,
                        counter,
                        done: false,
                    },
                    next_event,
                )))
            }
        }
    }

    async fn structured(
        &self,
        request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Result<StructuredOutput, LlmError> {
        self.structured_calls.fetch_add(1, Ordering::SeqCst);
        let delay = *lock(&self.structured_delay);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        // An analysis that finds nothing wrong, for every turn the call asks about.
        let asked: Value = request
            .messages
            .first()
            .and_then(|m| serde_json::from_str(&m.content).ok())
            .unwrap_or(Value::Null);
        let turns: Vec<Value> = asked["turns"]
            .as_array()
            .map(|turns| {
                turns
                    .iter()
                    .map(|t| {
                        json!({
                            "turn_seq": t["turn_seq"],
                            "errors": [],
                            "objective_evidence": [],
                            "understood_tutor": "yes",
                            "note_for_next_turn": "",
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(StructuredOutput {
            value: json!({ "turns": turns }),
            ladder_level: LadderLevel::NativeSchema,
            repaired: false,
            calls: 1,
            usage: None,
        })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
}

struct ReplyState {
    deltas: VecDeque<String>,
    index: usize,
    advance_first: Duration,
    hold_after: Option<(usize, Arc<Semaphore>)>,
    finish: FinishReason,
    clock: Option<Arc<ManualClock>>,
    cancel: CancellationToken,
    counter: Arc<AtomicUsize>,
    done: bool,
}

async fn next_event(mut state: ReplyState) -> Option<(Result<StreamEvent, LlmError>, ReplyState)> {
    if state.done {
        return None;
    }
    let hold = state.hold_after.clone();
    if let Some((after, permit)) = &hold
        && state.index == after + 1
    {
        tokio::select! {
            () = state.cancel.cancelled() => {
                state.counter.fetch_add(1, Ordering::SeqCst);
                state.done = true;
                return Some((Err(LlmError::Cancelled), state));
            }
            granted = permit.acquire() => {
                if let Ok(granted) = granted {
                    granted.forget();
                }
            }
        }
    }
    if state.cancel.is_cancelled() {
        state.counter.fetch_add(1, Ordering::SeqCst);
        state.done = true;
        return Some((Err(LlmError::Cancelled), state));
    }
    match state.deltas.pop_front() {
        Some(delta) => {
            if state.index == 0
                && let Some(clock) = &state.clock
            {
                clock.advance(state.advance_first);
            }
            state.index += 1;
            // Yield so a cancellation set meanwhile is seen before the next delta.
            tokio::task::yield_now().await;
            Some((Ok(StreamEvent::Delta(delta)), state))
        }
        None => {
            state.done = true;
            let finish = state.finish.clone();
            Some((
                Ok(StreamEvent::Finished(StreamSummary {
                    finish,
                    usage: None,
                    time_to_first_token: None,
                })),
                state,
            ))
        }
    }
}

// ---- speech engines ------------------------------------------------------------

/// A recogniser that returns the next scripted transcript for every utterance.
pub struct FakeStt {
    transcripts: VecDeque<String>,
    clock: Option<Arc<ManualClock>>,
    finalise: Duration,
    fed: usize,
    /// Fails the load of the engine instead.
    fail_start: bool,
}

impl FakeStt {
    pub fn new(transcripts: &[&str], clock: Option<Arc<ManualClock>>, finalise: Duration) -> Self {
        Self {
            transcripts: transcripts.iter().map(|t| (*t).to_owned()).collect(),
            clock,
            finalise,
            fed: 0,
            fail_start: false,
        }
    }

    #[must_use]
    pub fn failing(mut self) -> Self {
        self.fail_start = true;
        self
    }
}

impl SttEngine for FakeStt {
    fn start(&mut self) -> Result<(), SttError> {
        if self.fail_start {
            return Err(SttError::Engine(
                "the fake recogniser was told to fail".to_owned(),
            ));
        }
        self.fed = 0;
        Ok(())
    }

    fn feed(&mut self, samples: &[f32]) -> Result<(), SttError> {
        self.fed += samples.len();
        Ok(())
    }

    fn finish(&mut self, cancel: &CancelFlag) -> Result<Transcript, SttError> {
        if cancel.is_cancelled() {
            return Err(SttError::Cancelled);
        }
        if let Some(clock) = &self.clock {
            clock.advance(self.finalise);
        }
        // The one-second warm-up the worker runs after load has no scripted
        // transcript; it gets an empty one.
        let text = if self.fed <= 16_000 {
            String::new()
        } else {
            self.transcripts.pop_front().unwrap_or_default()
        };
        Ok(Transcript {
            text,
            words: Vec::new(),
            duration_ms: u32::try_from(self.fed / 16).unwrap_or(u32::MAX),
        })
    }

    fn info(&self) -> EngineInfo {
        EngineInfo::new("fake-stt", "test")
    }
}

/// What a fake synthesiser did, for the test to read.
#[derive(Debug, Default)]
pub struct TtsLog {
    pub spoken: Mutex<Vec<String>>,
    pub cancelled: AtomicUsize,
}

/// A synthesiser that answers every sentence with a tone as long as the text:
/// 40 ms per character, at least 200 ms, at 24 kHz.
pub struct FakeTts {
    clock: Option<Arc<ManualClock>>,
    latency: Duration,
    log: Arc<TtsLog>,
    /// The sentence containing this text fails.
    fail_on: Option<String>,
    /// Every sentence takes this long in real time, to look at a turn in flight.
    real_delay: Duration,
}

impl FakeTts {
    pub fn new(clock: Option<Arc<ManualClock>>, latency: Duration) -> (Self, Arc<TtsLog>) {
        let log = Arc::new(TtsLog::default());
        (
            Self {
                clock,
                latency,
                log: Arc::clone(&log),
                fail_on: None,
                real_delay: Duration::ZERO,
            },
            log,
        )
    }

    #[must_use]
    pub fn failing_on(mut self, text: &str) -> Self {
        self.fail_on = Some(text.to_owned());
        self
    }

    #[must_use]
    pub fn slow(mut self, real_delay: Duration) -> Self {
        self.real_delay = real_delay;
        self
    }
}

impl TtsEngine for FakeTts {
    fn synthesize(&mut self, text: &str, cancel: &CancelFlag) -> Result<PcmChunk, TtsError> {
        if cancel.is_cancelled() {
            self.log.cancelled.fetch_add(1, Ordering::SeqCst);
            return Err(TtsError::Cancelled);
        }
        if !self.real_delay.is_zero() {
            std::thread::sleep(self.real_delay);
            if cancel.is_cancelled() {
                self.log.cancelled.fetch_add(1, Ordering::SeqCst);
                return Err(TtsError::Cancelled);
            }
        }
        if self
            .fail_on
            .as_deref()
            .is_some_and(|bad| text.contains(bad))
        {
            return Err(TtsError::Engine(
                "the fake synthesiser was told to fail".to_owned(),
            ));
        }
        // The warm-up sentence is not part of any turn.
        if text != "Hello." {
            lock(&self.log.spoken).push(text.to_owned());
            if let Some(clock) = &self.clock {
                clock.advance(self.latency);
            }
        }
        let rate = 24_000_u32;
        let ms = (text.chars().count() as u64 * 40).max(200);
        let count = usize::try_from(u64::from(rate) * ms / 1000).unwrap_or(0);
        let samples = (0..count)
            .map(|i| 0.25 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin())
            .collect();
        Ok(PcmChunk {
            samples,
            sample_rate: rate,
        })
    }

    fn info(&self) -> EngineInfo {
        EngineInfo::new("fake-tts", "test")
    }
}

/// A VAD that calls a frame speech when its RMS is above 0.05.
///
/// With a clock it also plays the part of real time: from the first speech
/// frame to the frame that ends the utterance, every frame moves the clock by
/// the frame's length, so the measured endpointing wait is exactly the number
/// of silent frames the endpointer needs times that length.
pub struct EnergyVad {
    clock: Option<Arc<ManualClock>>,
    frame: Duration,
    end_silence_frames: u32,
    active: bool,
    silent: u32,
}

impl EnergyVad {
    pub fn new(clock: Option<Arc<ManualClock>>, end_silence_ms: u32) -> Self {
        let frame = Duration::from_millis(32);
        Self {
            clock,
            frame,
            end_silence_frames: end_silence_ms.div_ceil(32),
            active: false,
            silent: 0,
        }
    }
}

impl Vad for EnergyVad {
    fn push_frame(&mut self, frame: &[f32]) -> Result<f32, VadError> {
        if frame.len() != 512 {
            return Err(VadError::FrameSize {
                expected: 512,
                got: frame.len(),
            });
        }
        let energy = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        let speech = energy >= 0.05;
        if speech {
            self.active = true;
            self.silent = 0;
        } else if self.active {
            self.silent += 1;
        }
        if self.active
            && let Some(clock) = &self.clock
        {
            clock.advance(self.frame);
        }
        if self.silent >= self.end_silence_frames {
            self.active = false;
            self.silent = 0;
        }
        Ok(if speech { 0.9 } else { 0.05 })
    }

    fn reset(&mut self) {
        self.active = false;
        self.silent = 0;
    }
}

/// `ms` of a 220 Hz tone at 16 kHz, loud enough for [`EnergyVad`].
pub fn speech_audio(ms: u32) -> Vec<f32> {
    let count = (u64::from(ms) * 16) as usize;
    (0..count)
        .map(|i| 0.4 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin())
        .collect()
}

/// `ms` of silence at 16 kHz.
pub fn silence_audio(ms: u32) -> Vec<f32> {
    vec![0.0; (u64::from(ms) * 16) as usize]
}

// ---- audio ---------------------------------------------------------------------

/// A fake backend whose output can be held: while the hold is on, the output
/// stream plays silence and the queue is not consumed. A test uses it to move
/// the manual clock between "audio queued" and "first sample played".
#[derive(Clone)]
pub struct HeldBackend {
    pub inner: FakeBackend,
    hold: Arc<AtomicBool>,
}

impl HeldBackend {
    pub fn hold(&self, on: bool) {
        self.hold.store(on, Ordering::SeqCst);
    }
}

struct HeldStream(#[allow(dead_code)] Box<dyn AudioStream>);
impl AudioStream for HeldStream {}

impl AudioBackend for HeldBackend {
    fn devices(&self, direction: Direction) -> Result<Vec<DeviceInfo>, DeviceError> {
        self.inner.devices(direction)
    }

    fn open_input(
        &self,
        device: &DeviceInfo,
        callback: InputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError> {
        self.inner.open_input(device, callback, on_error)
    }

    fn open_output(
        &self,
        device: &DeviceInfo,
        mut callback: OutputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError> {
        let hold = Arc::clone(&self.hold);
        let stream = self.inner.open_output(
            device,
            Box::new(move |out| {
                if hold.load(Ordering::SeqCst) {
                    out.fill(0.0);
                } else {
                    callback(out);
                }
            }),
            on_error,
        )?;
        Ok(Box::new(HeldStream(stream)))
    }
}

/// A registry over a held fake backend with a default microphone (16 kHz mono)
/// and a default speaker (48 kHz stereo), so conversion runs on both sides.
pub fn fake_audio() -> (Arc<DeviceRegistry>, HeldBackend) {
    let inner = FakeBackend::new();
    inner.add_device(
        "Fake microphone",
        Direction::Input,
        StreamFormat::new(16_000, 1),
        true,
    );
    inner.add_device(
        "Fake speaker",
        Direction::Output,
        StreamFormat::new(48_000, 2),
        true,
    );
    let backend = HeldBackend {
        inner,
        hold: Arc::new(AtomicBool::new(false)),
    };
    let registry = Arc::new(DeviceRegistry::new(
        Arc::new(backend.clone()),
        Box::new(MemoryPrefs::default()),
    ));
    (registry, backend)
}
