//! The playback queue.
//!
//! TTS chunks arrive at the engine's rate. [`PlaybackQueue::enqueue`] converts
//! each one to the device rate and channel count on the caller's thread and
//! copies it into a lock-free ring. The output callback owns a
//! [`PlaybackSource`] and only reads from that ring: no lock, no allocation, no
//! logging.
//!
//! Stopping works without touching the callback's end of the ring. `stop` records
//! how many samples had been queued at that moment, and the callback discards
//! everything up to that position at the start of its next buffer. The flush is
//! therefore done within one device period, which is far below the 50 ms target.
//! Samples queued after `stop` sit past that position and are kept.
//!
//! An underrun is the callback running dry while a turn is still open, meaning
//! the producer has queued audio and has not yet said the turn is complete. Call
//! [`PlaybackQueue::finish_turn`] after the last chunk of a reply. Running dry
//! after that is a normal end, not an underrun.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use speech::PcmChunk;

use crate::format::StreamFormat;
use crate::resample::{MonoResampler, ResampleError};
use crate::sync::lock;

/// Why audio could not be queued.
#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    #[error("the output format has no samples ({0:?})")]
    EmptyFormat(StreamFormat),
    #[error("the playback queue must hold some audio")]
    ZeroCapacity,
    #[error("a chunk reports a sample rate of 0 Hz")]
    ChunkRate,
    #[error(
        "the playback queue is full: {needed_ms} ms do not fit into {free_ms} ms of free space"
    )]
    QueueFull { needed_ms: u64, free_ms: u64 },
    #[error("converting a chunk to the device rate failed: {0}")]
    Convert(#[from] ResampleError),
}

/// What happened to a chunk handed to [`PlaybackQueue::enqueue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Queued,
    /// `stop` was called while the chunk was being converted, so it was dropped.
    DiscardedByStop,
}

/// Counters for the output side. Zero underruns is the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaybackStats {
    pub underruns: u64,
    /// Interleaved device samples handed to the output stream as real audio.
    pub samples_played: u64,
    /// Interleaved device samples waiting in the queue.
    pub samples_queued: u64,
}

/// A turn that has no known end yet.
const TURN_OPEN: u64 = u64::MAX;

#[derive(Debug)]
struct Shared {
    /// Samples ever queued. Written only while holding the producer lock.
    pushed: AtomicU64,
    /// Samples the callback has played or discarded.
    consumed: AtomicU64,
    /// The callback discards everything before this position.
    flush_to: AtomicU64,
    /// Position at which the current turn ends, or [`TURN_OPEN`].
    turn_end: AtomicU64,
    underruns: AtomicU64,
    played: AtomicU64,
}

struct Converter {
    resampler: Option<MonoResampler>,
    scratch: Vec<f32>,
}

struct Inner {
    format: StreamFormat,
    shared: Shared,
    stop_generation: AtomicU64,
    /// Held from conversion to push so chunks keep the order of the calls.
    converter: Mutex<Converter>,
    producer: Mutex<rtrb::Producer<f32>>,
}

/// The producer side of the playback queue. Cheap to clone; all clones feed the
/// same queue.
#[derive(Clone)]
pub struct PlaybackQueue {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PlaybackQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackQueue")
            .field("format", &self.inner.format)
            .finish_non_exhaustive()
    }
}

/// The output-callback side of the playback queue.
pub struct PlaybackSource {
    consumer: rtrb::Consumer<f32>,
    inner: Arc<Inner>,
    consumed: u64,
    in_turn: bool,
}

impl std::fmt::Debug for PlaybackSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackSource")
            .field("format", &self.inner.format)
            .finish_non_exhaustive()
    }
}

