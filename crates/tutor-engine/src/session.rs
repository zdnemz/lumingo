//! The session state machine. Pure transitions: events go in, a new phase comes
//! out or the event is refused. The orchestrator that owns engines and the
//! language model drives it, and tests drive it directly.
//!
//! Voice turns run Listening, Transcribing, Thinking, Speaking, then back to
//! Listening. Text turns run Waiting, Thinking, Replying, then back to Waiting.
//! A session can also be Paused, find its provider unavailable, hit an engine
//! error, or be Ended.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Lesson,
    Conversation,
    TextChat,
    Writing,
    Reading,
    Drill,
    Review,
    Checkpoint,
    Placement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Voice,
    Text,
}

/// Where a turn is. Voice and text use different members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    // Voice
    Listening,
    Transcribing,
    Speaking,
    // Text
    Waiting,
    Replying,
    // Both
    Thinking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineFault {
    Microphone,
    SpeechRecognition,
    SpeechSynthesis,
    Playback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    Active { turn: TurnState },
    Paused,
    ProviderUnavailable,
    EngineError { fault: EngineFault },
    Ended { reason: EndReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    /// The voice activity engine found the end of an utterance.
    UtteranceEnded,
    /// The recogniser returned a transcript, possibly after the learner edited it.
    TranscriptReady,
    /// The learner sent a typed message.
    TextSent,
    /// The reply began: the first sentence is playing, or the first words are on screen.
    ReplyStarted,
    /// The reply is complete and, for voice, has finished playing.
    ReplyFinished,
    /// The learner pressed stop while the tutor was speaking.
    StopSpeaking,
    Pause,
    Resume,
    /// The provider failed and the one retry failed too.
    ProviderFailed,
    ProviderRecovered,
    EngineFailed(EngineFault),
    EngineRecovered,
    Finish,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the event {event:?} is not allowed while the session is {from:?}")]
pub struct TransitionError {
    pub from: Phase,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    kind: SessionKind,
    channel: Channel,
    phase: Phase,
    turns_completed: u32,
}

impl Session {
    /// A new session waits for the learner: Listening for voice, Waiting for text.
    pub fn new(kind: SessionKind, channel: Channel) -> Self {
        Self {
            kind,
            channel,
            phase: Phase::Active {
                turn: Self::idle(channel),
            },
            turns_completed: 0,
        }
    }

    fn idle(channel: Channel) -> TurnState {
        match channel {
            Channel::Voice => TurnState::Listening,
            Channel::Text => TurnState::Waiting,
        }
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }
    pub fn channel(&self) -> Channel {
        self.channel
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn turns_completed(&self) -> u32 {
        self.turns_completed
    }

    pub fn is_ended(&self) -> bool {
        matches!(self.phase, Phase::Ended { .. })
    }

    /// Applies an event and returns the new phase, or refuses it and leaves the
    /// session as it was.
    pub fn apply(&mut self, event: Event) -> Result<Phase, TransitionError> {
        let next = self.next_phase(event).ok_or(TransitionError {
            from: self.phase,
            event,
        })?;
        if matches!(event, Event::ReplyFinished) {
            self.turns_completed += 1;
        }
        self.phase = next;
        Ok(next)
    }

    fn next_phase(&self, event: Event) -> Option<Phase> {
        use Channel::{Text, Voice};
        use TurnState::{Listening, Replying, Speaking, Thinking, Transcribing, Waiting};

        let active = |turn| Some(Phase::Active { turn });
        let idle = Self::idle(self.channel);

        match (self.phase, event, self.channel) {
            // A session that has ended accepts nothing.
            (Phase::Ended { .. }, _, _) => None,

            // Ending is possible from every other phase.
            (_, Event::Finish, _) => Some(Phase::Ended {
                reason: EndReason::Finished,
            }),
            (_, Event::Cancel, _) => Some(Phase::Ended {
                reason: EndReason::Cancelled,
            }),

            // Voice turn.
            (Phase::Active { turn: Listening }, Event::UtteranceEnded, Voice) => {
                active(Transcribing)
            }
            (Phase::Active { turn: Transcribing }, Event::TranscriptReady, Voice) => {
                active(Thinking)
            }
            (Phase::Active { turn: Thinking }, Event::ReplyStarted, Voice) => active(Speaking),
            (
                Phase::Active { turn: Speaking },
                Event::ReplyFinished | Event::StopSpeaking,
                Voice,
            ) => active(Listening),
            // A typed message is allowed wherever speaking is.
            (Phase::Active { turn: Listening }, Event::TextSent, Voice) => active(Thinking),

            // Text turn.
            (Phase::Active { turn: Waiting }, Event::TextSent, Text) => active(Thinking),
            (Phase::Active { turn: Thinking }, Event::ReplyStarted, Text) => active(Replying),
            (Phase::Active { turn: Replying }, Event::ReplyFinished, Text) => active(Waiting),

            // A reply that ends without ever starting (an empty or refused answer
            // is replaced by a fallback line by the caller) still completes the turn.
            (Phase::Active { turn: Thinking }, Event::ReplyFinished, _) => active(idle),

            // Pause stops whatever is in flight. Resume goes back to waiting for the learner.
            (Phase::Active { .. }, Event::Pause, _) => Some(Phase::Paused),
            (Phase::Paused, Event::Resume, _) => active(idle),

            // Provider trouble: only while the model is being used.
            (
                Phase::Active {
                    turn: Thinking | Speaking | Replying,
                },
                Event::ProviderFailed,
                _,
            ) => Some(Phase::ProviderUnavailable),
            (Phase::ProviderUnavailable, Event::ProviderRecovered, _) => active(idle),
            (Phase::Paused, Event::ProviderFailed, _) => Some(Phase::ProviderUnavailable),

            // Engine trouble can happen in any active turn.
            (Phase::Active { .. }, Event::EngineFailed(fault), _) => {
                Some(Phase::EngineError { fault })
            }
            (Phase::EngineError { .. }, Event::EngineRecovered, _) => active(idle),
            (Phase::ProviderUnavailable, Event::EngineFailed(fault), _) => {
                Some(Phase::EngineError { fault })
            }

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice() -> Session {
        Session::new(SessionKind::Conversation, Channel::Voice)
    }
    fn text() -> Session {
        Session::new(SessionKind::TextChat, Channel::Text)
    }
    fn turn(state: TurnState) -> Phase {
        Phase::Active { turn: state }
    }

    #[test]
    fn a_voice_session_starts_listening_and_a_text_session_starts_waiting() {
        assert_eq!(voice().phase(), turn(TurnState::Listening));
        assert_eq!(text().phase(), turn(TurnState::Waiting));
    }

    #[test]
    fn a_voice_turn_runs_the_whole_circle_and_counts_the_turn() {
        let mut s = voice();
        let path = [
            (Event::UtteranceEnded, TurnState::Transcribing),
            (Event::TranscriptReady, TurnState::Thinking),
            (Event::ReplyStarted, TurnState::Speaking),
            (Event::ReplyFinished, TurnState::Listening),
        ];
        for (event, expected) in path {
            assert_eq!(s.apply(event), Ok(turn(expected)), "{event:?}");
        }
        assert_eq!(s.turns_completed(), 1);
    }

    #[test]
    fn a_text_turn_runs_the_whole_circle() {
        let mut s = text();
        for (event, expected) in [
            (Event::TextSent, TurnState::Thinking),
            (Event::ReplyStarted, TurnState::Replying),
            (Event::ReplyFinished, TurnState::Waiting),
        ] {
            assert_eq!(s.apply(event), Ok(turn(expected)), "{event:?}");
        }
        assert_eq!(s.turns_completed(), 1);
    }

    #[test]
    fn text_is_allowed_wherever_speaking_is() {
        let mut s = voice();
        assert_eq!(s.apply(Event::TextSent), Ok(turn(TurnState::Thinking)));
    }

    #[test]
    fn stopping_the_tutor_returns_to_listening_without_counting_a_finished_turn() {
        let mut s = voice();
        for event in [
            Event::UtteranceEnded,
            Event::TranscriptReady,
            Event::ReplyStarted,
        ] {
            s.apply(event).expect("legal");
        }
        assert_eq!(s.apply(Event::StopSpeaking), Ok(turn(TurnState::Listening)));
        assert_eq!(s.turns_completed(), 0);
    }

    #[test]
    fn a_fallback_reply_that_never_started_still_completes_the_turn() {
        let mut s = text();
        s.apply(Event::TextSent).expect("legal");
        assert_eq!(s.apply(Event::ReplyFinished), Ok(turn(TurnState::Waiting)));
        assert_eq!(s.turns_completed(), 1);
    }

    #[test]
    fn pause_works_from_any_active_turn_and_resume_returns_to_waiting_for_the_learner() {
        for (mut s, idle) in [
            (voice(), TurnState::Listening),
            (text(), TurnState::Waiting),
        ] {
            s.apply(if s.channel() == Channel::Voice {
                Event::UtteranceEnded
            } else {
                Event::TextSent
            })
            .expect("legal");
            assert_eq!(s.apply(Event::Pause), Ok(Phase::Paused));
            assert_eq!(s.apply(Event::Resume), Ok(turn(idle)));
        }
    }

    #[test]
    fn a_provider_failure_while_the_model_is_in_use_moves_to_unavailable_and_recovers() {
        let mut s = text();
        s.apply(Event::TextSent).expect("legal");
        assert_eq!(
            s.apply(Event::ProviderFailed),
            Ok(Phase::ProviderUnavailable)
        );
        assert_eq!(
            s.apply(Event::ProviderRecovered),
            Ok(turn(TurnState::Waiting))
        );
    }

    #[test]
    fn a_provider_failure_while_idle_is_refused() {
        let mut s = text();
        assert!(s.apply(Event::ProviderFailed).is_err());
        assert_eq!(
            s.phase(),
            turn(TurnState::Waiting),
            "a refused event changes nothing"
        );
    }

    #[test]
    fn an_engine_fault_stops_the_turn_and_names_the_engine() {
        let mut s = voice();
        s.apply(Event::UtteranceEnded).expect("legal");
        assert_eq!(
            s.apply(Event::EngineFailed(EngineFault::SpeechRecognition)),
            Ok(Phase::EngineError {
                fault: EngineFault::SpeechRecognition
            })
        );
        assert_eq!(
            s.apply(Event::EngineRecovered),
            Ok(turn(TurnState::Listening))
        );
    }

    #[test]
    fn finish_and_cancel_end_the_session_from_every_phase() {
        let starts: Vec<Vec<Event>> = vec![
            vec![],
            vec![Event::UtteranceEnded],
            vec![Event::Pause],
            vec![
                Event::UtteranceEnded,
                Event::TranscriptReady,
                Event::ProviderFailed,
            ],
            vec![Event::EngineFailed(EngineFault::Microphone)],
        ];
        for setup in starts {
            for (event, reason) in [
                (Event::Finish, EndReason::Finished),
                (Event::Cancel, EndReason::Cancelled),
            ] {
                let mut s = voice();
                for e in &setup {
                    s.apply(*e).expect("legal setup");
                }
                assert_eq!(
                    s.apply(event),
                    Ok(Phase::Ended { reason }),
                    "{setup:?} then {event:?}"
                );
                assert!(s.is_ended());
            }
        }
    }

    #[test]
    fn an_ended_session_accepts_nothing() {
        let mut s = text();
        s.apply(Event::Finish).expect("legal");
        for event in [
            Event::TextSent,
            Event::Resume,
            Event::Finish,
            Event::Cancel,
            Event::Pause,
        ] {
            assert!(s.apply(event).is_err(), "{event:?}");
        }
    }

    #[test]
    fn voice_events_are_refused_in_a_text_session_and_text_ones_in_waiting_voice() {
        let mut t = text();
        assert!(t.apply(Event::UtteranceEnded).is_err());
        assert!(t.apply(Event::TranscriptReady).is_err());
        let mut v = voice();
        v.apply(Event::UtteranceEnded).expect("legal");
        assert!(
            v.apply(Event::TextSent).is_err(),
            "typing is only taken while listening"
        );
    }

    #[test]
    fn out_of_order_events_are_refused_and_change_nothing() {
        let mut s = voice();
        for event in [
            Event::TranscriptReady,
            Event::ReplyStarted,
            Event::ReplyFinished,
            Event::Resume,
        ] {
            let before = s.phase();
            assert!(s.apply(event).is_err(), "{event:?}");
            assert_eq!(s.phase(), before);
        }
        assert_eq!(s.turns_completed(), 0);
    }

    #[test]
    fn the_error_names_what_was_refused() {
        let mut s = text();
        let error = s.apply(Event::ReplyStarted).expect_err("refused");
        assert_eq!(error.event, Event::ReplyStarted);
        assert_eq!(error.from, turn(TurnState::Waiting));
        assert!(error.to_string().contains("ReplyStarted"));
    }

    #[test]
    fn every_session_kind_can_be_created_on_both_channels() {
        for kind in [
            SessionKind::Lesson,
            SessionKind::Conversation,
            SessionKind::TextChat,
            SessionKind::Writing,
            SessionKind::Reading,
            SessionKind::Drill,
            SessionKind::Review,
            SessionKind::Checkpoint,
            SessionKind::Placement,
        ] {
            for channel in [Channel::Voice, Channel::Text] {
                let s = Session::new(kind, channel);
                assert_eq!((s.kind(), s.channel()), (kind, channel));
            }
        }
    }
}
