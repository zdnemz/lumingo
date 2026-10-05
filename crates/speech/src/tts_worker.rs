//! The TTS worker: one engine on its own thread, one sentence at a time, in order.
//!
//! The input is sentence strings. Cutting a streamed reply into sentences is the
//! job of `SentenceChunker` in `crates/tutor-engine/src/chunker.rs`; this crate
//! does not duplicate it and does not depend on that crate. The orchestrator
//! connects them: for every LLM delta it calls `chunker.push(delta)` and speaks
//! each returned sentence through a [`TtsTurn`], and when the stream ends it
//! speaks `chunker.finish()`. The first sentence therefore reaches the engine
//! while the rest of the reply is still arriving, and its audio is on the output
//! queue while later sentences are still being synthesised.
//!
//! Concurrency rules, from `context_pack.md` sections 3.3 and 13:
//!
//! * The engine is built, warmed up and used on the worker thread only.
//! * Sentences wait in a bounded queue of [`TtsWorkerConfig::input_capacity`]
//!   (default 8). A reply can hold more sentences than that while the first one
//!   plays, because the LLM streams faster than speech. When the queue is full,
//!   [`TtsWorker::submit`] and [`TtsTurn::speak`] never block and never drop a
//!   sentence: they return the job in [`SubmitError::Full`], count it in
//!   [`TtsWorkerStats::rejected_full`], and the caller keeps the text and speaks
//!   it again after the next [`TtsEvent::Audio`] frees a slot. Reply text is
//!   bounded by the LLM token limit, so that holding area is small.
//! * Audio leaves through a sink, with a bounded channel of
//!   [`TtsWorkerConfig::output_capacity`] chunks (default 8) from
//!   [`TtsWorker::spawn`]. When the consumer is slower than synthesis the worker
//!   waits, in 5 ms steps that watch the cancel flag, instead of dropping audio
//!   or growing a backlog. Each wait is counted in
//!   [`TtsWorkerStats::output_waits`]. Events other than audio are tiny, are
//!   offered once, and are dropped and counted in
//!   [`TtsWorkerStats::events_dropped`] if the consumer is not draining.
//! * Every accepted sentence ends in one terminal event: `Audio`, `Cancelled`
//!   or `Failed`.
//! * Cancellation: a [`TtsTurn`] shares one [`CancelFlag`] over all its
//!   sentences. It is checked before synthesis, passed to the engine, and
//!   watched while waiting on the output. Queued sentences of a cancelled turn
//!   end as `Cancelled` without touching the engine. The 5 s limit per sentence
//!   is the caller's to enforce by cancelling the turn.
//! * Sentence text is never logged.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{CancelFlag, EngineInfo, PcmChunk, TtsEngine, TtsError, WorkerError};

/// Spoken once after load so the first real sentence does not pay for lazy
/// initialisation. Its audio is discarded.
const WARM_UP_TEXT: &str = "Hello.";

/// How long the worker sleeps between attempts to hand audio to a full consumer.
const OUTPUT_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug, Clone)]
pub struct TtsWorkerConfig {
    /// Sentences that may wait while one is synthesised.
    pub input_capacity: usize,
    /// Audio chunks held for the consumer. Used by [`TtsWorker::spawn`].
    pub output_capacity: usize,
}

impl Default for TtsWorkerConfig {
    fn default() -> Self {
        Self {
            input_capacity: 8,
            output_capacity: 8,
        }
    }
}

/// One sentence to synthesise.
#[derive(Debug, Clone)]
pub struct TtsJob {
    /// The tutor turn this sentence belongs to.
    pub turn_id: u64,
    /// Position of the sentence in its turn, from 0.
    pub index: u32,
    pub text: String,
    pub cancel: CancelFlag,
    submitted_at: Instant,
}

impl TtsJob {
    pub fn new(turn_id: u64, index: u32, text: impl Into<String>, cancel: CancelFlag) -> Self {
        Self {
            turn_id,
            index,
            text: text.into(),
            cancel,
            submitted_at: Instant::now(),
        }
    }
}

