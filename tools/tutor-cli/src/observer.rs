//! Watching the voice loop's events: printing them and keeping what a run needs
//! to know afterwards.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use app_core::voice::{StopCause, TurnLatency, TurnOutcome, VoiceEvent};
use tokio::sync::broadcast;
use tutor_engine::{EngineFault, Phase, TurnState};

/// One finished turn of a run.
#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub turn: u64,
    /// `opening`, `text` or `audio`.
    pub input: &'static str,
    pub outcome: TurnOutcome,
    pub latency: Option<TurnLatency>,
}

#[derive(Default)]
struct State {
    records: Vec<TurnRecord>,
    /// What the next learner turn is, set by whoever sends it.
    next_input: &'static str,
    kinds: HashMap<u64, &'static str>,
    latencies: HashMap<u64, TurnLatency>,
    learner_turns_ended: usize,
    provider_message: Option<String>,
    engine_errors: Vec<String>,
}

/// Cloneable view of the events seen so far.
#[derive(Clone, Default)]
pub struct Observer {
    state: Arc<Mutex<State>>,
}

pub fn outcome_name(outcome: TurnOutcome) -> &'static str {
    match outcome {
        TurnOutcome::Replied => "replied",
        TurnOutcome::Fallback => "fallback",
        TurnOutcome::Stopped => "stopped",
        TurnOutcome::ProviderUnavailable => "provider_unavailable",
        TurnOutcome::EngineError => "engine_error",
    }
}

fn fault_name(fault: EngineFault) -> &'static str {
    match fault {
        EngineFault::Microphone => "microphone",
        EngineFault::SpeechRecognition => "speech recognition",
        EngineFault::SpeechSynthesis => "speech synthesis",
        EngineFault::Playback => "audio output",
    }
}

fn part(d: Option<Duration>) -> String {
    d.map_or_else(|| "-".to_owned(), |d| format!("{} ms", d.as_millis()))
}

/// One line with the parts of a turn's latency. A part that was not on the
/// turn's path is shown as `-`.
pub fn latency_line(latency: &TurnLatency) -> String {
    let p = &latency.parts;
    let mut line = format!(
        "latency  endpointing {} | stt {} | llm {} | tts {} | output {} | sum {} ms",
        part(p.endpointing_wait),
        part(p.stt_finalise),
        part(p.llm_first_sentence),
        part(p.tts_first_sentence),
        part(p.output_start),
        latency.sum().as_millis(),
    );
    if !latency.is_complete() {
        line.push_str(" (partial: - is a part this turn did not go through)");
    }
    line
}

impl Observer {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts printing and recording. `show_states` also prints every phase.
    pub fn start(mut rx: broadcast::Receiver<VoiceEvent>, show_states: bool) -> Self {
        let observer = Self::default();
        observer.lock().next_input = "text";
        let task = observer.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => task.on_event(&event, show_states),
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        eprintln!("(the display fell behind and missed {missed} events)");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        observer
    }

    /// Says what kind of turn the next learner message is, for the result file.
    pub fn set_next_input(&self, kind: &'static str) {
        self.lock().next_input = kind;
    }

    /// Turns that have ended, whatever the outcome.
    pub fn ended(&self) -> usize {
        self.lock().records.len()
    }

    /// Learner turns that have ended: the opening turn is not one.
    pub fn learner_turns_ended(&self) -> usize {
        self.lock().learner_turns_ended
    }

    pub fn records(&self) -> Vec<TurnRecord> {
        self.lock().records.clone()
    }

    /// The learner message of the provider failure, once there was one.
    pub fn provider_unavailable(&self) -> Option<String> {
        self.lock().provider_message.clone()
    }

    pub fn engine_errors(&self) -> Vec<String> {
        self.lock().engine_errors.clone()
    }

    fn on_event(&self, event: &VoiceEvent, show_states: bool) {
        match event {
            VoiceEvent::State(phase) if show_states => println!("[{}]", phase_name(*phase)),
            VoiceEvent::State(_) | VoiceEvent::SpeechStarted | VoiceEvent::Closed => {}
            VoiceEvent::Heard { turn, text } => {
                let mut state = self.lock();
                let kind = state.next_input;
                state.kinds.insert(*turn, kind);
                println!("you>    {text}");
            }
            VoiceEvent::TutorSentence { text, .. } => println!("tutor>  {text}"),
            VoiceEvent::Latency(latency) => {
                self.lock().latencies.insert(latency.turn, *latency);
                println!("{}", latency_line(latency));
            }
            VoiceEvent::Stopped(cause) => println!(
                "(stopped: {})",
                match cause {
                    StopCause::Command => "by command",
                    StopCause::PushToTalk => "push-to-talk",
                    StopCause::Speech => "you started speaking",
                }
            ),
            VoiceEvent::TurnEnded { turn, outcome } => {
                let mut state = self.lock();
                let input = state.kinds.get(turn).copied().unwrap_or("opening");
                if input != "opening" {
                    state.learner_turns_ended += 1;
                }
                let latency = state.latencies.get(turn).copied();
                state.records.push(TurnRecord {
                    turn: *turn,
                    input,
                    outcome: *outcome,
                    latency,
                });
            }
            VoiceEvent::ProviderUnavailable { message } => {
                eprintln!("provider unavailable: {message}");
                self.lock().provider_message = Some(message.clone());
            }
            VoiceEvent::EngineError { fault, message } => {
                let text = format!("{}: {message}", fault_name(*fault));
                eprintln!("problem with {text}");
                self.lock().engine_errors.push(text);
            }
        }
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Active { turn } => match turn {
            TurnState::Listening => "listening",
            TurnState::Transcribing => "transcribing",
            TurnState::Thinking => "thinking",
            TurnState::Speaking => "speaking",
            TurnState::Waiting => "waiting",
            TurnState::Replying => "replying",
        },
        Phase::Paused => "paused",
        Phase::ProviderUnavailable => "provider unavailable",
        Phase::EngineError { .. } => "engine error",
        Phase::Ended { .. } => "ended",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_core::voice::{LatencyParts, TurnLatency};

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    #[test]
    fn a_complete_turn_prints_every_part_and_the_sum() {
        let line = latency_line(&TurnLatency {
            turn: 2,
            parts: LatencyParts {
                endpointing_wait: ms(608),
                stt_finalise: ms(700),
                llm_first_sentence: ms(900),
                tts_first_sentence: ms(300),
                output_start: ms(50),
            },
        });
        assert_eq!(
            line,
            "latency  endpointing 608 ms | stt 700 ms | llm 900 ms | tts 300 ms | output 50 ms | sum 2558 ms"
        );
    }

    #[test]
    fn a_partial_turn_marks_the_missing_parts_and_says_so() {
        let line = latency_line(&TurnLatency {
            turn: 1,
            parts: LatencyParts {
                llm_first_sentence: ms(900),
                ..LatencyParts::default()
            },
        });
        assert!(
            line.contains("endpointing - | stt - | llm 900 ms | tts - | output - | sum 900 ms")
        );
        assert!(line.contains("partial"));
    }
}
