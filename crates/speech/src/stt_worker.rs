//! The STT worker: one engine on its own thread, one utterance at a time.
//!
//! Concurrency rules, from `context_pack.md` sections 3.3 and 13:
//!
//! * The engine is built, warmed up and used on the worker thread only. Loading
//!   and inference never run on a Tokio worker.
//! * Utterances arrive through a bounded queue of [`SttWorkerConfig::input_capacity`]
//!   (default 4). An utterance takes several seconds to speak and the worker
//!   decodes faster than real time on the target machines, so a queue deeper
//!   than a few means the engine is stuck or too slow. When the queue is full,
//!   [`SttWorker::submit`] does not block the caller: it hands the job back in
//!   [`SubmitError::Full`] and counts it in [`SttWorkerStats::rejected_full`].
//!   The caller decides what the learner sees; nothing is dropped silently.
//! * Every submitted job ends in exactly one terminal event: `Transcript`,
//!   `Cancelled` or `Failed`. A job rejected by `submit` produces no event
//!   because it never entered the worker.
//! * Events leave through a sink. [`SttWorker::spawn`] wires a bounded channel of
//!   [`SttWorkerConfig::event_capacity`] (default 32). The consumer drains it
//!   once per utterance, so a full channel means the consumer is gone. The worker
//!   never blocks on it: the event is dropped, counted in
//!   [`SttWorkerStats::events_dropped`] and logged.
//! * Cancellation: each job carries its own [`CancelFlag`], checked before the
//!   job starts, between audio chunks and by the engine in `finish`.
//!   [`SttWorker::cancel_current`] and shutdown cancel the job in flight. The
//!   10 s limit for STT is the caller's to enforce by cancelling the job's flag,
//!   because a native call cannot be interrupted from outside.
//! * Learner text is never logged here.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::{
    CancelFlag, EngineInfo, SPEECH_SAMPLE_RATE, SttEngine, SttError, Transcript, WorkerError,
};

/// One second of silence runs through the engine after load, so the first real
/// utterance does not pay for lazy initialisation.
const WARM_UP_SAMPLES: usize = SPEECH_SAMPLE_RATE as usize;

#[derive(Debug, Clone)]
pub struct SttWorkerConfig {
    /// Utterances that may wait while one is decoding.
    pub input_capacity: usize,
    /// Events held for the consumer before new ones are dropped.
    pub event_capacity: usize,
    /// Samples handed to `feed` at a time. Smaller chunks let a streaming
    /// engine decode while the rest is fed and let cancellation act sooner.
    pub feed_chunk_samples: usize,
}

impl Default for SttWorkerConfig {
    fn default() -> Self {
        Self {
            input_capacity: 4,
            event_capacity: 32,
            // 200 ms
            feed_chunk_samples: 3200,
        }
    }
}

/// One utterance to recognise.
#[derive(Debug, Clone)]
pub struct SttJob {
    /// Chosen by the caller, echoed in the terminal event.
    pub utterance_id: u64,
    /// 16 kHz mono samples.
    pub samples: Vec<f32>,
    pub cancel: CancelFlag,
}

#[derive(Debug)]
pub enum SttEvent {
    /// The engine loaded and ran its warm-up. Jobs are accepted before this
    /// event, but they wait behind the load.
    Ready {
        info: EngineInfo,
        load_ms: u64,
        warm_up_ms: u64,
    },
    /// The engine could not load or warm up. The worker thread ends and later
    /// submissions fail with [`SubmitError::Stopped`].
    LoadFailed {
        error: SttError,
    },
    Transcript {
        utterance_id: u64,
        transcript: Transcript,
        /// The engine and model that produced the text, for evidence rows.
        info: EngineInfo,
        /// Time spent in `finish`: the "STT finalise" part of the latency budget.
        finalise_ms: u64,
        /// Time from the start of the job to the transcript.
        total_ms: u64,
    },
    Cancelled {
        utterance_id: u64,
    },
    Failed {
        utterance_id: u64,
        error: SttError,
    },
    /// The last event. Nothing follows it.
    Stopped,
}