#[derive(Debug)]
pub enum TtsEvent {
    Ready {
        info: EngineInfo,
        load_ms: u64,
        warm_up_ms: u64,
    },
    LoadFailed {
        error: TtsError,
    },
    /// Audio for one sentence, at the engine's sample rate.
    Audio {
        turn_id: u64,
        index: u32,
        chunk: PcmChunk,
        info: EngineInfo,
        /// Time inside `synthesize`.
        synth_ms: u64,
        /// Time from `submit` to this event being produced. For the first
        /// sentence of a turn this is the "TTS first sentence" part of the
        /// latency budget, plus any wait behind an earlier turn.
        since_submit_ms: u64,
    },
    Cancelled {
        turn_id: u64,
        index: u32,
    },
    Failed {
        turn_id: u64,
        index: u32,
        error: TtsError,
    },
    /// The last event. Nothing follows it.
    Stopped,
}

/// What a sink says about an event it was offered.
#[derive(Debug)]
pub enum SinkResult {
    Accepted,
    /// No room now. The event comes back so the worker can offer it again.
    Full(TtsEvent),
    /// The consumer is gone. The worker stops.
    Closed,
}

#[derive(Debug)]
pub enum SubmitError {
    /// The queue is full. The job is returned unchanged.
    Full(TtsJob),
    /// The worker has stopped or never started.
    Stopped(TtsJob),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TtsWorkerStats {
    pub submitted: u64,
    pub rejected_full: u64,
    pub synthesized: u64,
    pub failed: u64,
    pub cancelled: u64,
    /// Times the worker had to wait because the consumer had no room for audio.
    pub output_waits: u64,
    pub events_dropped: u64,
}

#[derive(Default)]
struct Shared {
    stopping: AtomicBool,
    current: Mutex<Option<CancelFlag>>,
    submitted: AtomicU64,
    rejected_full: AtomicU64,
    synthesized: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    output_waits: AtomicU64,
    events_dropped: AtomicU64,
}

pub struct TtsWorker {
    tx: Option<SyncSender<TtsJob>>,
    handle: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for TtsWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtsWorker")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl TtsWorker {
    /// Starts the worker with a bounded audio channel.
    ///
    /// `load` runs on the worker thread. The first event is `Ready` or
    /// `LoadFailed`.
    pub fn spawn<L>(
        config: TtsWorkerConfig,
        load: L,
    ) -> Result<(Self, Receiver<TtsEvent>), WorkerError>
    where
        L: FnOnce() -> Result<Box<dyn TtsEngine>, TtsError> + Send + 'static,
    {
        if config.output_capacity == 0 {
            return Err(WorkerError::ZeroCapacity);
        }
        let (event_tx, event_rx) = mpsc::sync_channel(config.output_capacity);
        let worker =
            Self::spawn_with_sink(config, load, move |event| match event_tx.try_send(event) {
                Ok(()) => SinkResult::Accepted,
                Err(TrySendError::Full(event)) => SinkResult::Full(event),
                Err(TrySendError::Disconnected(_)) => SinkResult::Closed,
            })?;
        Ok((worker, event_rx))
    }

