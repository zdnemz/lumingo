//! Endpointing: turns a stream of per-frame speech probabilities into utterances.
//!
//! The rules come from `context_pack.md` section 4: 300 ms of pre-roll, a
//! minimum of 200 ms of speech before an utterance counts, the end of the turn
//! after 600 ms of silence (configurable from 400 to 900 ms) and a forced end at
//! 30 s.
//!
//! Two layers, so the decision logic can be tested without audio:
//!
//! * [`Endpointer`] sees only probabilities and frame numbers. It is pure logic.
//! * [`UtteranceSegmenter`] wraps an `Endpointer`, keeps the recent frames so the
//!   pre-roll exists when speech is confirmed, and hands back the audio.
//!
//! The default thresholds are conventions, not measurements. Whether they cut
//! learner speech early is decided by the F2 run through the real VAD (S2-04
//! verify), which needs the model and the fixtures.

use std::collections::VecDeque;
use std::fmt;

use thiserror::Error;

use crate::SPEECH_SAMPLE_RATE;

/// Smallest end-of-turn silence the configuration accepts.
pub const MIN_END_SILENCE_MS: u32 = 400;
/// Largest end-of-turn silence the configuration accepts.
pub const MAX_END_SILENCE_MS: u32 = 900;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EndpointConfigError {
    #[error("a frame must hold at least one sample")]
    EmptyFrame,
    #[error("end silence must be {MIN_END_SILENCE_MS} to {MAX_END_SILENCE_MS} ms, got {0}")]
    EndSilenceOutOfRange(u32),
    #[error("thresholds must satisfy 0 <= silence <= speech <= 1")]
    Thresholds,
    #[error("the maximum length of {max_ms} ms must exceed pre-roll plus minimum speech")]
    MaxLengthTooShort { max_ms: u32 },
    #[error("a duration of zero frames is not usable: {0}")]
    ZeroDuration(&'static str),
}

/// How the endpointer decides.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointConfig {
    /// Samples per VAD frame at 16 kHz (512 for Silero).
    pub frame_samples: usize,
    /// A frame at or above this probability counts as speech when no utterance
    /// is running.
    pub speech_threshold: f32,
    /// Inside an utterance a frame counts as silence only below this lower
    /// probability. The gap between the two thresholds stops a probability that
    /// hovers near one value from ending a turn.
    pub silence_threshold: f32,
    pub pre_roll_ms: u32,
    pub min_speech_ms: u32,
    pub end_silence_ms: u32,
    pub max_utterance_ms: u32,
    /// Trailing frames kept after the last speech frame. The rest of the end
    /// silence is cut, so the recogniser does not wait on it.
    pub tail_ms: u32,
}

impl EndpointConfig {
    /// The defaults for 512-sample frames.
    pub fn new() -> Self {
        Self {
            frame_samples: 512,
            speech_threshold: 0.5,
            silence_threshold: 0.35,
            pre_roll_ms: 300,
            min_speech_ms: 200,
            end_silence_ms: 600,
            max_utterance_ms: 30_000,
            tail_ms: 150,
        }
    }

    /// Sets the end-of-turn silence. Fails outside 400 to 900 ms.
    pub fn with_end_silence_ms(mut self, ms: u32) -> Result<Self, EndpointConfigError> {
        if !(MIN_END_SILENCE_MS..=MAX_END_SILENCE_MS).contains(&ms) {
            return Err(EndpointConfigError::EndSilenceOutOfRange(ms));
        }
        self.end_silence_ms = ms;
        Ok(self)
    }

    /// Length of one frame in milliseconds, rounded down.
    pub fn frame_ms(&self) -> u32 {
        (self.frame_samples as u64 * 1000 / u64::from(SPEECH_SAMPLE_RATE)) as u32
    }

    pub fn validate(&self) -> Result<(), EndpointConfigError> {
        if self.frame_samples == 0 {
            return Err(EndpointConfigError::EmptyFrame);
        }
        if !(MIN_END_SILENCE_MS..=MAX_END_SILENCE_MS).contains(&self.end_silence_ms) {
            return Err(EndpointConfigError::EndSilenceOutOfRange(
                self.end_silence_ms,
            ));
        }
        let ok = (0.0..=1.0).contains(&self.silence_threshold)
            && (0.0..=1.0).contains(&self.speech_threshold)
            && self.silence_threshold <= self.speech_threshold;
        if !ok {
            return Err(EndpointConfigError::Thresholds);
        }
        if self.frame_ms() == 0 {
            return Err(EndpointConfigError::ZeroDuration("frame"));
        }
        if self.min_speech_ms == 0 {
            return Err(EndpointConfigError::ZeroDuration("minimum speech"));
        }
        if self.max_utterance_ms <= self.pre_roll_ms + self.min_speech_ms {
            return Err(EndpointConfigError::MaxLengthTooShort {
                max_ms: self.max_utterance_ms,
            });
        }
        Ok(())
    }