#[derive(Debug)]
pub enum SubmitError {
    /// The queue is full. The job is returned unchanged.
    Full(SttJob),
    /// The worker has stopped or never started.
    Stopped(SttJob),
}

/// Counters since the worker started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SttWorkerStats {
    pub submitted: u64,
    pub rejected_full: u64,
    pub transcribed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub events_dropped: u64,
}

#[derive(Default)]
struct Shared {
    stopping: AtomicBool,
    current: Mutex<Option<CancelFlag>>,
    submitted: AtomicU64,
    rejected_full: AtomicU64,
    transcribed: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    events_dropped: AtomicU64,
}

pub struct SttWorker {
    tx: Option<SyncSender<SttJob>>,
    handle: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for SttWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SttWorker")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl SttWorker {
    /// Starts the worker with a bounded event channel.
    ///
    /// `load` runs on the worker thread, so a slow model load does not hold the
    /// caller. The first event is `Ready` or `LoadFailed`.
    pub fn spawn<L>(
        config: SttWorkerConfig,
        load: L,
    ) -> Result<(Self, Receiver<SttEvent>), WorkerError>
    where
        L: FnOnce() -> Result<Box<dyn SttEngine>, SttError> + Send + 'static,
    {
        if config.event_capacity == 0 {
            return Err(WorkerError::ZeroCapacity);
        }
        let (event_tx, event_rx) = mpsc::sync_channel(config.event_capacity);
        let worker = Self::spawn_with_sink(config, load, move |event| {
            event_tx.try_send(event).map_err(|_| ())
        })?;
        Ok((worker, event_rx))
    }

    /// Starts the worker with a caller-supplied sink. The sink must not block:
    /// it runs on the inference thread. It returns `Err(())` when it dropped the
    /// event, which the worker counts.
    pub fn spawn_with_sink<L, S>(
        config: SttWorkerConfig,
        load: L,
        sink: S,
    ) -> Result<Self, WorkerError>
    where
        L: FnOnce() -> Result<Box<dyn SttEngine>, SttError> + Send + 'static,
        S: FnMut(SttEvent) -> Result<(), ()> + Send + 'static,
    {
        if config.input_capacity == 0 || config.feed_chunk_samples == 0 {
            return Err(WorkerError::ZeroCapacity);
        }
        let (tx, rx) = mpsc::sync_channel(config.input_capacity);
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("stt".to_owned())
            .spawn(move || run(config, load, sink, rx, worker_shared))
            .map_err(|source| WorkerError::Spawn {
                name: "stt",
                source,
            })?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            shared,
        })
    }

    /// Queues an utterance without blocking.
    pub fn submit(&self, job: SttJob) -> Result<(), SubmitError> {
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

    /// Cancels the utterance being decoded, if any. Queued jobs keep their own
    /// flags and are not touched.
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

    pub fn stats(&self) -> SttWorkerStats {
        let s = &self.shared;
        SttWorkerStats {
            submitted: s.submitted.load(Ordering::Relaxed),
            rejected_full: s.rejected_full.load(Ordering::Relaxed),
            transcribed: s.transcribed.load(Ordering::Relaxed),
            failed: s.failed.load(Ordering::Relaxed),
            cancelled: s.cancelled.load(Ordering::Relaxed),
            events_dropped: s.events_dropped.load(Ordering::Relaxed),
        }
    }

    /// Stops the worker: the job in flight is cancelled, queued jobs end as
    /// `Cancelled`, then the thread is joined. Blocks until the engine returns
    /// from the call it is in, which the cancel flag shortens.
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
            tracing::error!("the stt worker thread panicked");
        }
    }
}

