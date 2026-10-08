//! The half-duplex microphone gate.
//!
//! While tutor audio plays, and for 150 ms after it stops, the microphone would
//! pick up the tutor's own voice. The gate sits in front of the VAD and tells the
//! worker to drop those frames. Push-to-talk is the explicit exception: while the
//! learner holds the key, frames go to the push-to-talk path instead, whatever
//! the playback state is, and the VAD never sees them.
//!
//! Time is counted in captured samples, not wall-clock time. The hold-off is a
//! property of the recording ("do not use the next 150 ms of audio"), so it stays
//! correct when the worker runs late or a test runs faster than real time.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::playback::PlaybackQueue;

/// How long the gate stays closed after playback ends.
pub const HOLD_AFTER_PLAYBACK: Duration = Duration::from_millis(150);

/// Anything that can say whether tutor audio is being played right now.
pub trait PlaybackActivity: Send + Sync {
    fn is_playing(&self) -> bool;
}

impl PlaybackActivity for PlaybackQueue {
    fn is_playing(&self) -> bool {
        self.is_active()
    }
}

impl PlaybackActivity for AtomicBool {
    fn is_playing(&self) -> bool {
        self.load(Ordering::Acquire)
    }
}

/// The push-to-talk key state. Clones share one flag, so the API handler that
/// sees the key and the capture worker that reads it can live on different
/// threads.
#[derive(Debug, Clone, Default)]
pub struct PushToTalk(Arc<AtomicBool>);

impl PushToTalk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, pressed: bool) {
        self.0.store(pressed, Ordering::Release);
    }

    pub fn is_pressed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Where a frame goes after the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Normal listening: the frame goes to the VAD and endpointing.
    Vad,
    /// The learner is holding push-to-talk: the frame belongs to the utterance
    /// that ends when the key is released. It bypasses the VAD and the playback
    /// gate.
    PushToTalk,
    /// Playback, or the tail after it: the frame is thrown away.
    Dropped,
}

#[derive(Debug, Default)]
struct Counters {
    vad: AtomicU64,
    push_to_talk: AtomicU64,
    dropped: AtomicU64,
}

/// How many frames went each way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateStats {
    pub to_vad: u64,
    pub push_to_talk: u64,
    pub dropped: u64,
}

/// A cloneable read-only view of the gate counters.
#[derive(Debug, Clone)]
pub struct GateCounters(Arc<Counters>);

impl GateCounters {
    pub fn snapshot(&self) -> GateStats {
        GateStats {
            to_vad: self.0.vad.load(Ordering::Relaxed),
            push_to_talk: self.0.push_to_talk.load(Ordering::Relaxed),
            dropped: self.0.dropped.load(Ordering::Relaxed),
        }
    }
}

/// Decides, frame by frame, whether the VAD may hear the microphone.
pub struct MicGate {
    activity: Arc<dyn PlaybackActivity>,
    push_to_talk: PushToTalk,
    hold_samples: u64,
    hold_left: u64,
    counters: Arc<Counters>,
}

impl std::fmt::Debug for MicGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicGate")
            .field("hold_left", &self.hold_left)
            .finish_non_exhaustive()
    }
}

impl MicGate {
    /// `sample_rate` is the rate of the frames passed to [`route`](Self::route),
    /// which is 16 kHz on the capture path.
    pub fn new(
        activity: Arc<dyn PlaybackActivity>,
        push_to_talk: PushToTalk,
        sample_rate: u32,
    ) -> Self {
        let hold_samples = HOLD_AFTER_PLAYBACK.as_millis() as u64 * u64::from(sample_rate) / 1000;
        Self {
            activity,
            push_to_talk,
            hold_samples,
            hold_left: 0,
            counters: Arc::new(Counters::default()),
        }
    }

    pub fn counters(&self) -> GateCounters {
        GateCounters(Arc::clone(&self.counters))
    }