    /// Frames needed to cover `ms`, rounded up so a wait is never shorter than asked.
    fn frames_for(&self, ms: u32) -> u64 {
        let frame = u64::from(self.frame_ms().max(1));
        u64::from(ms).div_ceil(frame)
    }
}

impl Default for EndpointConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Why an utterance ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// The end-of-turn silence elapsed.
    Silence,
    /// The utterance reached the maximum length.
    MaxLength,
    /// The caller ended the stream while speech was running.
    Flushed,
}

impl fmt::Display for EndReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Silence => "silence",
            Self::MaxLength => "max_length",
            Self::Flushed => "flushed",
        })
    }
}

/// What the endpointer reports. Frame numbers count from the first frame pushed
/// after construction or `reset`, starting at 0. Ranges are half open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointEvent {
    /// Speech was confirmed. The utterance begins at `start_frame`, which
    /// already includes the pre-roll.
    SpeechStarted { start_frame: u64 },
    /// The utterance covers `start_frame..end_frame`. `last_speech_frame` is the
    /// last frame that counted as speech, for the latency measurement.
    Ended {
        start_frame: u64,
        end_frame: u64,
        last_speech_frame: u64,
        reason: EndReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// Consecutive speech frames seen, not yet enough to confirm.
    Pending {
        first_speech_frame: u64,
        speech_frames: u64,
    },
    Speaking {
        start_frame: u64,
        last_speech_frame: u64,
        silence_frames: u64,
    },
}

/// Pure endpointing logic over frame probabilities.
#[derive(Debug, Clone)]
pub struct Endpointer {
    config: EndpointConfig,
    min_speech_frames: u64,
    end_silence_frames: u64,
    pre_roll_frames: u64,
    tail_frames: u64,
    max_frames: u64,
    next_frame: u64,
    /// First frame the next utterance may claim. Pre-roll must not reach back
    /// into audio that already belongs to the previous utterance.
    floor_frame: u64,
    state: State,
}

impl Endpointer {
    pub fn new(config: EndpointConfig) -> Result<Self, EndpointConfigError> {
        config.validate()?;
        Ok(Self {
            min_speech_frames: config.frames_for(config.min_speech_ms),
            end_silence_frames: config.frames_for(config.end_silence_ms),
            pre_roll_frames: config.frames_for(config.pre_roll_ms),
            tail_frames: config.frames_for(config.tail_ms),
            max_frames: u64::from(config.max_utterance_ms) / u64::from(config.frame_ms()),
            config,
            next_frame: 0,
            floor_frame: 0,
            state: State::Idle,
        })
    }

    pub fn config(&self) -> &EndpointConfig {
        &self.config
    }

    /// Frames of pre-roll the caller must be able to look back over.
    pub fn pre_roll_frames(&self) -> u64 {
        self.pre_roll_frames
    }

    /// Frames kept after the last speech frame in a finished utterance.
    pub fn tail_frames(&self) -> u64 {
        self.tail_frames
    }

    pub fn is_speaking(&self) -> bool {
        matches!(self.state, State::Speaking { .. })
    }

    /// Frames confirmed in the current speech run, for look-back sizing.
    pub fn min_speech_frames(&self) -> u64 {
        self.min_speech_frames
    }