/// Creates a queue for an output stream with `format` that holds up to
/// `capacity` of audio. When it is full, `enqueue` fails with
/// [`PlaybackError::QueueFull`] and nothing is dropped silently.
pub fn playback_queue(
    format: StreamFormat,
    capacity: Duration,
) -> Result<(PlaybackQueue, PlaybackSource), PlaybackError> {
    if format.sample_rate == 0 || format.channels == 0 {
        return Err(PlaybackError::EmptyFormat(format));
    }
    let samples = format.samples_per_second() * capacity.as_millis() as u64 / 1000;
    let samples = usize::try_from(samples).map_err(|_| PlaybackError::ZeroCapacity)?;
    if samples < usize::from(format.channels) {
        return Err(PlaybackError::ZeroCapacity);
    }
    let (producer, consumer) = rtrb::RingBuffer::new(samples);
    let inner = Arc::new(Inner {
        format,
        shared: Shared {
            pushed: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            flush_to: AtomicU64::new(0),
            turn_end: AtomicU64::new(TURN_OPEN),
            underruns: AtomicU64::new(0),
            played: AtomicU64::new(0),
        },
        stop_generation: AtomicU64::new(0),
        converter: Mutex::new(Converter {
            resampler: None,
            scratch: Vec::new(),
        }),
        producer: Mutex::new(producer),
    });
    Ok((
        PlaybackQueue {
            inner: Arc::clone(&inner),
        },
        PlaybackSource {
            consumer,
            inner,
            consumed: 0,
            in_turn: false,
        },
    ))
}

impl PlaybackQueue {
    pub fn format(&self) -> StreamFormat {
        self.inner.format
    }

    /// Converts `chunk` to the device rate and channel count and queues it behind
    /// everything queued before. All of the chunk fits or none of it is queued.
    pub fn enqueue(&self, chunk: &PcmChunk) -> Result<EnqueueOutcome, PlaybackError> {
        if chunk.sample_rate == 0 {
            return Err(PlaybackError::ChunkRate);
        }
        let generation = self.inner.stop_generation.load(Ordering::Acquire);
        let device = self.inner.format;
        let channels = usize::from(device.channels);
        let mut converter = lock(&self.inner.converter);
        let Converter { resampler, scratch } = &mut *converter;

        let reusable = resampler
            .as_ref()
            .is_some_and(|r| r.in_rate() == chunk.sample_rate);
        if !reusable {
            *resampler = Some(MonoResampler::new(chunk.sample_rate, device.sample_rate)?);
        }
        let Some(resampler) = resampler.as_mut() else {
            return Err(PlaybackError::ChunkRate);
        };
        let mono = resampler.convert_clip(&chunk.samples)?;

        scratch.clear();
        scratch.reserve(mono.len() * channels);
        for sample in mono {
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            scratch.extend(std::iter::repeat_n(sample, channels));
        }

        let mut producer = lock(&self.inner.producer);
        if self.inner.stop_generation.load(Ordering::Acquire) != generation {
            return Ok(EnqueueOutcome::DiscardedByStop);
        }
        let free = producer.slots();
        if scratch.len() > free {
            let per_ms = (device.samples_per_second() / 1000).max(1);
            return Err(PlaybackError::QueueFull {
                needed_ms: scratch.len() as u64 / per_ms,
                free_ms: free as u64 / per_ms,
            });
        }
        // The turn is marked open before the samples become visible, so the
        // callback can never see audio that belongs to a turn it thinks has ended.
        self.inner
            .shared
            .turn_end
            .store(TURN_OPEN, Ordering::Release);
        // Cannot fail: this thread is the only producer and `free` only grows.
        let _ = producer.push_entire_slice(scratch);
        self.inner
            .shared
            .pushed
            .fetch_add(scratch.len() as u64, Ordering::Release);
        Ok(EnqueueOutcome::Queued)
    }

    /// Says that everything queued so far is the end of the reply. Running out of
    /// audio after this point is not counted as an underrun.
    pub fn finish_turn(&self) {
        let _producer = lock(&self.inner.producer);
        let pushed = self.inner.shared.pushed.load(Ordering::Acquire);
        self.inner.shared.turn_end.store(pushed, Ordering::Release);
    }

    /// Drops everything queued and everything being converted. The output goes
    /// quiet at the start of the callback's next buffer.
    pub fn stop(&self) {
        let _producer = lock(&self.inner.producer);
        self.inner.stop_generation.fetch_add(1, Ordering::AcqRel);
        let pushed = self.inner.shared.pushed.load(Ordering::Acquire);
        // Order matters: the callback reads `flush_to` first, so once it sees the
        // new value the closed turn is already visible too.
        self.inner.shared.turn_end.store(pushed, Ordering::Release);
        self.inner.shared.flush_to.store(pushed, Ordering::Release);
    }

