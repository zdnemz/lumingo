#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! S2-08: no thread or stream survives a stopped session.
//!
//! This file holds a single test on purpose. It counts the threads and file
//! handles of the whole test process, so no other test may run beside it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use audio_io::fake::FakeBackend;
use audio_io::{AudioSession, DeviceRegistry, Direction, MemoryPrefs, SessionConfig, StreamFormat};
use speech::PcmChunk;

const CYCLES: usize = 100;

/// Threads in this process. Linux only: the other platforms rely on the fake
/// backend's own counters and on the join counts below.
#[cfg(target_os = "linux")]
fn thread_count() -> Option<usize> {
    std::fs::read_dir("/proc/self/task")
        .ok()
        .map(Iterator::count)
}

#[cfg(not(target_os = "linux"))]
fn thread_count() -> Option<usize> {
    None
}

/// Open file handles of this process (Linux).
#[cfg(target_os = "linux")]
fn handle_count() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd").ok().map(Iterator::count)
}

#[cfg(not(target_os = "linux"))]
fn handle_count() -> Option<usize> {
    None
}

fn chunk() -> PcmChunk {
    PcmChunk {
        samples: vec![0.2; 2_400],
        sample_rate: 24_000,
    }
}

#[test]
fn a_hundred_sessions_start_and_stop_without_leaking_threads_streams_or_handles() {
    let backend = FakeBackend::with_period(Duration::from_millis(2));
    backend.add_device("Mic", Direction::Input, StreamFormat::new(48_000, 2), true);
    backend.add_device(
        "Speakers",
        Direction::Output,
        StreamFormat::new(48_000, 2),
        true,
    );
    backend.set_input_tone(Some(440.0));
    let registry = Arc::new(DeviceRegistry::new(
        Arc::new(backend.clone()),
        Box::new(MemoryPrefs::default()),
    ));

    // One warm-up cycle so lazily created process-wide state (thread-local
    // storage, the first /proc reads) is not counted as a leak.
    run_cycle(&registry, 7, &AtomicUsize::new(0));
    let threads_before = thread_count();
    let handles_before = handle_count();

    let worker_runs = AtomicUsize::new(0);
    for cycle in 1..=CYCLES {
        run_cycle(&registry, cycle, &worker_runs);
        assert_eq!(backend.live_streams(), 0, "streams after cycle {cycle}");
        assert_eq!(
            backend.live_threads(),
            0,
            "device threads after cycle {cycle}"
        );
    }

    assert_eq!(
        worker_runs.load(Ordering::SeqCst),
        CYCLES,
        "every extra worker started and finished"
    );
    assert_eq!(
        backend.opened_total(),
        2 * (CYCLES + 1) + recoveries(CYCLES)
    );
    assert_eq!(thread_count(), threads_before, "thread count");
    assert_eq!(handle_count(), handles_before, "open handle count");
}

/// Every tenth cycle also replaces the input stream once, so recovery's streams
/// are part of what must not leak.
fn recoveries(cycles: usize) -> usize {
    (1..=cycles).filter(|c| c % 10 == 0).count()
}

fn run_cycle(registry: &Arc<DeviceRegistry>, cycle: usize, worker_runs: &AtomicUsize) {
    let frames = Arc::new(AtomicUsize::new(0));
    let sink_frames = Arc::clone(&frames);
    let mut session = AudioSession::start(
        Arc::clone(registry),
        SessionConfig::default(),
        Box::new(move |_, _| {
            sink_frames.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .expect("session starts");

    // A downstream worker, like the VAD or STT thread of a later stage. It waits
    // on the cancel flag the way a real worker does.
    let started = Arc::new(AtomicUsize::new(0));
    let worker_started = Arc::clone(&started);
    let finished = Arc::new(AtomicUsize::new(0));
    let worker_finished = Arc::clone(&finished);
    session
        .spawn_worker("test-worker", move |cancel| {
            worker_started.fetch_add(1, Ordering::SeqCst);
            while !cancel.is_cancelled() {
                std::thread::park_timeout(Duration::from_millis(50));
            }
            worker_finished.fetch_add(1, Ordering::SeqCst);
        })
        .expect("worker starts");

    session.playback().enqueue(&chunk()).expect("queued");
    if cycle % 10 == 0 {
        session.recover(Direction::Input).expect("recovers");
    }
    // Let a little real work happen in about half of the cycles.
    if cycle % 2 == 0 {
        std::thread::sleep(Duration::from_millis(4));
    }

    match cycle % 3 {
        // Stop waits for everything and reports what it joined.
        0 => {
            let report = session.stop();
            assert_eq!(report.workers_joined, 2, "capture worker and test worker");
            assert_eq!(report.worker_panics, 0);
        }
        // Cancel reaches every worker without anyone calling stop; drop joins.
        1 => {
            session.cancel();
            drop(session);
        }
        // Plain drop.
        _ => drop(session),
    }
    // Even a worker that was never scheduled before the stop still ran its body
    // and ended: shutdown joined it.
    assert_eq!(started.load(Ordering::SeqCst), 1, "cycle {cycle}");
    assert_eq!(finished.load(Ordering::SeqCst), 1, "cycle {cycle}");
    worker_runs.fetch_add(finished.load(Ordering::SeqCst), Ordering::SeqCst);
}
