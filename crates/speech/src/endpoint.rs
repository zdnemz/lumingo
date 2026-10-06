use std::collections::VecDeque;

/// Timing rules of context_pack.md section 4, step 4. All durations are in
/// milliseconds and are rounded up to whole frames.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointConfig {
    /// Length of one frame as fed to the VAD.
    pub frame_ms: u32,
    /// Probability at or above which a frame counts as speech.
    pub threshold: f32,
    /// Audio kept from before the first confirmed speech frame.
    pub pre_roll_ms: u32,
    /// Speech shorter than this is dropped as a click or cough.
    pub min_speech_ms: u32,
    /// Silence that ends a turn. The tunable range is 400 to 900.
    pub end_silence_ms: u32,
    /// An utterance is cut here even if the learner keeps talking.
    pub max_utterance_ms: u32,
}

impl Default for EndpointConfig {
    fn default() -> Self {
        Self {
            frame_ms: 32,
            threshold: 0.5,
            pre_roll_ms: 300,
            min_speech_ms: 200,
            end_silence_ms: 600,
            max_utterance_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EndpointEvent {
    /// Enough speech was heard to be sure this is an utterance.
    SpeechStarted,
    /// The utterance, pre-roll included. `forced` is true when it hit the maximum length.
    Utterance { samples: Vec<f32>, forced: bool },
}

#[derive(Debug)]
enum State {
    /// `pending` holds speech frames not yet long enough to confirm.
    Idle { pending: Vec<Vec<f32>> },
    Speaking {
        buffer: Vec<f32>,
        silent_frames: u32,
    },
}

/// Pure logic over per-frame speech probabilities. It owns no model and no
/// clock, so it is tested with probability sequences. Memory is bounded: the
/// pre-roll ring holds at most `pre_roll_ms` and an utterance at most
/// `max_utterance_ms` of audio.
#[derive(Debug)]
pub struct Endpointer {
    cfg: EndpointConfig,
    pre_roll: VecDeque<Vec<f32>>,
    state: State,
}

fn frames(ms: u32, frame_ms: u32) -> usize {
    ms.div_ceil(frame_ms.max(1)) as usize
}

impl Endpointer {
    pub fn new(cfg: EndpointConfig) -> Self {
        Self {
            cfg,
            pre_roll: VecDeque::new(),
            state: State::Idle {
                pending: Vec::new(),
            },
        }
    }

    /// Forgets everything, for the start of a new session.
    pub fn reset(&mut self) {
        self.pre_roll.clear();
        self.state = State::Idle {
            pending: Vec::new(),
        };
    }

    /// Feeds one frame with its speech probability. Returns at most one event.
    pub fn push(&mut self, frame: &[f32], probability: f32) -> Option<EndpointEvent> {
        let is_speech = probability >= self.cfg.threshold;
        let cfg = &self.cfg;
        match &mut self.state {
            State::Idle { pending } => {
                if is_speech {
                    pending.push(frame.to_vec());
                    if pending.len() < frames(cfg.min_speech_ms, cfg.frame_ms) {
                        return None;
                    }
                    let mut buffer: Vec<f32> = self.pre_roll.drain(..).flatten().collect();
                    buffer.extend(pending.drain(..).flatten());
                    self.state = State::Speaking {
                        buffer,
                        silent_frames: 0,
                    };
                    return Some(EndpointEvent::SpeechStarted);
                }
                // A short burst did not qualify: it becomes ordinary pre-roll.
                self.pre_roll.extend(pending.drain(..));
                self.pre_roll.push_back(frame.to_vec());
                let cap = frames(cfg.pre_roll_ms, cfg.frame_ms);
                while self.pre_roll.len() > cap {
                    self.pre_roll.pop_front();
                }
                None
            }
            State::Speaking {
                buffer,
                silent_frames,
            } => {
                buffer.extend_from_slice(frame);
                *silent_frames = if is_speech { 0 } else { *silent_frames + 1 };
                let ended = *silent_frames as usize >= frames(cfg.end_silence_ms, cfg.frame_ms);
                let max_samples =
                    (frames(cfg.max_utterance_ms, cfg.frame_ms) * frame.len().max(1)).max(1);
                let forced = !ended && buffer.len() >= max_samples;
                if !(ended || forced) {
                    return None;
                }
                let samples = std::mem::take(buffer);
                self.state = State::Idle {
                    pending: Vec::new(),
                };
                Some(EndpointEvent::Utterance { samples, forced })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: usize = 512; // 32 ms at 16 kHz

    /// 'S' is a speech frame, '_' a silent one. Each frame is filled with its index
    /// so tests can see which frames ended up in an utterance.
    fn run(cfg: EndpointConfig, pattern: &str) -> Vec<EndpointEvent> {
        let mut ep = Endpointer::new(cfg);
        let mut out = Vec::new();
        for (i, c) in pattern.chars().enumerate() {
            let p = if c == 'S' { 0.9 } else { 0.1 };
            out.extend(ep.push(&vec![i as f32; FRAME], p));
        }
        out
    }

    fn frame_ids(samples: &[f32]) -> Vec<usize> {
        samples.chunks(FRAME).map(|c| c[0] as usize).collect()
    }

    // 32 ms frames: min speech 200 ms = 7 frames, end silence 600 ms = 19 frames,
    // pre-roll 300 ms = 10 frames.
    fn speech(n: usize) -> String {
        "S".repeat(n)
    }
    fn silence(n: usize) -> String {
        "_".repeat(n)
    }

    #[test]
    fn silence_alone_emits_nothing() {
        assert!(run(EndpointConfig::default(), &silence(200)).is_empty());
    }

    #[test]
    fn a_burst_shorter_than_min_speech_is_dropped() {
        let p = format!("{}{}{}", silence(5), speech(6), silence(40));
        assert!(run(EndpointConfig::default(), &p).is_empty());
    }

    #[test]
    fn speech_at_min_length_starts_an_utterance_that_ends_after_the_silence() {
        let p = format!("{}{}{}", speech(7), silence(19), silence(5));
        let ev = run(EndpointConfig::default(), &p);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0], EndpointEvent::SpeechStarted);
        let EndpointEvent::Utterance { samples, forced } = &ev[1] else {
            panic!("no utterance")
        };
        assert!(!forced);
        assert_eq!(frame_ids(samples).len(), 7 + 19);
    }

    #[test]
    fn silence_one_frame_short_does_not_end_the_turn() {
        let p = format!("{}{}", speech(10), silence(18));
        let ev = run(EndpointConfig::default(), &p);
        assert_eq!(ev, vec![EndpointEvent::SpeechStarted]);
    }

    #[test]
    fn a_pause_inside_the_turn_resets_the_silence_count() {
        let p = format!(
            "{}{}{}{}{}",
            speech(10),
            silence(15),
            speech(5),
            silence(15),
            speech(1)
        );
        assert_eq!(
            run(EndpointConfig::default(), &p),
            vec![EndpointEvent::SpeechStarted]
        );
    }

    #[test]
    fn pre_roll_is_kept_and_capped() {
        let p = format!("{}{}{}", silence(30), speech(7), silence(19));
        let ev = run(EndpointConfig::default(), &p);
        let EndpointEvent::Utterance { samples, .. } = &ev[1] else {
            panic!("no utterance")
        };
        let ids = frame_ids(samples);
        assert_eq!(ids.len(), 10 + 7 + 19);
        assert_eq!(ids[0], 20); // the ten frames before the first speech frame
    }

    #[test]
    fn dropped_burst_counts_as_pre_roll_for_the_next_utterance() {
        let p = format!("{}{}{}{}", speech(3), silence(1), speech(7), silence(19));
        let ev = run(EndpointConfig::default(), &p);
        let EndpointEvent::Utterance { samples, .. } = &ev[1] else {
            panic!("no utterance")
        };
        assert_eq!(frame_ids(samples)[0], 0);
    }

    #[test]
    fn continuous_speech_is_cut_at_the_maximum_length() {
        let cfg = EndpointConfig {
            max_utterance_ms: 640,
            ..EndpointConfig::default()
        };
        let ev = run(cfg, &speech(40));
        let EndpointEvent::Utterance { samples, forced } = &ev[1] else {
            panic!("no utterance")
        };
        assert!(forced);
        assert_eq!(frame_ids(samples).len(), 20);
    }

    #[test]
    fn a_second_utterance_follows_the_first() {
        let p = format!("{0}{1}{0}{1}", speech(8), silence(19));
        let started = run(EndpointConfig::default(), &p)
            .iter()
            .filter(|e| **e == EndpointEvent::SpeechStarted)
            .count();
        assert_eq!(started, 2);
    }

    #[test]
    fn end_silence_is_configurable() {
        let cfg = EndpointConfig {
            end_silence_ms: 400,
            ..EndpointConfig::default()
        };
        let ev = run(cfg, &format!("{}{}", speech(8), silence(13)));
        assert_eq!(ev.len(), 2);
    }
}