    /// Starts the worker with a caller-supplied sink. The sink must not block:
    /// it runs on the inference thread and reports `Full` instead.
    pub fn spawn_with_sink<L, S>(
        config: TtsWorkerConfig,
        load: L,
        sink: S,
    ) -> Result<Self, WorkerError>
    where
        L: FnOnce() -> Result<Box<dyn TtsEngine>, TtsError> + Send + 'static,
        S: FnMut(TtsEvent) -> SinkResult + Send + 'static,
    {
        if config.input_capacity == 0 {
            return Err(WorkerError::ZeroCapacity);
        }
        let (tx, rx) = mpsc::sync_channel(config.input_capacity);
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("tts".to_owned())
            .spawn(move || run(load, sink, rx, worker_shared))
            .map_err(|source| WorkerError::Spawn {
                name: "tts",
                source,
            })?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            shared,
        })
    }

    /// Starts a turn: sentences spoken through it share one cancel flag and are
    /// numbered in order.
    pub fn turn(&self, turn_id: u64) -> TtsTurn<'_> {
        TtsTurn {
            worker: self,
            turn_id,
            next_index: 0,
            cancel: CancelFlag::new(),
        }
    }

    /// Queues one sentence without blocking.
    pub fn submit(&self, job: TtsJob) -> Result<(), SubmitError> {
        let Some(tx) = &self.tx else {
            return Err(SubmitError::Stopped(job));
        };
        match tx.try_send(job) {
            Ok(()) => {
                self.shared.submitted.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Full(job)) => {
                self.shared.rejected_full.fetch_add(1, Ordering::Relaxed);
                Err(SubmitError::Full(job))
            }
            Err(TrySendError::Disconnected(job)) => Err(SubmitError::Stopped(job)),
        }
    }

    /// Cancels the sentence being synthesised, if any, without touching the
    /// rest of its turn. Cancel a [`TtsTurn`] to stop the whole reply.
    pub fn cancel_current(&self) {
        let current = self
            .shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(flag) = current.as_ref() {
            flag.cancel();
        }
    }

    pub fn stats(&self) -> TtsWorkerStats {
        let s = &self.shared;
        TtsWorkerStats {
            submitted: s.submitted.load(Ordering::Relaxed),
            rejected_full: s.rejected_full.load(Ordering::Relaxed),
            synthesized: s.synthesized.load(Ordering::Relaxed),
            failed: s.failed.load(Ordering::Relaxed),
            cancelled: s.cancelled.load(Ordering::Relaxed),
            output_waits: s.output_waits.load(Ordering::Relaxed),
            events_dropped: s.events_dropped.load(Ordering::Relaxed),
        }
    }

    /// Stops the worker: the sentence in flight is cancelled, queued ones end
    /// as `Cancelled`, then the thread is joined.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        self.cancel_current();
        self.tx = None;
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::error!("the tts worker thread panicked");
        }
    }
}

impl Drop for TtsWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The sentences of one tutor reply.
#[derive(Debug)]
pub struct TtsTurn<'w> {
    worker: &'w TtsWorker,
    turn_id: u64,
    next_index: u32,
    cancel: CancelFlag,
}

impl TtsTurn<'_> {
    pub fn turn_id(&self) -> u64 {
        self.turn_id
    }

    /// Queues the next sentence of the reply and returns its index. On `Full`
    /// the index is not used up, so speaking the same text again keeps the order.
    pub fn speak(&mut self, sentence: impl Into<String>) -> Result<u32, SubmitError> {
        let index = self.next_index;
        let job = TtsJob::new(self.turn_id, index, sentence, self.cancel.clone());
        self.worker.submit(job)?;
        self.next_index += 1;
        Ok(index)
    }

    /// Cancels every sentence of this turn, queued or in flight.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn cancel_flag(&self) -> CancelFlag {
        self.cancel.clone()
    }
}

