//! The latency of one voice turn, in the five parts of `context_pack.md`
//! section 4.
//!
//! A turn is measured from the last sample the voice activity detector
//! classified as speech to the first tutor sample written to the output stream.
//! Six instants, all read from one [`LoopClock`](super::LoopClock), cut that
//! span into five parts that add up to the whole by construction:
//!
//! | Part | From | To |
//! |---|---|---|
//! | endpointing wait | last speech sample | the endpointer ended the utterance |
//! | STT finalise | utterance ended | transcript received |
//! | LLM first sentence | transcript received | first sentence complete |
//! | TTS first sentence | first sentence complete | its audio queued for playback |
//! | output start | audio queued | first sample played |
//!
//! "First sample played" is the moment the output callback first consumed queued
//! audio, observed by polling the queue counters every couple of milliseconds, so
//! the output part carries that polling resolution.
//!
//! A turn without a microphone or without speech output has no such span. Parts
//! that were not on its path are `None`, the sum covers the parts present and
//! `complete` says whether all five were.

use std::time::Duration;

use serde::Serialize;

/// The instants of one turn. Each is a reading of the loop clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stamps {
    pub last_speech: Option<Duration>,
    pub utterance_ended: Option<Duration>,
    pub transcript_ready: Option<Duration>,
    pub first_sentence: Option<Duration>,
    pub audio_queued: Option<Duration>,
    pub first_sample_played: Option<Duration>,
}

/// The five parts. A part is `None` when its start or end was not on the turn's path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LatencyParts {
    pub endpointing_wait: Option<Duration>,
    pub stt_finalise: Option<Duration>,
    pub llm_first_sentence: Option<Duration>,
    pub tts_first_sentence: Option<Duration>,
    pub output_start: Option<Duration>,
}

fn between(from: Option<Duration>, to: Option<Duration>) -> Option<Duration> {
    // A reading that is behind its predecessor cannot happen with a monotonic
    // clock; the part is dropped instead of being reported as zero.
    to?.checked_sub(from?)
}

impl LatencyParts {
    pub fn from_stamps(s: &Stamps) -> Self {
        Self {
            endpointing_wait: between(s.last_speech, s.utterance_ended),
            stt_finalise: between(s.utterance_ended, s.transcript_ready),
            llm_first_sentence: between(s.transcript_ready, s.first_sentence),
            tts_first_sentence: between(s.first_sentence, s.audio_queued),
            output_start: between(s.audio_queued, s.first_sample_played),
        }
    }

    fn all(&self) -> [Option<Duration>; 5] {
        [
            self.endpointing_wait,
            self.stt_finalise,
            self.llm_first_sentence,
            self.tts_first_sentence,
            self.output_start,
        ]
    }

    /// The sum of the parts that are present.
    pub fn sum(&self) -> Duration {
        self.all().into_iter().flatten().sum()
    }

    /// True when all five parts are present: a spoken turn answered by speech.
    pub fn is_complete(&self) -> bool {
        self.all().iter().all(Option::is_some)
    }
}

/// One turn's latency as reported to the learner, the event stream and the result file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnLatency {
    pub turn: u64,
    pub parts: LatencyParts,
}

impl TurnLatency {
    pub fn sum(&self) -> Duration {
        self.parts.sum()
    }

    pub fn is_complete(&self) -> bool {
        self.parts.is_complete()
    }

    /// The turn as one line of the result file.
    pub fn record(&self) -> LatencyRecord {
        let ms = |d: Option<Duration>| d.map(duration_ms);
        LatencyRecord {
            turn: self.turn,
            endpointing_wait_ms: ms(self.parts.endpointing_wait),
            stt_finalise_ms: ms(self.parts.stt_finalise),
            llm_first_sentence_ms: ms(self.parts.llm_first_sentence),
            tts_first_sentence_ms: ms(self.parts.tts_first_sentence),
            output_start_ms: ms(self.parts.output_start),
            sum_ms: duration_ms(self.sum()),
            complete: self.is_complete(),
        }
    }
}

/// Milliseconds with microsecond precision. Parts are whole microseconds, so the
/// sum of the printed parts equals the printed sum.
pub fn duration_ms(d: Duration) -> f64 {
    d.as_micros() as f64 / 1000.0
}