    /// True while there is queued audio that has not been played or flushed. The
    /// microphone gate reads this.
    pub fn is_active(&self) -> bool {
        let shared = &self.inner.shared;
        let pushed = shared.pushed.load(Ordering::Acquire);
        let done = shared
            .consumed
            .load(Ordering::Acquire)
            .max(shared.flush_to.load(Ordering::Acquire));
        pushed > done
    }

    /// Audio waiting in the queue, as a duration.
    pub fn queued(&self) -> Duration {
        let stats = self.stats();
        let per_second = self.inner.format.samples_per_second().max(1);
        Duration::from_micros(stats.samples_queued * 1_000_000 / per_second)
    }

    pub fn stats(&self) -> PlaybackStats {
        let shared = &self.inner.shared;
        let pushed = shared.pushed.load(Ordering::Acquire);
        let done = shared
            .consumed
            .load(Ordering::Acquire)
            .max(shared.flush_to.load(Ordering::Acquire));
        PlaybackStats {
            underruns: shared.underruns.load(Ordering::Relaxed),
            samples_played: shared.played.load(Ordering::Relaxed),
            samples_queued: pushed.saturating_sub(done),
        }
    }
}

impl PlaybackSource {
    pub fn format(&self) -> StreamFormat {
        self.inner.format
    }