fn run<S>(
    load: impl FnOnce() -> Result<Box<dyn TtsEngine>, TtsError>,
    mut sink: S,
    rx: Receiver<TtsJob>,
    shared: Arc<Shared>,
) where
    S: FnMut(TtsEvent) -> SinkResult,
{
    let load_started = Instant::now();
    let mut engine = match load() {
        Ok(engine) => engine,
        Err(error) => {
            tracing::warn!(%error, "the tts engine did not load");
            offer(&mut sink, &shared, TtsEvent::LoadFailed { error });
            return;
        }
    };
    let load_ms = elapsed_ms(load_started);

    let warm_started = Instant::now();
    if let Err(error) = engine.synthesize(WARM_UP_TEXT, &CancelFlag::new()) {
        tracing::warn!(%error, "the tts engine failed its warm-up");
        offer(&mut sink, &shared, TtsEvent::LoadFailed { error });
        return;
    }
    let info = engine.info();
    if !offer(
        &mut sink,
        &shared,
        TtsEvent::Ready {
            info: info.clone(),
            load_ms,
            warm_up_ms: elapsed_ms(warm_started),
        },
    ) {
        return;
    }

    while let Ok(job) = rx.recv() {
        let TtsJob {
            turn_id,
            index,
            text,
            cancel,
            submitted_at,
        } = job;
        if shared.stopping.load(Ordering::Acquire) || cancel.is_cancelled() {
            shared.cancelled.fetch_add(1, Ordering::Relaxed);
            if !offer(&mut sink, &shared, TtsEvent::Cancelled { turn_id, index }) {
                return;
            }
            continue;
        }
        set_current(&shared, Some(cancel.clone()));
        // A stop that arrived between the check above and the line before.
        if shared.stopping.load(Ordering::Acquire) {
            cancel.cancel();
        }
        let started = Instant::now();
        let result = if text.trim().is_empty() {
            Err(TtsError::EmptyText)
        } else {
            engine.synthesize(&text, &cancel)
        };
        let event = match result {
            Ok(chunk) => TtsEvent::Audio {
                turn_id,
                index,
                chunk,
                info: info.clone(),
                synth_ms: elapsed_ms(started),
                since_submit_ms: elapsed_ms(submitted_at),
            },
            Err(TtsError::Cancelled) => TtsEvent::Cancelled { turn_id, index },
            Err(error) => {
                tracing::warn!(turn_id, index, %error, "tts failed for a sentence");
                TtsEvent::Failed {
                    turn_id,
                    index,
                    error,
                }
            }
        };
        let alive = match event {
            TtsEvent::Audio { .. } => {
                // Audio is held, not dropped, until the consumer has room.
                deliver_audio(&mut sink, event, &cancel, &shared)
            }
            other => {
                match &other {
                    TtsEvent::Cancelled { .. } => &shared.cancelled,
                    _ => &shared.failed,
                }
                .fetch_add(1, Ordering::Relaxed);
                offer(&mut sink, &shared, other)
            }
        };
        set_current(&shared, None);
        if !alive {
            return;
        }
    }
    offer(&mut sink, &shared, TtsEvent::Stopped);
}