impl Drop for SttWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run<S>(
    config: SttWorkerConfig,
    load: impl FnOnce() -> Result<Box<dyn SttEngine>, SttError>,
    mut sink: S,
    rx: Receiver<SttJob>,
    shared: Arc<Shared>,
) where
    S: FnMut(SttEvent) -> Result<(), ()>,
{
    let mut emit = |event: SttEvent| {
        if sink(event).is_err() {
            shared.events_dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("an stt event was dropped because the consumer did not take it");
        }
    };

    let load_started = Instant::now();
    let mut engine = match load() {
        Ok(engine) => engine,
        Err(error) => {
            tracing::warn!(%error, "the stt engine did not load");
            emit(SttEvent::LoadFailed { error });
            return;
        }
    };
    let load_ms = elapsed_ms(load_started);

    let warm_started = Instant::now();
    if let Err(error) = warm_up(engine.as_mut()) {
        tracing::warn!(%error, "the stt engine failed its warm-up");
        emit(SttEvent::LoadFailed { error });
        return;
    }
    let info = engine.info();
    emit(SttEvent::Ready {
        info: info.clone(),
        load_ms,
        warm_up_ms: elapsed_ms(warm_started),
    });

    while let Ok(job) = rx.recv() {
        let utterance_id = job.utterance_id;
        if shared.stopping.load(Ordering::Acquire) || job.cancel.is_cancelled() {
            shared.cancelled.fetch_add(1, Ordering::Relaxed);
            emit(SttEvent::Cancelled { utterance_id });
            continue;
        }
        set_current(&shared, Some(job.cancel.clone()));
        // A stop that arrived between the check above and the line before.
        if shared.stopping.load(Ordering::Acquire) {
            job.cancel.cancel();
        }
        let event = process(engine.as_mut(), &info, &job, &config);
        set_current(&shared, None);
        match &event {
            SttEvent::Transcript { .. } => shared.transcribed.fetch_add(1, Ordering::Relaxed),
            SttEvent::Cancelled { .. } => shared.cancelled.fetch_add(1, Ordering::Relaxed),
            _ => shared.failed.fetch_add(1, Ordering::Relaxed),
        };
        emit(event);
    }
    emit(SttEvent::Stopped);
}

fn set_current(shared: &Shared, flag: Option<CancelFlag>) {
    *shared
        .current
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = flag;
}

fn warm_up(engine: &mut dyn SttEngine) -> Result<(), SttError> {
    engine.start()?;
    engine.feed(&vec![0.0; WARM_UP_SAMPLES])?;
    // The transcript of silence is discarded: only the first-call cost matters.
    engine.finish(&CancelFlag::new()).map(|_| ())
}

fn process(
    engine: &mut dyn SttEngine,
    info: &EngineInfo,
    job: &SttJob,
    config: &SttWorkerConfig,
) -> SttEvent {
    let utterance_id = job.utterance_id;
    let started = Instant::now();
    let result = (|| {
        engine.start()?;
        for chunk in job.samples.chunks(config.feed_chunk_samples) {
            if job.cancel.is_cancelled() {
                return Err(SttError::Cancelled);
            }
            engine.feed(chunk)?;
        }
        let finalise_started = Instant::now();
        let transcript = engine.finish(&job.cancel)?;
        Ok((transcript, elapsed_ms(finalise_started)))
    })();
    match result {
        Ok((transcript, finalise_ms)) => SttEvent::Transcript {
            utterance_id,
            transcript,
            info: info.clone(),
            finalise_ms,
            total_ms: elapsed_ms(started),
        },
        Err(SttError::Cancelled) => SttEvent::Cancelled { utterance_id },
        Err(error) => {
            // The error text comes from the engine, not from the learner's words.
            tracing::warn!(utterance_id, %error, "stt failed for an utterance");
            SttEvent::Failed {
                utterance_id,
                error,
            }
        }
    }
}

fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Start,
        Feed(usize),
        Finish,
    }

    /// Test double. `gate` makes `finish` wait for a release, `fail_on` makes the
    /// n-th `finish` fail, and a cancel flag ends a gated wait early.
    struct FakeStt {
        calls: Arc<Mutex<Vec<Call>>>,
        gate: Option<Receiver<()>>,
        fail_finish_at: Option<usize>,
        finishes: usize,
        fail_warm_up: bool,
        fed: usize,
    }

    impl FakeStt {
        fn new(calls: Arc<Mutex<Vec<Call>>>) -> Self {
            Self {
                calls,
                gate: None,
                fail_finish_at: None,
                finishes: 0,
                fail_warm_up: false,
                fed: 0,
            }
        }
        fn log(&self, call: Call) {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(call);
        }
    }

    impl SttEngine for FakeStt {
        fn start(&mut self) -> Result<(), SttError> {
            self.log(Call::Start);
            self.fed = 0;
            Ok(())
        }
        fn feed(&mut self, samples: &[f32]) -> Result<(), SttError> {
            self.log(Call::Feed(samples.len()));
            self.fed += samples.len();
            Ok(())
        }
        fn finish(&mut self, cancel: &CancelFlag) -> Result<Transcript, SttError> {
            self.log(Call::Finish);
            self.finishes += 1;
            if self.fail_warm_up && self.finishes == 1 {
                return Err(SttError::Engine("warm-up broke".into()));
            }
            if self.fail_finish_at == Some(self.finishes) {
                return Err(SttError::Engine("decode broke".into()));
            }
            if let Some(gate) = &self.gate {
                loop {
                    if cancel.is_cancelled() {
                        return Err(SttError::Cancelled);
                    }
                    match gate.recv_timeout(Duration::from_millis(5)) {
                        Ok(()) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            }
            Ok(Transcript {
                text: format!("heard {} samples", self.fed),
                words: vec![],
                duration_ms: (self.fed * 1000 / 16_000) as u32,
            })
        }
        fn info(&self) -> EngineInfo {
            EngineInfo::new("fake-stt", "test")
        }
    }

    fn job(id: u64, samples: usize) -> SttJob {
        SttJob {
            utterance_id: id,
            samples: vec![0.1; samples],
            cancel: CancelFlag::new(),
        }
    }

    fn calls(log: &Arc<Mutex<Vec<Call>>>) -> Vec<Call> {
        log.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn start_worker(config: SttWorkerConfig, engine: FakeStt) -> (SttWorker, Receiver<SttEvent>) {
        SttWorker::spawn(config, move || Ok(Box::new(engine) as Box<dyn SttEngine>))
            .expect("worker starts")
    }

    fn next(rx: &Receiver<SttEvent>) -> SttEvent {
        rx.recv_timeout(WAIT).expect("an event within the wait")
    }

    #[test]
    fn warm_up_runs_before_ready_and_before_any_job() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (worker, rx) = start_worker(SttWorkerConfig::default(), FakeStt::new(log.clone()));
        let SttEvent::Ready { info, .. } = next(&rx) else {
            panic!("the first event must be Ready");
        };
        assert_eq!(info.id, "fake-stt");
        // The warm-up fed one second of silence in a single call and finished.
        assert_eq!(
            calls(&log),
            vec![Call::Start, Call::Feed(16_000), Call::Finish]
        );
        drop(worker);
    }

    #[test]
    fn one_transcript_event_per_utterance_in_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let config = SttWorkerConfig {
            feed_chunk_samples: 1000,
            ..SttWorkerConfig::default()
        };
        let (worker, rx) = start_worker(config, FakeStt::new(log.clone()));
        assert!(matches!(next(&rx), SttEvent::Ready { .. }));
        for id in 1..=3 {
            worker.submit(job(id, 2500)).expect("queued");
            let SttEvent::Transcript {
                utterance_id,
                transcript,
                info,
                ..
            } = next(&rx)
            else {
                panic!("expected a transcript for {id}");
            };
            assert_eq!(utterance_id, id);
            assert_eq!(transcript.text, "heard 2500 samples");
            assert_eq!(info.id, "fake-stt");
        }
        // Audio went to the engine in chunks, so a streaming engine can decode
        // while the rest arrives.
        let all = calls(&log);
        let first_job: Vec<Call> = all[3..8].to_vec();
        assert_eq!(
            first_job,
            vec![
                Call::Start,
                Call::Feed(1000),
                Call::Feed(1000),
                Call::Feed(500),
                Call::Finish
            ]
        );
        assert_eq!(worker.stats().transcribed, 3);
        worker.shutdown();
        assert!(matches!(next(&rx), SttEvent::Stopped));
    }

    #[test]
    fn a_full_queue_hands_the_job_back_and_counts_it() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (release, gate) = mpsc::channel();
        let mut engine = FakeStt::new(log.clone());
        // The warm-up passes the gate too, so release it once before the job.
        engine.gate = Some(gate);
        let config = SttWorkerConfig {
            input_capacity: 1,
            ..SttWorkerConfig::default()
        };
        let (worker, rx) = start_worker(config, engine);
        release.send(()).expect("release the warm-up");
        assert!(matches!(next(&rx), SttEvent::Ready { .. }));

        worker.submit(job(1, 100)).expect("taken by the worker");
        // Wait until job 1 is inside `finish` so the queue is empty again.
        wait_for(|| calls(&log).len() >= 3 + 3);
        worker.submit(job(2, 100)).expect("fills the queue of one");
        let SubmitError::Full(returned) = worker.submit(job(3, 100)).expect_err("queue is full")
        else {
            panic!("expected Full");
        };
        assert_eq!(returned.utterance_id, 3);
        assert_eq!(worker.stats().rejected_full, 1);
        assert_eq!(worker.stats().submitted, 2);

        release.send(()).expect("release job 1");
        release.send(()).expect("release job 2");
        let ids: Vec<u64> = (0..2)
            .map(|_| match next(&rx) {
                SttEvent::Transcript { utterance_id, .. } => utterance_id,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(ids, vec![1, 2]);
        // Job 3 produced no event: it never entered the worker.
        worker.shutdown();
        assert!(matches!(next(&rx), SttEvent::Stopped));
    }

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !condition() {
            assert!(Instant::now() < deadline, "condition not reached in time");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_job_cancelled_before_it_starts_never_reaches_the_engine() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (worker, rx) = start_worker(SttWorkerConfig::default(), FakeStt::new(log.clone()));
        assert!(matches!(next(&rx), SttEvent::Ready { .. }));
        let cancelled = job(7, 500);
        cancelled.cancel.cancel();
        worker.submit(cancelled).expect("queued");
        assert!(matches!(next(&rx), SttEvent::Cancelled { utterance_id: 7 }));
        // Only the warm-up calls exist.
        assert_eq!(calls(&log).len(), 3);
        assert_eq!(worker.stats().cancelled, 1);
    }

    #[test]
    fn cancelling_during_decode_ends_that_job_and_the_next_still_works() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (release, gate) = mpsc::channel();
        let mut engine = FakeStt::new(log.clone());
        engine.gate = Some(gate);
        let (worker, rx) = start_worker(SttWorkerConfig::default(), engine);
        release.send(()).expect("release the warm-up");
        assert!(matches!(next(&rx), SttEvent::Ready { .. }));

        worker.submit(job(1, 100)).expect("queued");
        wait_for(|| calls(&log).len() >= 3 + 3);
        worker.cancel_current();
        assert!(matches!(next(&rx), SttEvent::Cancelled { utterance_id: 1 }));

        release.send(()).expect("release job 2");
        worker.submit(job(2, 100)).expect("queued");
        assert!(matches!(
            next(&rx),
            SttEvent::Transcript {
                utterance_id: 2,
                ..
            }
        ));
    }

    #[test]
    fn an_engine_error_fails_one_job_and_the_worker_goes_on() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeStt::new(log);
        // Finish 1 is the warm-up, finish 2 is the first job.
        engine.fail_finish_at = Some(2);
        let (worker, rx) = start_worker(SttWorkerConfig::default(), engine);
        assert!(matches!(next(&rx), SttEvent::Ready { .. }));
        worker.submit(job(1, 100)).expect("queued");
        let SttEvent::Failed {
            utterance_id,
            error,
        } = next(&rx)
        else {
            panic!("expected Failed");
        };
        assert_eq!(utterance_id, 1);
        assert!(error.to_string().contains("decode broke"));
        worker.submit(job(2, 100)).expect("queued");
        assert!(matches!(
            next(&rx),
            SttEvent::Transcript {
                utterance_id: 2,
                ..
            }
        ));
        let stats = worker.stats();
        assert_eq!((stats.failed, stats.transcribed), (1, 1));
    }

    #[test]
    fn a_load_failure_is_reported_and_later_submissions_are_refused() {
        let (worker, rx) = SttWorker::spawn(SttWorkerConfig::default(), || {
            Err(SttError::Unavailable {
                reason: "no model".into(),
            })
        })
        .expect("thread starts");
        assert!(matches!(next(&rx), SttEvent::LoadFailed { .. }));
        wait_for(|| matches!(worker.submit(job(1, 10)), Err(SubmitError::Stopped(_))));
    }

    #[test]
    fn a_failed_warm_up_counts_as_a_load_failure() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut engine = FakeStt::new(log);
        engine.fail_warm_up = true;
        let (_worker, rx) = start_worker(SttWorkerConfig::default(), engine);
        let SttEvent::LoadFailed { error } = next(&rx) else {
            panic!("expected LoadFailed");
        };
        assert!(error.to_string().contains("warm-up broke"));
    }

    #[test]
    fn shutdown_cancels_the_job_in_flight_and_the_queued_ones_then_joins() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (release, gate) = mpsc::channel();
        let mut engine = FakeStt::new(log.clone());
        engine.gate = Some(gate);
        let (worker, rx) = start_worker(SttWorkerConfig::default(), engine);
        release.send(()).expect("release the warm-up");
        assert!(matches!(next(&rx), SttEvent::Ready { .. }));
        worker.submit(job(1, 100)).expect("queued");
        wait_for(|| calls(&log).len() >= 3 + 3);
        worker.submit(job(2, 100)).expect("queued");
        worker.submit(job(3, 100)).expect("queued");

        worker.shutdown();

        let mut cancelled = Vec::new();
        loop {
            match next(&rx) {
                SttEvent::Cancelled { utterance_id } => cancelled.push(utterance_id),
                SttEvent::Stopped => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(cancelled, vec![1, 2, 3]);
        // The channel is closed after Stopped.
        assert!(matches!(
            rx.recv_timeout(WAIT),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn a_full_event_channel_drops_and_counts_instead_of_blocking() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let config = SttWorkerConfig {
            event_capacity: 1,
            ..SttWorkerConfig::default()
        };
        // Nobody reads the events, so only Ready fits; each transcript is dropped.
        let (worker, _rx) = start_worker(config, FakeStt::new(log));
        for id in 1..=3 {
            worker.submit(job(id, 10)).expect("queued");
        }
        wait_for(|| worker.stats().transcribed == 3);
        assert_eq!(worker.stats().events_dropped, 3);
    }

    #[test]
    fn zero_capacities_are_refused() {
        let zero_input = SttWorkerConfig {
            input_capacity: 0,
            ..SttWorkerConfig::default()
        };
        assert!(matches!(
            SttWorker::spawn(zero_input, || Err(SttError::Engine("x".into()))),
            Err(WorkerError::ZeroCapacity)
        ));
        let zero_events = SttWorkerConfig {
            event_capacity: 0,
            ..SttWorkerConfig::default()
        };
        assert!(matches!(
            SttWorker::spawn(zero_events, || Err(SttError::Engine("x".into()))),
            Err(WorkerError::ZeroCapacity)
        ));
    }

    #[test]
    fn an_unavailable_engine_reports_through_the_worker() {
        let (_worker, rx) = SttWorker::spawn(SttWorkerConfig::default(), || {
            Ok(
                Box::new(crate::UnavailableEngine::new("built without sherpa"))
                    as Box<dyn SttEngine>,
            )
        })
        .expect("thread starts");
        let SttEvent::LoadFailed { error } = next(&rx) else {
            panic!("expected LoadFailed");
        };
        assert!(matches!(error, SttError::Unavailable { .. }));
    }
}