    /// Forgets the current utterance and restarts frame numbering.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.next_frame = 0;
        self.floor_frame = 0;
    }

    /// Consumes the probability of the next frame.
    pub fn push(&mut self, probability: f32) -> Option<EndpointEvent> {
        let frame = self.next_frame;
        self.next_frame += 1;
        match self.state {
            State::Idle => {
                if probability >= self.config.speech_threshold {
                    self.state = State::Pending {
                        first_speech_frame: frame,
                        speech_frames: 1,
                    };
                    return self.confirm_if_enough(frame);
                }
                None
            }
            State::Pending {
                first_speech_frame,
                speech_frames,
            } => {
                if probability >= self.config.speech_threshold {
                    self.state = State::Pending {
                        first_speech_frame,
                        speech_frames: speech_frames + 1,
                    };
                    self.confirm_if_enough(frame)
                } else {
                    // A gap before confirmation means a click or a cough, not a
                    // turn. Start counting again.
                    self.state = State::Idle;
                    None
                }
            }
            State::Speaking {
                start_frame,
                last_speech_frame,
                silence_frames,
            } => {
                let is_silence = probability < self.config.silence_threshold;
                let (last_speech_frame, silence_frames) = if is_silence {
                    (last_speech_frame, silence_frames + 1)
                } else {
                    (frame, 0)
                };
                let length = frame + 1 - start_frame;
                if length >= self.max_frames {
                    self.state = State::Idle;
                    self.floor_frame = frame + 1;
                    return Some(EndpointEvent::Ended {
                        start_frame,
                        end_frame: frame + 1,
                        last_speech_frame,
                        reason: EndReason::MaxLength,
                    });
                }
                if silence_frames >= self.end_silence_frames {
                    self.state = State::Idle;
                    let end_frame = (last_speech_frame + 1 + self.tail_frames).min(frame + 1);
                    self.floor_frame = end_frame;
                    return Some(EndpointEvent::Ended {
                        start_frame,
                        end_frame,
                        last_speech_frame,
                        reason: EndReason::Silence,
                    });
                }
                self.state = State::Speaking {
                    start_frame,
                    last_speech_frame,
                    silence_frames,
                };
                None
            }
        }
    }

    /// Ends the stream. An utterance that is still running is closed with
    /// [`EndReason::Flushed`]; a pending one is dropped as too short.
    pub fn finish(&mut self) -> Option<EndpointEvent> {
        let state = std::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Speaking {
                start_frame,
                last_speech_frame,
                ..
            } => {
                let end_frame = (last_speech_frame + 1 + self.tail_frames).min(self.next_frame);
                self.floor_frame = end_frame;
                Some(EndpointEvent::Ended {
                    start_frame,
                    end_frame,
                    last_speech_frame,
                    reason: EndReason::Flushed,
                })
            }
            _ => None,
        }
    }

    fn confirm_if_enough(&mut self, frame: u64) -> Option<EndpointEvent> {
        let State::Pending {
            first_speech_frame,
            speech_frames,
        } = self.state
        else {
            return None;
        };
        if speech_frames < self.min_speech_frames {
            return None;
        }
        let start_frame = first_speech_frame
            .saturating_sub(self.pre_roll_frames)
            .max(self.floor_frame);
        self.state = State::Speaking {
            start_frame,
            last_speech_frame: frame,
            silence_frames: 0,
        };
        Some(EndpointEvent::SpeechStarted { start_frame })
    }
}

/// One finished utterance with its audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    /// 16 kHz mono samples, pre-roll included, most of the end silence cut.
    pub samples: Vec<f32>,
    pub reason: EndReason,
    /// Milliseconds from the first sample of the utterance to the end of the
    /// last frame that counted as speech.
    pub speech_end_offset_ms: u32,
}

/// What the segmenter reports to the caller.
#[derive(Debug, Clone, PartialEq)]
pub enum SegmentEvent {
    SpeechStarted,
    Utterance(Utterance),
}

/// An [`Endpointer`] plus the audio around it.
///
/// Memory is bounded: while no utterance runs it keeps the last
/// `pre_roll + min_speech + 1` frames, and during one it holds at most
/// `max_utterance_ms` of samples, because the endpointer forces an end there.
#[derive(Debug)]
pub struct UtteranceSegmenter {
    endpointer: Endpointer,
    /// Recent frames with their frame numbers.
    frames: VecDeque<(u64, Vec<f32>)>,
    idle_limit: usize,
    next_frame: u64,
}

impl UtteranceSegmenter {
    pub fn new(config: EndpointConfig) -> Result<Self, EndpointConfigError> {
        let endpointer = Endpointer::new(config)?;
        let idle_limit = (endpointer.pre_roll_frames() + endpointer.min_speech_frames() + 1)
            .try_into()
            .unwrap_or(usize::MAX);
        Ok(Self {
            endpointer,
            frames: VecDeque::new(),
            idle_limit,
            next_frame: 0,
        })
    }