/// Hands one audio event to the sink, waiting while it is full. Gives up and
/// reports a cancellation if the turn is cancelled or the worker is stopping
/// while it waits. Returns false when the consumer is gone.
fn deliver_audio<S>(sink: &mut S, mut event: TtsEvent, cancel: &CancelFlag, shared: &Shared) -> bool
where
    S: FnMut(TtsEvent) -> SinkResult,
{
    let (turn_id, index) = match &event {
        TtsEvent::Audio { turn_id, index, .. } => (*turn_id, *index),
        _ => return true,
    };
    let mut waited = false;
    loop {
        match sink(event) {
            SinkResult::Accepted => {
                shared.synthesized.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            SinkResult::Closed => return false,
            SinkResult::Full(back) => {
                if !waited {
                    waited = true;
                    shared.output_waits.fetch_add(1, Ordering::Relaxed);
                }
                if cancel.is_cancelled() || shared.stopping.load(Ordering::Acquire) {
                    shared.cancelled.fetch_add(1, Ordering::Relaxed);
                    // Cancelled audio is not wanted, so a refusal here costs nothing.
                    return !matches!(
                        sink(TtsEvent::Cancelled { turn_id, index }),
                        SinkResult::Closed
                    );
                }
                event = back;
                std::thread::sleep(OUTPUT_RETRY);
            }
        }
    }
}

/// Offers an event once. A full consumer costs the event, which is counted.
/// Returns false when the consumer is gone.
fn offer<S>(sink: &mut S, shared: &Shared, event: TtsEvent) -> bool
where
    S: FnMut(TtsEvent) -> SinkResult,
{
    match sink(event) {
        SinkResult::Accepted => true,
        SinkResult::Full(_) => {
            shared.events_dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("a tts event was dropped because the consumer did not take it");
            true
        }
        SinkResult::Closed => false,
    }
}

fn set_current(shared: &Shared, flag: Option<CancelFlag>) {
    *shared
        .current
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = flag;
}

fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::RecvTimeoutError;

    const WAIT: Duration = Duration::from_secs(5);

    /// Test double. The audio length is the text length in samples, so a test
    /// can tell which sentence a chunk belongs to. `gate` holds `synthesize`
    /// until released or cancelled.
    struct FakeTts {
        spoken: Arc<Mutex<Vec<String>>>,
        gate: Option<Receiver<()>>,
        fail_on: Option<String>,
    }

    impl FakeTts {
        fn new(spoken: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                spoken,
                gate: None,
                fail_on: None,
            }
        }
    }

    impl TtsEngine for FakeTts {
        fn synthesize(&mut self, text: &str, cancel: &CancelFlag) -> Result<PcmChunk, TtsError> {
            self.spoken
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(text.to_owned());
            if self.fail_on.as_deref() == Some(text) {
                return Err(TtsError::Engine("synthesis broke".into()));
            }
            if text != WARM_UP_TEXT
                && let Some(gate) = &self.gate
            {
                loop {
                    if cancel.is_cancelled() {
                        return Err(TtsError::Cancelled);
                    }
                    match gate.recv_timeout(Duration::from_millis(5)) {
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                }
            }
            Ok(PcmChunk {
                samples: vec![0.0; text.len()],
                sample_rate: 24_000,
            })
        }
        fn info(&self) -> EngineInfo {
            EngineInfo::new("fake-tts", "test")
        }
    }

    fn start(config: TtsWorkerConfig, engine: FakeTts) -> (TtsWorker, Receiver<TtsEvent>) {
        TtsWorker::spawn(config, move || Ok(Box::new(engine) as Box<dyn TtsEngine>))
            .expect("worker starts")
    }

    fn next(rx: &Receiver<TtsEvent>) -> TtsEvent {
        rx.recv_timeout(WAIT).expect("an event within the wait")
    }

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !condition() {
            assert!(Instant::now() < deadline, "condition not reached in time");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn spoken(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[test]
    fn warm_up_speaks_once_before_ready_and_its_audio_is_discarded() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (worker, rx) = start(TtsWorkerConfig::default(), FakeTts::new(log.clone()));
        let TtsEvent::Ready { info, .. } = next(&rx) else {
            panic!("the first event must be Ready");
        };
        assert_eq!(info.id, "fake-tts");
        assert_eq!(spoken(&log), vec![WARM_UP_TEXT.to_owned()]);
        assert!(rx.try_recv().is_err(), "no audio for the warm-up");
        drop(worker);
    }

    #[test]
    fn sentences_come_back_in_order_with_their_turn_and_index() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (worker, rx) = start(TtsWorkerConfig::default(), FakeTts::new(log.clone()));
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));
        let mut turn = worker.turn(9);
        assert_eq!(turn.speak("One.").expect("queued"), 0);
        assert_eq!(turn.speak("Two two.").expect("queued"), 1);
        assert_eq!(turn.speak("Three three three.").expect("queued"), 2);
        let mut got = Vec::new();
        for _ in 0..3 {
            let TtsEvent::Audio {
                turn_id,
                index,
                chunk,
                ..
            } = next(&rx)
            else {
                panic!("expected audio");
            };
            got.push((turn_id, index, chunk.samples.len()));
        }
        assert_eq!(got, vec![(9, 0, 4), (9, 1, 8), (9, 2, 18)]);
        // The counter moves just after the sink accepts, so it may trail the event.
        wait_for(|| worker.stats().synthesized == 3);
        worker.shutdown();
        assert!(matches!(next(&rx), TtsEvent::Stopped));
    }

    #[test]
    fn the_first_sentence_is_audio_before_the_rest_are_even_submitted() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (worker, rx) = start(TtsWorkerConfig::default(), FakeTts::new(log));
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));
        let mut turn = worker.turn(1);
        turn.speak("First sentence.").expect("queued");
        // The reply is still streaming: nothing else has been submitted yet.
        let TtsEvent::Audio { index, .. } = next(&rx) else {
            panic!("expected audio");
        };
        assert_eq!(index, 0);
        turn.speak("Second sentence.").expect("queued");
        assert!(matches!(next(&rx), TtsEvent::Audio { index: 1, .. }));
    }

    #[test]
    fn a_full_queue_returns_the_sentence_and_keeps_the_order_on_retry() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (release, gate) = mpsc::channel();
        let mut engine = FakeTts::new(log.clone());
        engine.gate = Some(gate);
        let config = TtsWorkerConfig {
            input_capacity: 1,
            ..TtsWorkerConfig::default()
        };
        let (worker, rx) = start(config, engine);
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));

        let mut turn = worker.turn(1);
        turn.speak("A.").expect("taken by the worker");
        wait_for(|| spoken(&log).len() == 2);
        turn.speak("B.").expect("fills the queue");
        let SubmitError::Full(rejected) = turn.speak("C.").expect_err("full") else {
            panic!("expected Full");
        };
        assert_eq!((rejected.index, rejected.text.as_str()), (2, "C."));
        assert_eq!(worker.stats().rejected_full, 1);

        release.send(()).expect("release A");
        assert!(matches!(next(&rx), TtsEvent::Audio { index: 0, .. }));
        // The slot is free again: the same text takes the same index.
        wait_for(|| turn.speak("C.").is_ok());
        release.send(()).expect("release B");
        release.send(()).expect("release C");
        assert!(matches!(next(&rx), TtsEvent::Audio { index: 1, .. }));
        assert!(matches!(next(&rx), TtsEvent::Audio { index: 2, .. }));
    }

    #[test]
    fn cancelling_a_turn_cancels_the_running_sentence_and_the_queued_ones() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (_release, gate) = mpsc::channel::<()>();
        let mut engine = FakeTts::new(log.clone());
        engine.gate = Some(gate);
        let (worker, rx) = start(TtsWorkerConfig::default(), engine);
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));

        let mut turn = worker.turn(4);
        turn.speak("A.").expect("queued");
        wait_for(|| spoken(&log).len() == 2);
        turn.speak("B.").expect("queued");
        turn.speak("C.").expect("queued");
        turn.cancel();

        let mut cancelled = Vec::new();
        for _ in 0..3 {
            let TtsEvent::Cancelled { turn_id, index } = next(&rx) else {
                panic!("expected Cancelled");
            };
            assert_eq!(turn_id, 4);
            cancelled.push(index);
        }
        assert_eq!(cancelled, vec![0, 1, 2]);
        // Only the warm-up and the running sentence reached the engine.
        assert_eq!(spoken(&log).len(), 2);

        // A new turn is unaffected by the cancelled one.
        let mut next_turn = worker.turn(5);
        next_turn.speak("D.").expect("queued");
        wait_for(|| spoken(&log).len() == 3);
        worker.cancel_current();
        assert!(matches!(next(&rx), TtsEvent::Cancelled { turn_id: 5, .. }));
        assert_eq!(worker.stats().cancelled, 4);
    }

    #[test]
    fn an_engine_error_fails_one_sentence_and_the_worker_goes_on() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeTts::new(log);
        engine.fail_on = Some("Bad.".to_owned());
        let (worker, rx) = start(TtsWorkerConfig::default(), engine);
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));
        let mut turn = worker.turn(1);
        turn.speak("Bad.").expect("queued");
        turn.speak("Good.").expect("queued");
        let TtsEvent::Failed { index, error, .. } = next(&rx) else {
            panic!("expected Failed");
        };
        assert_eq!(index, 0);
        assert!(error.to_string().contains("synthesis broke"));
        assert!(matches!(next(&rx), TtsEvent::Audio { index: 1, .. }));
        assert_eq!(worker.stats().failed, 1);
    }

    #[test]
    fn blank_text_fails_without_reaching_the_engine() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (worker, rx) = start(TtsWorkerConfig::default(), FakeTts::new(log.clone()));
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));
        worker.turn(1).speak("   ").expect("queued");
        assert!(matches!(
            next(&rx),
            TtsEvent::Failed {
                error: TtsError::EmptyText,
                ..
            }
        ));
        assert_eq!(spoken(&log).len(), 1, "only the warm-up");
    }

    #[test]
    fn a_slow_consumer_makes_the_worker_wait_and_no_audio_is_lost() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let config = TtsWorkerConfig {
            input_capacity: 8,
            output_capacity: 1,
        };
        let (worker, rx) = start(config, FakeTts::new(log));
        // Not draining yet: Ready fills the single output slot.
        let mut turn = worker.turn(1);
        for text in ["A.", "B.", "C."] {
            turn.speak(text).expect("queued");
        }
        wait_for(|| worker.stats().output_waits >= 1);
        assert_eq!(worker.stats().events_dropped, 0);
        let mut indices = Vec::new();
        while indices.len() < 3 {
            if let TtsEvent::Audio { index, .. } = next(&rx) {
                indices.push(index);
            }
        }
        assert_eq!(indices, vec![0, 1, 2]);
        assert_eq!(worker.stats().events_dropped, 0);
    }

    #[test]
    fn cancelling_while_waiting_on_a_full_output_does_not_hang() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let config = TtsWorkerConfig {
            input_capacity: 8,
            output_capacity: 1,
        };
        let (worker, _rx) = start(config, FakeTts::new(log));
        let mut turn = worker.turn(1);
        turn.speak("A.").expect("queued");
        wait_for(|| worker.stats().output_waits >= 1);
        turn.cancel();
        wait_for(|| worker.stats().cancelled >= 1);
        // The worker is free again for the next turn.
        assert_eq!(worker.stats().synthesized, 0);
    }

    #[test]
    fn shutdown_ends_queued_sentences_as_cancelled_and_joins() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (_release, gate) = mpsc::channel::<()>();
        let mut engine = FakeTts::new(log.clone());
        engine.gate = Some(gate);
        let (worker, rx) = start(TtsWorkerConfig::default(), engine);
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));
        let mut turn = worker.turn(1);
        turn.speak("A.").expect("queued");
        wait_for(|| spoken(&log).len() == 2);
        turn.speak("B.").expect("queued");
        turn.speak("C.").expect("queued");

        worker.shutdown();

        let mut cancelled = Vec::new();
        loop {
            match next(&rx) {
                TtsEvent::Cancelled { index, .. } => cancelled.push(index),
                TtsEvent::Stopped => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(cancelled, vec![0, 1, 2]);
        assert!(matches!(
            rx.recv_timeout(WAIT),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn a_load_failure_is_reported_and_later_submissions_are_refused() {
        let (worker, rx) = TtsWorker::spawn(TtsWorkerConfig::default(), || {
            Err(TtsError::Unavailable {
                reason: "no voice".into(),
            })
        })
        .expect("thread starts");
        assert!(matches!(next(&rx), TtsEvent::LoadFailed { .. }));
        wait_for(|| matches!(worker.turn(1).speak("Hi."), Err(SubmitError::Stopped(_))));
    }

    #[test]
    fn a_dropped_receiver_stops_the_worker() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (worker, rx) = start(TtsWorkerConfig::default(), FakeTts::new(log));
        assert!(matches!(next(&rx), TtsEvent::Ready { .. }));
        drop(rx);
        worker.turn(1).speak("A.").expect("queued");
        wait_for(|| matches!(worker.turn(2).speak("B."), Err(SubmitError::Stopped(_))));
    }

    #[test]
    fn an_unavailable_engine_reports_through_the_worker() {
        let (_worker, rx) = TtsWorker::spawn(TtsWorkerConfig::default(), || {
            Ok(
                Box::new(crate::UnavailableEngine::new("built without sherpa"))
                    as Box<dyn TtsEngine>,
            )
        })
        .expect("thread starts");
        let TtsEvent::LoadFailed { error } = next(&rx) else {
            panic!("expected LoadFailed");
        };
        assert!(matches!(error, TtsError::Unavailable { .. }));
    }

    #[test]
    fn zero_capacities_are_refused() {
        let zero_input = TtsWorkerConfig {
            input_capacity: 0,
            ..TtsWorkerConfig::default()
        };
        assert!(matches!(
            TtsWorker::spawn(zero_input, || Err(TtsError::Engine("x".into()))),
            Err(WorkerError::ZeroCapacity)
        ));
        let zero_output = TtsWorkerConfig {
            output_capacity: 0,
            ..TtsWorkerConfig::default()
        };
        assert!(matches!(
            TtsWorker::spawn(zero_output, || Err(TtsError::Engine("x".into()))),
            Err(WorkerError::ZeroCapacity)
        ));
    }
}