/// The serialised form of [`TurnLatency`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatencyRecord {
    pub turn: u64,
    pub endpointing_wait_ms: Option<f64>,
    pub stt_finalise_ms: Option<f64>,
    pub llm_first_sentence_ms: Option<f64>,
    pub tts_first_sentence_ms: Option<f64>,
    pub output_start_ms: Option<f64>,
    pub sum_ms: f64,
    pub complete: bool,
}

/// Nearest-rank percentile of `values`, `q` in 0 to 100. `None` for no values.
/// Nearest rank returns a value that was measured; it never interpolates a
/// latency nobody saw.
pub fn percentile(values: &[Duration], q: u32) -> Option<Duration> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let q = u64::from(q.min(100));
    let rank = (q * sorted.len() as u64).div_ceil(100).max(1);
    sorted.get(usize::try_from(rank).ok()? - 1).copied()
}

/// p50 and p95 of one series.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Quantiles {
    pub count: usize,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
}

impl Quantiles {
    fn of(values: &[Duration]) -> Self {
        Self {
            count: values.len(),
            p50_ms: percentile(values, 50).map(duration_ms),
            p95_ms: percentile(values, 95).map(duration_ms),
        }
    }
}

/// p50 and p95 per part and for the sum, over a set of turns.
///
/// The sum is taken over complete turns only, so a text turn without a
/// microphone cannot pull the end-to-end figure down. Each part uses the turns
/// that have it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatencySummary {
    pub turns: usize,
    pub complete_turns: usize,
    pub endpointing_wait: Quantiles,
    pub stt_finalise: Quantiles,
    pub llm_first_sentence: Quantiles,
    pub tts_first_sentence: Quantiles,
    pub output_start: Quantiles,
    pub sum: Quantiles,
}