    pub fn config(&self) -> &EndpointConfig {
        self.endpointer.config()
    }

    pub fn is_speaking(&self) -> bool {
        self.endpointer.is_speaking()
    }

    /// Drops everything held and restarts, for a new session.
    pub fn reset(&mut self) {
        self.endpointer.reset();
        self.frames.clear();
        self.next_frame = 0;
    }

    /// Consumes one frame and the probability the VAD gave it. The caller
    /// passes frames of `config().frame_samples` samples; the VAD rejects any
    /// other size, so no probability would exist for one.
    pub fn push_frame(&mut self, probability: f32, frame: &[f32]) -> Option<SegmentEvent> {
        self.frames.push_back((self.next_frame, frame.to_vec()));
        self.next_frame += 1;
        let event = self.endpointer.push(probability);
        let out = match event {
            None => None,
            Some(EndpointEvent::SpeechStarted { .. }) => Some(SegmentEvent::SpeechStarted),
            Some(ended @ EndpointEvent::Ended { .. }) => self.take_utterance(ended),
        };
        self.trim_idle();
        out
    }

    /// Ends the stream. Returns the utterance that was still running, if any.
    pub fn finish(&mut self) -> Option<Utterance> {
        let ended = self.endpointer.finish()?;
        let out = match self.take_utterance(ended) {
            Some(SegmentEvent::Utterance(utterance)) => Some(utterance),
            _ => None,
        };
        self.trim_idle();
        out
    }

    fn take_utterance(&mut self, event: EndpointEvent) -> Option<SegmentEvent> {
        let EndpointEvent::Ended {
            start_frame,
            end_frame,
            last_speech_frame,
            reason,
        } = event
        else {
            return None;
        };
        let mut samples = Vec::new();
        for (number, frame) in &self.frames {
            if (start_frame..end_frame).contains(number) {
                samples.extend_from_slice(frame);
            }
        }
        // Frames from `end_frame` on stay as pre-roll for the next utterance.
        while self.frames.front().is_some_and(|(n, _)| *n < end_frame) {
            self.frames.pop_front();
        }
        let frame_ms = u64::from(self.endpointer.config().frame_ms());
        let speech_end_offset_ms = ((last_speech_frame + 1 - start_frame) * frame_ms) as u32;
        Some(SegmentEvent::Utterance(Utterance {
            samples,
            reason,
            speech_end_offset_ms,
        }))
    }