    /// Fills one output buffer of interleaved device samples. Queued audio comes
    /// first; the rest of the buffer is silence. Call this from the output
    /// callback and nothing else: it never locks, allocates or logs.
    pub fn fill(&mut self, out: &mut [f32]) {
        let shared = &self.inner.shared;

        let flush_to = shared.flush_to.load(Ordering::Acquire);
        if self.consumed < flush_to {
            let wanted = usize::try_from(flush_to - self.consumed).unwrap_or(usize::MAX);
            let take = wanted.min(self.consumer.slots());
            if let Ok(chunk) = self.consumer.read_chunk(take) {
                chunk.commit_all();
            }
            self.consumed += take as u64;
        }

        let (got, rest) = self.consumer.pop_partial_slice(out);
        let got = got.len();
        rest.fill(0.0);
        self.consumed += got as u64;
        shared.consumed.store(self.consumed, Ordering::Release);
        if got > 0 {
            shared.played.fetch_add(got as u64, Ordering::Relaxed);
            self.in_turn = true;
        }

        if got < out.len() && self.in_turn {
            // Dry. It is a normal end when the producer closed the turn at or
            // before this position, and an underrun when it did not.
            self.in_turn = false;
            if self.consumed < shared.turn_end.load(Ordering::Acquire) {
                shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;
    use std::time::Instant;

    const DEVICE: StreamFormat = StreamFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    /// 10 ms of stereo at 48 kHz, a typical device period.
    const PERIOD: usize = 960;

    fn queue() -> (PlaybackQueue, PlaybackSource) {
        playback_queue(DEVICE, Duration::from_secs(30)).expect("queue builds")
    }

    fn tone(rate: u32, ms: u32, hz: f32) -> PcmChunk {
        let n = (u64::from(rate) * u64::from(ms) / 1000) as usize;
        PcmChunk {
            samples: (0..n)
                .map(|i| 0.5 * (2.0 * PI * hz * i as f32 / rate as f32).sin())
                .collect(),
            sample_rate: rate,
        }
    }

    /// Runs the callback until it has nothing left, returning all output.
    fn drain(source: &mut PlaybackSource, queue: &PlaybackQueue) -> Vec<f32> {
        let mut all = Vec::new();
        let mut buf = vec![0.0; PERIOD];
        while queue.is_active() {
            source.fill(&mut buf);
            all.extend_from_slice(&buf);
        }
        all
    }

    fn goertzel(x: &[f32], rate: u32, hz: f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * f64::from(hz) / f64::from(rate);
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0_f64, 0.0_f64);
        for &s in x {
            let s0 = f64::from(s) + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (2.0 * (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / x.len() as f64) as f32
    }

    fn left(interleaved: &[f32]) -> Vec<f32> {
        interleaved.iter().step_by(2).copied().collect()
    }

    #[test]
    fn three_chunks_are_converted_ordered_and_gapless() {
        let (q, mut source) = queue();
        // Three 100 ms sentences at the TTS rate, each a different pitch.
        for hz in [1_000.0, 2_000.0, 3_000.0] {
            assert_eq!(
                q.enqueue(&tone(24_000, 100, hz)).expect("fits"),
                EnqueueOutcome::Queued
            );
        }
        q.finish_turn();
        let out = drain(&mut source, &q);
        let mono = left(&out);
        let per_chunk = 4_800; // 100 ms at 48 kHz
        assert!(mono.len() >= 3 * per_chunk);
        // Stereo: both channels carry the same signal.
        assert!(out.chunks_exact(2).all(|f| f[0] == f[1]));
        for (i, hz) in [1_000.0, 2_000.0, 3_000.0].into_iter().enumerate() {
            // Measure the middle of each section to stay clear of the joins.
            let part = &mono[i * per_chunk + 1_000..(i + 1) * per_chunk - 1_000];
            let wanted = goertzel(part, 48_000, hz);
            assert!(
                (wanted - 0.5).abs() < 0.05,
                "section {i}: {wanted} at {hz} Hz"
            );
            for other in [1_000.0, 2_000.0, 3_000.0] {
                if other != hz {
                    assert!(
                        goertzel(part, 48_000, other) < 0.05,
                        "section {i} leaks {other}"
                    );
                }
            }
        }
        // Everything played, nothing inserted between chunks, nothing counted.
        let stats = q.stats();
        assert_eq!(stats.samples_played, 3 * per_chunk as u64 * 2);
        assert_eq!(stats.underruns, 0);
        assert!(!q.is_active());
    }

    #[test]
    fn a_chunk_at_the_device_rate_is_not_resampled() {
        let (q, mut source) = queue();
        let chunk = tone(48_000, 50, 500.0);
        q.enqueue(&chunk).expect("fits");
        q.finish_turn();
        let out = drain(&mut source, &q);
        let mono: Vec<f32> = left(&out).into_iter().take(chunk.samples.len()).collect();
        assert_eq!(mono, chunk.samples);
    }

    #[test]
    fn stop_makes_the_very_next_buffer_silent_and_empties_the_queue() {
        let (q, mut source) = queue();
        q.enqueue(&tone(24_000, 5_000, 800.0)).expect("fits");
        let mut buf = vec![0.0; PERIOD];
        source.fill(&mut buf);
        assert!(buf.iter().any(|s| *s != 0.0), "audio is playing");
        assert!(q.is_active());

        q.stop();
        assert!(!q.is_active(), "the gate sees playback end at once");
        source.fill(&mut buf);
        assert!(buf.iter().all(|s| *s == 0.0), "next buffer is silent");
        assert_eq!(q.stats().samples_queued, 0);
        assert_eq!(q.stats().underruns, 0, "a stop is not an underrun");
    }

    #[test]
    fn audio_queued_after_a_stop_is_played() {
        let (q, mut source) = queue();
        q.enqueue(&tone(24_000, 500, 800.0)).expect("fits");
        q.stop();
        q.enqueue(&tone(24_000, 100, 1_500.0)).expect("fits");
        q.finish_turn();
        drain(&mut source, &q);
        assert_eq!(q.stats().samples_played, 4_800 * 2, "only the new chunk");
    }

    #[test]
    fn a_stop_flushes_within_50_ms_of_wall_time_on_a_running_fake_device() {
        let (q, mut source) = queue();
        q.enqueue(&tone(24_000, 10_000, 600.0)).expect("fits");

        let (silent_tx, silent_rx) = std::sync::mpsc::sync_channel::<Instant>(1);
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let device_running = Arc::clone(&running);
        // The fake device asks for a 10 ms buffer every 10 ms, like a real one.
        let device = std::thread::spawn(move || {
            let mut buf = vec![0.0; PERIOD];
            let mut played_something = false;
            let mut reported = false;
            while device_running.load(Ordering::Relaxed) {
                source.fill(&mut buf);
                let silent = buf.iter().all(|s| *s == 0.0);
                played_something |= !silent;
                if played_something && silent && !reported {
                    reported = true;
                    let _ = silent_tx.try_send(Instant::now());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });

        std::thread::sleep(Duration::from_millis(100));
        let stopped_at = Instant::now();
        q.stop();
        let silent_at = silent_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the device went quiet");
        running.store(false, Ordering::Relaxed);
        device.join().expect("device thread ends");
        let took = silent_at.duration_since(stopped_at);
        assert!(took < Duration::from_millis(50), "flush took {took:?}");
    }

    #[test]
    fn running_dry_in_an_open_turn_is_one_underrun_per_gap() {
        let (q, mut source) = queue();
        q.enqueue(&tone(24_000, 15, 800.0)).expect("fits"); // 1.5 periods
        let mut buf = vec![0.0; PERIOD];
        for _ in 0..5 {
            source.fill(&mut buf);
        }
        assert_eq!(q.stats().underruns, 1, "five dry buffers are one gap");

        // The next sentence arrives late, plays, and is the end of the reply.
        q.enqueue(&tone(24_000, 15, 800.0)).expect("fits");
        q.finish_turn();
        for _ in 0..5 {
            source.fill(&mut buf);
        }
        assert_eq!(
            q.stats().underruns,
            1,
            "a finished turn ends without a count"
        );
    }

    #[test]
    fn a_second_gap_in_the_same_turn_is_counted_again() {
        let (q, mut source) = queue();
        let mut buf = vec![0.0; PERIOD];
        for _ in 0..2 {
            q.enqueue(&tone(24_000, 10, 800.0)).expect("fits");
            for _ in 0..3 {
                source.fill(&mut buf);
            }
        }
        assert_eq!(q.stats().underruns, 2);
    }

    #[test]
    fn silence_with_nothing_queued_is_not_an_underrun() {
        let (q, mut source) = queue();
        let mut buf = vec![1.0; PERIOD];
        for _ in 0..10 {
            source.fill(&mut buf);
        }
        assert!(buf.iter().all(|s| *s == 0.0));
        assert_eq!(q.stats().underruns, 0);
    }

    #[test]
    fn a_full_queue_refuses_the_chunk_whole() {
        let (q, mut source) = playback_queue(DEVICE, Duration::from_millis(100)).expect("builds");
        q.enqueue(&tone(24_000, 80, 700.0)).expect("first fits");
        let before = q.stats().samples_queued;
        let err = q
            .enqueue(&tone(24_000, 80, 700.0))
            .expect_err("second does not");
        assert!(matches!(err, PlaybackError::QueueFull { .. }), "{err}");
        assert_eq!(
            q.stats().samples_queued,
            before,
            "nothing partial was queued"
        );
        // Playing makes room again.
        let mut buf = vec![0.0; PERIOD * 8];
        source.fill(&mut buf);
        q.enqueue(&tone(24_000, 80, 700.0))
            .expect("fits after playing");
    }

    #[test]
    fn invalid_input_is_an_error() {
        assert!(matches!(
            playback_queue(StreamFormat::new(0, 2), Duration::from_secs(1)),
            Err(PlaybackError::EmptyFormat(_))
        ));
        assert!(matches!(
            playback_queue(DEVICE, Duration::ZERO),
            Err(PlaybackError::ZeroCapacity)
        ));
        let (q, _source) = queue();
        let bad = PcmChunk {
            samples: vec![0.0; 10],
            sample_rate: 0,
        };
        assert!(matches!(q.enqueue(&bad), Err(PlaybackError::ChunkRate)));
    }

    #[test]
    fn loud_or_broken_samples_cannot_reach_the_device() {
        let (q, mut source) = queue();
        let chunk = PcmChunk {
            samples: vec![5.0, -5.0, f32::NAN, f32::INFINITY],
            sample_rate: 48_000,
        };
        q.enqueue(&chunk).expect("fits");
        q.finish_turn();
        let out = drain(&mut source, &q);
        assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert_eq!(&out[..8], &[1.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn many_enqueue_and_stop_calls_from_two_threads_stay_consistent() {
        let (q, mut source) = queue();
        let writer_q = q.clone();
        let writer = std::thread::spawn(move || {
            for _ in 0..200 {
                let _ = writer_q.enqueue(&tone(24_000, 20, 900.0));
            }
        });
        let stopper_q = q.clone();
        let stopper = std::thread::spawn(move || {
            for _ in 0..50 {
                stopper_q.stop();
                std::thread::yield_now();
            }
        });
        let mut buf = vec![0.0; PERIOD];
        while !(writer.is_finished() && stopper.is_finished()) {
            source.fill(&mut buf);
        }
        writer.join().expect("writer ends");
        stopper.join().expect("stopper ends");
        q.stop();
        source.fill(&mut buf);
        assert!(!q.is_active());
        assert_eq!(q.stats().samples_queued, 0);
    }
}