    /// Routes one frame of `frame_len` samples.
    ///
    /// The hold-off starts at the first frame that is seen without playback and
    /// lasts at least 150 ms, rounded up to whole frames.
    pub fn route(&mut self, frame_len: usize) -> Route {
        let playing = self.activity.is_playing();
        let blocked = if playing {
            self.hold_left = self.hold_samples;
            true
        } else if self.hold_left > 0 {
            self.hold_left = self.hold_left.saturating_sub(frame_len as u64);
            true
        } else {
            false
        };

        let (route, counter) = if self.push_to_talk.is_pressed() {
            (Route::PushToTalk, &self.counters.push_to_talk)
        } else if blocked {
            (Route::Dropped, &self.counters.dropped)
        } else {
            (Route::Vad, &self.counters.vad)
        };
        counter.fetch_add(1, Ordering::Relaxed);
        route
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: usize = 160; // 10 ms at 16 kHz

    fn gate() -> (MicGate, Arc<AtomicBool>, PushToTalk) {
        let playing = Arc::new(AtomicBool::new(false));
        let ptt = PushToTalk::new();
        let gate = MicGate::new(playing.clone(), ptt.clone(), 16_000);
        (gate, playing, ptt)
    }

    fn run(gate: &mut MicGate, frames: usize) -> Vec<Route> {
        (0..frames).map(|_| gate.route(FRAME)).collect()
    }

    #[test]
    fn frames_pass_to_the_vad_when_nothing_is_playing() {
        let (mut gate, _playing, _ptt) = gate();
        assert!(run(&mut gate, 20).iter().all(|r| *r == Route::Vad));
    }

    #[test]
    fn frames_drop_while_playing_and_for_150_ms_after() {
        // Table: (frames with playback on, frames after it ends, expected tail drops).
        for (during, after) in [(1, 40), (10, 40), (200, 40)] {
            let (mut gate, playing, _ptt) = gate();
            playing.store(true, Ordering::Release);
            assert!(run(&mut gate, during).iter().all(|r| *r == Route::Dropped));
            playing.store(false, Ordering::Release);
            let tail = run(&mut gate, after);
            let dropped = tail.iter().take_while(|r| **r == Route::Dropped).count();
            assert_eq!(dropped, 15, "150 ms of 10 ms frames after {during} frames");
            assert!(tail[dropped..].iter().all(|r| *r == Route::Vad));
        }
    }

    #[test]
    fn a_larger_frame_rounds_the_hold_up_never_down() {
        let (mut gate, playing, _ptt) = gate();
        playing.store(true, Ordering::Release);
        gate.route(512);
        playing.store(false, Ordering::Release);
        // 150 ms is 2400 samples; 512-sample frames need five of them.
        let tail: Vec<Route> = (0..8).map(|_| gate.route(512)).collect();
        assert_eq!(tail.iter().take_while(|r| **r == Route::Dropped).count(), 5);
    }

    #[test]
    fn playback_restarting_inside_the_hold_restarts_the_hold() {
        let (mut gate, playing, _ptt) = gate();
        playing.store(true, Ordering::Release);
        run(&mut gate, 3);
        playing.store(false, Ordering::Release);
        run(&mut gate, 10); // 100 ms into the hold
        playing.store(true, Ordering::Release);
        run(&mut gate, 3);
        playing.store(false, Ordering::Release);
        let tail = run(&mut gate, 30);
        assert_eq!(
            tail.iter().take_while(|r| **r == Route::Dropped).count(),
            15
        );
    }

    #[test]
    fn push_to_talk_bypasses_the_gate_and_the_vad() {
        let (mut gate, playing, ptt) = gate();
        playing.store(true, Ordering::Release);
        ptt.set(true);
        assert!(run(&mut gate, 5).iter().all(|r| *r == Route::PushToTalk));
        ptt.set(false);
        // Released during playback: the gate applies again.
        assert!(run(&mut gate, 5).iter().all(|r| *r == Route::Dropped));
    }

    #[test]
    fn counters_add_up_to_the_frames_seen() {
        let (mut gate, playing, ptt) = gate();
        let counters = gate.counters();
        run(&mut gate, 4);
        playing.store(true, Ordering::Release);
        run(&mut gate, 3);
        ptt.set(true);
        run(&mut gate, 2);
        let stats = counters.snapshot();
        assert_eq!(
            stats,
            GateStats {
                to_vad: 4,
                push_to_talk: 2,
                dropped: 3
            }
        );
    }
}