impl LatencySummary {
    pub fn of(turns: &[TurnLatency]) -> Self {
        let part = |pick: fn(&LatencyParts) -> Option<Duration>| -> Vec<Duration> {
            turns.iter().filter_map(|t| pick(&t.parts)).collect()
        };
        let sums: Vec<Duration> = turns
            .iter()
            .filter(|t| t.is_complete())
            .map(TurnLatency::sum)
            .collect();
        Self {
            turns: turns.len(),
            complete_turns: sums.len(),
            endpointing_wait: Quantiles::of(&part(|p| p.endpointing_wait)),
            stt_finalise: Quantiles::of(&part(|p| p.stt_finalise)),
            llm_first_sentence: Quantiles::of(&part(|p| p.llm_first_sentence)),
            tts_first_sentence: Quantiles::of(&part(|p| p.tts_first_sentence)),
            output_start: Quantiles::of(&part(|p| p.output_start)),
            sum: Quantiles::of(&sums),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn full_stamps() -> Stamps {
        Stamps {
            last_speech: Some(ms(1_000)),
            utterance_ended: Some(ms(1_608)),
            transcript_ready: Some(ms(2_308)),
            first_sentence: Some(ms(3_208)),
            audio_queued: Some(ms(3_508)),
            first_sample_played: Some(ms(3_558)),
        }
    }

    #[test]
    fn the_five_parts_come_from_neighbouring_stamps_and_add_up_to_the_whole() {
        let stamps = full_stamps();
        let parts = LatencyParts::from_stamps(&stamps);
        assert_eq!(parts.endpointing_wait, Some(ms(608)));
        assert_eq!(parts.stt_finalise, Some(ms(700)));
        assert_eq!(parts.llm_first_sentence, Some(ms(900)));
        assert_eq!(parts.tts_first_sentence, Some(ms(300)));
        assert_eq!(parts.output_start, Some(ms(50)));
        assert!(parts.is_complete());
        let whole = stamps.first_sample_played.unwrap() - stamps.last_speech.unwrap();
        assert_eq!(parts.sum(), whole);
        assert_eq!(parts.sum(), ms(2_558));
    }

    #[test]
    fn a_part_without_both_ends_is_missing_and_the_turn_is_not_complete() {
        // Table: which stamp is absent, which parts remain.
        type Clear = fn(&mut Stamps);
        let cases: [(Clear, [bool; 5]); 6] = [
            (|s| s.last_speech = None, [false, true, true, true, true]),
            (
                |s| s.utterance_ended = None,
                [false, false, true, true, true],
            ),
            (
                |s| s.transcript_ready = None,
                [true, false, false, true, true],
            ),
            (
                |s| s.first_sentence = None,
                [true, true, false, false, true],
            ),
            (|s| s.audio_queued = None, [true, true, true, false, false]),
            (
                |s| s.first_sample_played = None,
                [true, true, true, true, false],
            ),
        ];
        for (clear, present) in cases {
            let mut stamps = full_stamps();
            clear(&mut stamps);
            let parts = LatencyParts::from_stamps(&stamps);
            assert_eq!(parts.all().map(|p| p.is_some()), present);
            assert!(!parts.is_complete());
        }
    }

    #[test]
    fn a_typed_turn_has_only_the_parts_it_went_through() {
        let stamps = Stamps {
            transcript_ready: Some(ms(100)),
            first_sentence: Some(ms(1_100)),
            ..Stamps::default()
        };
        let parts = LatencyParts::from_stamps(&stamps);
        assert_eq!(parts.llm_first_sentence, Some(ms(1_000)));
        assert_eq!(parts.sum(), ms(1_000));
        assert!(!parts.is_complete());
    }

    #[test]
    fn a_reading_behind_its_predecessor_is_not_reported_as_a_part() {
        let stamps = Stamps {
            transcript_ready: Some(ms(500)),
            first_sentence: Some(ms(400)),
            ..Stamps::default()
        };
        assert_eq!(LatencyParts::from_stamps(&stamps).llm_first_sentence, None);
    }

    #[test]
    fn the_record_prints_parts_and_a_sum_that_agree() {
        let latency = TurnLatency {
            turn: 3,
            parts: LatencyParts::from_stamps(&full_stamps()),
        };
        let record = latency.record();
        let printed = [
            record.endpointing_wait_ms,
            record.stt_finalise_ms,
            record.llm_first_sentence_ms,
            record.tts_first_sentence_ms,
            record.output_start_ms,
        ]
        .into_iter()
        .flatten()
        .sum::<f64>();
        assert!((printed - record.sum_ms).abs() < 1e-9);
        assert_eq!(record.sum_ms, 2_558.0);
        assert!(record.complete);
        assert_eq!(record.turn, 3);
    }

    #[test]
    fn percentiles_use_the_nearest_rank_and_return_a_measured_value() {
        let values: Vec<Duration> = (1..=20).map(|n| ms(n * 10)).collect();
        // Table: quantile, expected value.
        for (q, expected) in [
            (50, 100),
            (95, 190),
            (100, 200),
            (0, 10),
            (5, 10),
            (51, 110),
        ] {
            assert_eq!(percentile(&values, q), Some(ms(expected)), "q = {q}");
        }
        assert_eq!(percentile(&[ms(7)], 95), Some(ms(7)));
        assert_eq!(percentile(&[], 50), None);
        // The order of the input does not matter.
        let mut reversed = values.clone();
        reversed.reverse();
        assert_eq!(percentile(&reversed, 95), Some(ms(190)));
    }

    #[test]
    fn the_summary_takes_the_sum_over_complete_turns_only() {
        let complete = |turn, extra: u64| TurnLatency {
            turn,
            parts: LatencyParts {
                endpointing_wait: Some(ms(600)),
                stt_finalise: Some(ms(700)),
                llm_first_sentence: Some(ms(900 + extra)),
                tts_first_sentence: Some(ms(300)),
                output_start: Some(ms(50)),
            },
        };
        let typed = TurnLatency {
            turn: 3,
            parts: LatencyParts {
                llm_first_sentence: Some(ms(100)),
                ..LatencyParts::default()
            },
        };
        let summary = LatencySummary::of(&[complete(1, 0), complete(2, 1_000), typed]);
        assert_eq!(summary.turns, 3);
        assert_eq!(summary.complete_turns, 2);
        assert_eq!(summary.sum.count, 2);
        assert_eq!(summary.sum.p50_ms, Some(2_550.0));
        assert_eq!(summary.sum.p95_ms, Some(3_550.0));
        assert_eq!(summary.llm_first_sentence.count, 3);
        assert_eq!(summary.llm_first_sentence.p50_ms, Some(900.0));
        assert_eq!(summary.endpointing_wait.count, 2);
    }

    #[test]
    fn an_empty_run_has_no_quantiles() {
        let summary = LatencySummary::of(&[]);
        assert_eq!(summary.turns, 0);
        assert_eq!(summary.sum.p50_ms, None);
    }
}