    fn trim_idle(&mut self) {
        if self.endpointer.is_speaking() {
            return;
        }
        while self.frames.len() > self.idle_limit {
            self.frames.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One character per frame: `s` is clear speech, `.` is clear silence,
    /// `m` sits between the two thresholds.
    fn probabilities(pattern: &str) -> Vec<f32> {
        pattern
            .chars()
            .map(|c| match c {
                's' => 0.9,
                '.' => 0.05,
                'm' => 0.4,
                other => panic!("bad pattern character {other}"),
            })
            .collect()
    }

    fn run(config: EndpointConfig, pattern: &str, flush: bool) -> Vec<(usize, EndpointEvent)> {
        let mut endpointer = Endpointer::new(config).expect("valid config");
        let mut out = Vec::new();
        for (i, p) in probabilities(pattern).into_iter().enumerate() {
            if let Some(event) = endpointer.push(p) {
                out.push((i, event));
            }
        }
        if flush && let Some(event) = endpointer.finish() {
            out.push((pattern.len(), event));
        }
        out
    }

    /// 100 ms frames keep the arithmetic readable: pre-roll 3 frames, minimum
    /// speech 2, end silence 6 (the 600 ms default), tail 1, maximum 300.
    fn config_100ms() -> EndpointConfig {
        EndpointConfig {
            frame_samples: 1600,
            tail_ms: 100,
            ..EndpointConfig::new()
        }
    }

    fn config_100ms_max(max_utterance_ms: u32) -> EndpointConfig {
        EndpointConfig {
            max_utterance_ms,
            ..config_100ms()
        }
    }

    fn started(start_frame: u64) -> EndpointEvent {
        EndpointEvent::SpeechStarted { start_frame }
    }

    fn ended(start: u64, end: u64, last: u64, reason: EndReason) -> EndpointEvent {
        EndpointEvent::Ended {
            start_frame: start,
            end_frame: end,
            last_speech_frame: last,
            reason,
        }
    }

    fn dots(n: usize) -> String {
        ".".repeat(n)
    }

    #[test]
    fn table_of_probability_sequences() {
        struct Case {
            name: &'static str,
            config: EndpointConfig,
            pattern: String,
            flush: bool,
            expected: Vec<(usize, EndpointEvent)>,
        }
        let cases = vec![
            Case {
                name: "silence only produces nothing",
                config: config_100ms(),
                pattern: dots(50),
                flush: true,
                expected: vec![],
            },
            Case {
                name: "a click shorter than the minimum speech is ignored",
                config: config_100ms(),
                pattern: format!("{}s{}", dots(5), dots(20)),
                flush: true,
                expected: vec![],
            },
            Case {
                name: "exactly the minimum speech starts a turn, pre-roll reaches back 3 frames",
                config: config_100ms(),
                // Speech frames 5 and 6. Silence frames 7 to 12 make 6 at index 12.
                // The tail keeps one frame after the last speech frame.
                pattern: format!("{}ss{}", dots(5), dots(6)),
                flush: false,
                expected: vec![(6, started(2)), (12, ended(2, 8, 6, EndReason::Silence))],
            },
            Case {
                name: "pre-roll cannot reach before the first frame",
                config: config_100ms(),
                pattern: format!("ss{}", dots(6)),
                flush: false,
                expected: vec![(1, started(0)), (7, ended(0, 3, 1, EndReason::Silence))],
            },
            Case {
                name: "a gap inside the pending run resets the count",
                config: config_100ms(),
                pattern: format!("s.s{}", dots(6)),
                flush: false,
                expected: vec![],
            },
            Case {
                name: "five silent frames do not end the turn, the sixth does",
                config: config_100ms(),
                pattern: format!("ss{}ss{}", dots(5), dots(6)),
                flush: false,
                expected: vec![(1, started(0)), (14, ended(0, 10, 8, EndReason::Silence))],
            },
            Case {
                name: "exactly five silent frames at the end do not end the turn",
                config: config_100ms(),
                pattern: format!("ss{}", dots(5)),
                flush: false,
                expected: vec![(1, started(0))],
            },
            Case {
                name: "probabilities between the thresholds keep the turn open",
                config: config_100ms(),
                pattern: format!("ss{}s{}", "m".repeat(20), dots(6)),
                flush: false,
                expected: vec![(1, started(0)), (28, ended(0, 24, 22, EndReason::Silence))],
            },
            Case {
                name: "the middle probability does not start a turn from idle",
                config: config_100ms(),
                pattern: "m".repeat(30),
                flush: true,
                expected: vec![],
            },
            Case {
                name: "flush closes a running utterance",
                config: config_100ms(),
                pattern: format!("{}ssss", dots(4)),
                flush: true,
                expected: vec![(5, started(1)), (8, ended(1, 8, 7, EndReason::Flushed))],
            },
            Case {
                name: "flush with one speech frame pending produces nothing",
                config: config_100ms(),
                pattern: format!("{}s", dots(4)),
                flush: true,
                expected: vec![],
            },
            Case {
                name: "a turn that never goes quiet is cut at the maximum length",
                // 1000 ms is 10 frames. Speech from frame 5, start 2, cut at 12.
                config: config_100ms_max(1000),
                pattern: format!("{}{}", dots(5), "s".repeat(7)),
                flush: false,
                expected: vec![
                    (6, started(2)),
                    (11, ended(2, 12, 11, EndReason::MaxLength)),
                ],
            },
            Case {
                name: "after a forced end the next speech starts a new turn without overlap",
                config: config_100ms_max(1000),
                // Speech from frame 0. Start 0, cut at frame 9 (10 frames). Frame 10
                // is pending, 11 confirms. The pre-roll may not reach before 10.
                pattern: "s".repeat(12),
                flush: false,
                expected: vec![
                    (1, started(0)),
                    (9, ended(0, 10, 9, EndReason::MaxLength)),
                    (11, started(10)),
                ],
            },
        ];
        for case in cases {
            let got = run(case.config, &case.pattern, case.flush);
            assert_eq!(got, case.expected, "{}", case.name);
        }
    }

    #[test]
    fn no_utterance_is_longer_than_the_maximum() {
        let config = EndpointConfig {
            max_utterance_ms: 2000,
            ..EndpointConfig::new()
        };
        let max_frames = u64::from(config.max_utterance_ms / config.frame_ms());
        let got = run(config, &"s".repeat(400), false);
        let mut ends = 0;
        for (_, event) in got {
            if let EndpointEvent::Ended {
                start_frame,
                end_frame,
                ..
            } = event
            {
                ends += 1;
                assert!(end_frame - start_frame <= max_frames);
            }
        }
        assert!(ends >= 5, "continuous speech must be cut repeatedly");
    }

    #[test]
    fn default_durations_round_to_whole_frames_of_32_ms() {
        let endpointer = Endpointer::new(EndpointConfig::new()).expect("valid");
        assert_eq!(endpointer.config().frame_ms(), 32);
        assert_eq!(endpointer.pre_roll_frames(), 10);
        assert_eq!(endpointer.min_speech_frames(), 7);
        // 600 ms is 18.75 frames, so 19: a wait is at least as long as asked.
        assert_eq!(endpointer.end_silence_frames, 19);
        // 30 s is 937.5 frames: the cap rounds down so it is never exceeded.
        assert_eq!(endpointer.max_frames, 937);
    }

    #[test]
    fn end_silence_is_bounded_to_400_through_900_ms() {
        let base = EndpointConfig::new();
        assert!(base.clone().with_end_silence_ms(400).is_ok());
        assert!(base.clone().with_end_silence_ms(900).is_ok());
        assert_eq!(
            base.clone().with_end_silence_ms(399),
            Err(EndpointConfigError::EndSilenceOutOfRange(399))
        );
        assert_eq!(
            base.with_end_silence_ms(901),
            Err(EndpointConfigError::EndSilenceOutOfRange(901))
        );
    }

    #[test]
    fn a_longer_end_silence_waits_longer() {
        let short = EndpointConfig::new().with_end_silence_ms(400).expect("ok");
        let long = EndpointConfig::new().with_end_silence_ms(900).expect("ok");
        let pattern = format!("{}{}", "s".repeat(10), dots(60));
        let end_at = |config: EndpointConfig| {
            run(config, &pattern, false)
                .into_iter()
                .find_map(|(i, e)| matches!(e, EndpointEvent::Ended { .. }).then_some(i))
        };
        let short_end = end_at(short).expect("ends");
        let long_end = end_at(long).expect("ends");
        // 400 ms is 13 frames of 32 ms, 900 ms is 29.
        assert_eq!(long_end - short_end, 29 - 13);
    }

    #[test]
    fn invalid_configs_are_rejected() {
        let mut c = EndpointConfig::new();
        c.frame_samples = 0;
        assert_eq!(c.validate(), Err(EndpointConfigError::EmptyFrame));

        let mut c = EndpointConfig::new();
        c.silence_threshold = 0.9;
        c.speech_threshold = 0.5;
        assert_eq!(c.validate(), Err(EndpointConfigError::Thresholds));

        let mut c = EndpointConfig::new();
        c.max_utterance_ms = 400;
        assert!(matches!(
            c.validate(),
            Err(EndpointConfigError::MaxLengthTooShort { .. })
        ));

        let mut c = EndpointConfig::new();
        c.min_speech_ms = 0;
        assert!(matches!(
            c.validate(),
            Err(EndpointConfigError::ZeroDuration(_))
        ));
        assert!(Endpointer::new(c).is_err());

        let mut c = EndpointConfig::new();
        c.end_silence_ms = 100;
        assert!(Endpointer::new(c).is_err());
    }

    #[test]
    fn reset_restarts_frame_numbers_and_state() {
        let mut endpointer = Endpointer::new(config_100ms()).expect("valid");
        for p in probabilities("ss") {
            endpointer.push(p);
        }
        assert!(endpointer.is_speaking());
        endpointer.reset();
        assert!(!endpointer.is_speaking());
        let events: Vec<_> = probabilities("ss")
            .into_iter()
            .filter_map(|p| endpointer.push(p))
            .collect();
        assert_eq!(events, vec![started(0)]);
    }

    /// Frames whose samples all equal their index, so an utterance's audio shows
    /// which frames it holds.
    fn marked_frames(config: &EndpointConfig, pattern: &str) -> Vec<(f32, Vec<f32>)> {
        probabilities(pattern)
            .into_iter()
            .enumerate()
            .map(|(i, p)| (p, vec![i as f32; config.frame_samples]))
            .collect()
    }

    fn frame_indices(samples: &[f32], frame_samples: usize) -> Vec<u32> {
        samples
            .chunks(frame_samples)
            .map(|chunk| chunk[0] as u32)
            .collect()
    }

    #[test]
    fn the_segmenter_returns_pre_roll_speech_and_a_short_tail() {
        let config = config_100ms();
        let mut segmenter = UtteranceSegmenter::new(config.clone()).expect("valid");
        let pattern = format!("{}ss..s{}", dots(5), dots(6));
        let mut events = Vec::new();
        for (p, frame) in marked_frames(&config, &pattern) {
            if let Some(event) = segmenter.push_frame(p, &frame) {
                events.push(event);
            }
        }
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0], SegmentEvent::SpeechStarted);
        let SegmentEvent::Utterance(utterance) = &events[1] else {
            panic!("expected an utterance");
        };
        // Speech frames 5, 6 and 9. Pre-roll starts at frame 2. The last speech
        // frame is 9 and the tail keeps frame 10.
        assert_eq!(
            frame_indices(&utterance.samples, config.frame_samples),
            vec![2, 3, 4, 5, 6, 7, 8, 9, 10]
        );
        assert_eq!(utterance.reason, EndReason::Silence);
        // From the first sample (frame 2) to the end of frame 9: 8 frames.
        assert_eq!(utterance.speech_end_offset_ms, 800);
    }

    #[test]
    fn the_segmenter_flushes_a_running_utterance() {
        let config = config_100ms();
        let mut segmenter = UtteranceSegmenter::new(config.clone()).expect("valid");
        for (p, frame) in marked_frames(&config, "..sss") {
            segmenter.push_frame(p, &frame);
        }
        assert!(segmenter.is_speaking());
        let utterance = segmenter.finish().expect("running utterance");
        assert_eq!(utterance.reason, EndReason::Flushed);
        assert_eq!(
            frame_indices(&utterance.samples, config.frame_samples),
            vec![0, 1, 2, 3, 4]
        );
        assert!(segmenter.finish().is_none());
        assert!(!segmenter.is_speaking());
    }

    #[test]
    fn the_segmenter_reports_two_utterances_in_a_row() {
        let config = config_100ms();
        let mut segmenter = UtteranceSegmenter::new(config.clone()).expect("valid");
        let pattern = format!("ss{}ss{}", dots(8), dots(8));
        let utterances: Vec<Utterance> = marked_frames(&config, &pattern)
            .into_iter()
            .filter_map(|(p, frame)| match segmenter.push_frame(p, &frame) {
                Some(SegmentEvent::Utterance(u)) => Some(u),
                _ => None,
            })
            .collect();
        assert_eq!(utterances.len(), 2);
        assert_eq!(
            frame_indices(&utterances[0].samples, config.frame_samples),
            vec![0, 1, 2]
        );
        // The second speech run is frames 10 and 11, so the pre-roll starts at 7.
        // The first utterance ended at frame 3, so no audio is shared.
        assert_eq!(
            frame_indices(&utterances[1].samples, config.frame_samples)[0],
            7
        );
    }

    #[test]
    fn the_segmenter_never_holds_more_than_the_maximum() {
        let config = EndpointConfig {
            max_utterance_ms: 1000,
            ..EndpointConfig::new()
        };
        let max_samples =
            (config.max_utterance_ms / config.frame_ms()) as usize * config.frame_samples;
        let mut segmenter = UtteranceSegmenter::new(config.clone()).expect("valid");
        let frame = vec![0.1_f32; config.frame_samples];
        let mut utterances = 0;
        for _ in 0..500 {
            if let Some(SegmentEvent::Utterance(u)) = segmenter.push_frame(0.9, &frame) {
                utterances += 1;
                assert!(u.samples.len() <= max_samples);
            }
            assert!(segmenter.frames.len() <= 40);
        }
        assert!(utterances > 5);
    }

    #[test]
    fn the_segmenter_keeps_only_the_idle_look_back_while_silent() {
        let config = EndpointConfig::new();
        let mut segmenter = UtteranceSegmenter::new(config.clone()).expect("valid");
        let frame = vec![0.0_f32; config.frame_samples];
        for _ in 0..1000 {
            segmenter.push_frame(0.0, &frame);
        }
        // pre-roll 10 + minimum speech 7 + 1
        assert_eq!(segmenter.frames.len(), 18);
    }
}
