//! The session state machine (`context_pack.md` section 9).
//!
//! Pure transitions: an event goes in and either the session moves to a new
//! phase or the event is refused and nothing changes. The caller owns the
//! engines, the storage and the language model; this module only decides what
//! may happen next.
//!
//! A voice turn walks `Listening -> Transcribing -> Thinking -> Speaking` and
//! back to `Listening`. A text turn walks `Waiting -> Thinking -> Replying` and
//! back to `Waiting`. A session that opens with the tutor's turn goes from the
//! idle state to `Thinking` (`Event::OpeningTurn`) before the opening reply.
//! Both can be `Paused`, lose the provider (`ProviderUnavailable`), hit an
//! engine fault (`EngineError`) or end.

/// The kind of session. The channel is chosen separately, because a lesson can
/// mix a spoken roleplay with typed answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Where the learner's turn arrives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Voice,
    Text,
}

/// Where a turn is. Voice and text turns use different members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Which engine failed, for the `EngineError` phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineFault {
    Microphone,
    SpeechRecognition,
    SpeechSynthesis,
    Playback,
}

/// Why the session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndReason {
    Finished,
    Cancelled,
}

/// The session's phase: an active turn, or one of the side states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Active { turn: TurnState },
    Paused,
    ProviderUnavailable,
    EngineError { fault: EngineFault },
    Ended { reason: EndReason },
}

/// What can happen to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    /// The voice activity engine found the end of an utterance.
    UtteranceEnded,
    /// The recogniser returned a final transcript.
    TranscriptReady,
    /// The learner sent a typed message.
    TextSent,
    /// The tutor starts the conversation without learner input (T1's trigger:
    /// session start when the tutor speaks first). Valid while the session waits
    /// for the learner, on either channel.
    OpeningTurn,
    /// The reply began: the first words arrived, or the first sentence is playing.
    ReplyStarted,
    /// The reply is complete and, on the voice channel, has finished playing.
    ReplyFinished,
    /// The learner pressed stop while the tutor was speaking or replying.
    StopSpeaking,
    Pause,
    Resume,
    /// The provider failed and the client's own retry failed too.
    ProviderFailed,
    ProviderRecovered,
    EngineFailed(EngineFault),
    EngineRecovered,
    Finish,
    Cancel,
}

/// An event that is not allowed in the session's current phase. The session is
/// unchanged when this is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the event {event:?} is not allowed while the session is {from:?}")]
pub struct TransitionError {
    pub from: Phase,
    pub event: Event,
}

/// One session of any kind. Cheap to clone and compare; the caller keeps the
/// storage id next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    kind: SessionKind,
    channel: Channel,
    phase: Phase,
    turns_completed: u32,
}

impl Session {
    /// A fresh session waits for the learner: `Listening` on the voice channel,
    /// `Waiting` on the text channel.
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

    pub fn kind(&self) -> SessionKind {
        self.kind
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The turn state when the session is active, `None` in a side state.
    pub fn turn(&self) -> Option<TurnState> {
        match self.phase {
            Phase::Active { turn } => Some(turn),
            _ => None,
        }
    }

    /// Turns that ran to `ReplyFinished`. A stopped reply does not count.
    pub fn turns_completed(&self) -> u32 {
        self.turns_completed
    }

    pub fn is_ended(&self) -> bool {
        matches!(self.phase, Phase::Ended { .. })
    }

    /// Applies an event and returns the new phase, or refuses it and leaves the
    /// session exactly as it was.
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

    fn idle(channel: Channel) -> TurnState {
        match channel {
            Channel::Voice => TurnState::Listening,
            Channel::Text => TurnState::Waiting,
        }
    }

    fn next_phase(&self, event: Event) -> Option<Phase> {
        use Channel::{Text, Voice};
        use TurnState::{Listening, Replying, Speaking, Thinking, Transcribing, Waiting};

        let active = |turn| Some(Phase::Active { turn });
        let idle = Self::idle(self.channel);

        match (self.phase, event, self.channel) {
            // An ended session accepts nothing.
            (Phase::Ended { .. }, _, _) => None,

            // Ending is possible from every other phase.
            (_, Event::Finish, _) => Some(Phase::Ended {
                reason: EndReason::Finished,
            }),
            (_, Event::Cancel, _) => Some(Phase::Ended {
                reason: EndReason::Cancelled,
            }),

            // Voice turn: Listening -> Transcribing -> Thinking -> Speaking -> Listening.
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

            // Text turn: Waiting -> Thinking -> Replying -> Waiting.
            (Phase::Active { turn: Waiting }, Event::TextSent, Text) => active(Thinking),
            (Phase::Active { turn: Thinking }, Event::ReplyStarted, Text) => active(Replying),
            (
                Phase::Active { turn: Replying },
                Event::ReplyFinished | Event::StopSpeaking,
                Text,
            ) => active(Waiting),

            // A typed message is accepted while a voice session listens, because
            // a lesson can mix a spoken roleplay with typed answers.
            (Phase::Active { turn: Listening }, Event::TextSent, Voice) => active(Thinking),

            // The tutor speaks first (T1's trigger). Only from the idle state:
            // once a turn is in flight the opening turn is refused.
            (
                Phase::Active {
                    turn: Listening | Waiting,
                },
                Event::OpeningTurn,
                _,
            ) => active(Thinking),

            // A reply that ended before it ever started (an empty or refused
            // answer replaced by the authored line) still completes the turn.
            (Phase::Active { turn: Thinking }, Event::ReplyFinished, _) => active(idle),

            // Pause stops whatever is in flight; resume waits for the learner again.
            (Phase::Active { .. }, Event::Pause, _) => Some(Phase::Paused),
            (Phase::Paused, Event::Resume, _) => active(idle),

            // The provider is used while the reply is being made, and a call in
            // flight can fail while the session is paused.
            (
                Phase::Active {
                    turn: Thinking | Speaking | Replying,
                },
                Event::ProviderFailed,
                _,
            )
            | (Phase::Paused, Event::ProviderFailed, _) => Some(Phase::ProviderUnavailable),
            (Phase::ProviderUnavailable, Event::ProviderRecovered, _) => active(idle),

            // An engine fault can happen while the session is active or paused,
            // and while it waits for the provider to come back.
            (
                Phase::Active { .. } | Phase::Paused | Phase::ProviderUnavailable,
                Event::EngineFailed(fault),
                _,
            ) => Some(Phase::EngineError { fault }),
            (Phase::EngineError { .. }, Event::EngineRecovered, _) => active(idle),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EndReason::{Cancelled, Finished};
    use EngineFault::Microphone;
    use Phase::{Paused, ProviderUnavailable};
    use TurnState::{Listening, Replying, Speaking, Thinking, Transcribing, Waiting};

    fn voice() -> Session {
        Session::new(SessionKind::Conversation, Channel::Voice)
    }

    fn text() -> Session {
        Session::new(SessionKind::TextChat, Channel::Text)
    }

    fn active(turn: TurnState) -> Phase {
        Phase::Active { turn }
    }

    #[test]
    fn a_voice_turn_walks_through_its_states_and_counts_as_one() {
        let mut s = voice();
        assert_eq!(s.phase(), active(Listening));
        s.apply(Event::UtteranceEnded).unwrap();
        assert_eq!(s.turn(), Some(Transcribing));
        s.apply(Event::TranscriptReady).unwrap();
        assert_eq!(s.turn(), Some(Thinking));
        s.apply(Event::ReplyStarted).unwrap();
        assert_eq!(s.turn(), Some(Speaking));
        assert_eq!(s.turns_completed(), 0);
        s.apply(Event::ReplyFinished).unwrap();
        assert_eq!(s.turn(), Some(Listening));
        assert_eq!(s.turns_completed(), 1);
    }

    #[test]
    fn a_text_turn_walks_through_its_states_and_counts_as_one() {
        let mut s = text();
        assert_eq!(s.phase(), active(Waiting));
        s.apply(Event::TextSent).unwrap();
        assert_eq!(s.turn(), Some(Thinking));
        s.apply(Event::ReplyStarted).unwrap();
        assert_eq!(s.turn(), Some(Replying));
        s.apply(Event::ReplyFinished).unwrap();
        assert_eq!(s.turn(), Some(Waiting));
        assert_eq!(s.turns_completed(), 1);
    }

    #[test]
    fn the_opening_turn_goes_from_idle_to_thinking_on_both_channels() {
        for mut s in [voice(), text()] {
            assert_eq!(s.phase(), active(Session::idle(s.channel())));
            s.apply(Event::OpeningTurn).unwrap();
            assert_eq!(s.turn(), Some(Thinking));
            s.apply(Event::ReplyStarted).unwrap();
            let speaking = if s.channel() == Channel::Voice {
                Speaking
            } else {
                Replying
            };
            assert_eq!(s.turn(), Some(speaking));
            s.apply(Event::ReplyFinished).unwrap();
            assert_eq!(s.phase(), active(Session::idle(s.channel())));
            assert_eq!(s.turns_completed(), 1);
        }
    }

    #[test]
    fn an_opening_turn_is_refused_once_a_turn_is_in_flight() {
        let mut s = voice();
        s.apply(Event::UtteranceEnded).unwrap();
        assert!(s.apply(Event::OpeningTurn).is_err());
        s.apply(Event::TranscriptReady).unwrap();
        assert!(s.apply(Event::OpeningTurn).is_err());
        let mut s = text();
        s.apply(Event::TextSent).unwrap();
        assert!(s.apply(Event::OpeningTurn).is_err());
    }

    #[test]
    fn a_typed_message_is_accepted_while_a_voice_session_listens() {
        let mut s = voice();
        s.apply(Event::TextSent).unwrap();
        assert_eq!(s.turn(), Some(Thinking));
        s.apply(Event::ReplyStarted).unwrap();
        assert_eq!(s.turn(), Some(Speaking));
    }

    #[test]
    fn a_reply_that_never_started_still_completes_the_turn() {
        for mut s in [voice(), text()] {
            if s.channel() == Channel::Voice {
                s.apply(Event::UtteranceEnded).unwrap();
                s.apply(Event::TranscriptReady).unwrap();
            } else {
                s.apply(Event::TextSent).unwrap();
            }
            assert_eq!(s.turn(), Some(Thinking));
            s.apply(Event::ReplyFinished).unwrap();
            assert_eq!(s.phase(), active(Session::idle(s.channel())));
            assert_eq!(s.turns_completed(), 1);
        }
    }

    #[test]
    fn stop_speaking_returns_to_idle_without_completing_the_turn() {
        let mut s = voice();
        s.apply(Event::UtteranceEnded).unwrap();
        s.apply(Event::TranscriptReady).unwrap();
        s.apply(Event::ReplyStarted).unwrap();
        s.apply(Event::StopSpeaking).unwrap();
        assert_eq!(s.turn(), Some(Listening));
        assert_eq!(s.turns_completed(), 0);

        let mut s = text();
        s.apply(Event::TextSent).unwrap();
        s.apply(Event::ReplyStarted).unwrap();
        s.apply(Event::StopSpeaking).unwrap();
        assert_eq!(s.turn(), Some(Waiting));
        assert_eq!(s.turns_completed(), 0);
    }

    #[test]
    fn pause_and_resume_work_from_every_active_state() {
        let mut s = voice();
        s.apply(Event::UtteranceEnded).unwrap();
        s.apply(Event::Pause).unwrap();
        assert_eq!(s.phase(), Paused);
        // Pause again, and a turn event, are refused while paused.
        assert!(s.apply(Event::Pause).is_err());
        assert!(s.apply(Event::UtteranceEnded).is_err());
        s.apply(Event::Resume).unwrap();
        assert_eq!(s.phase(), active(Listening));
        assert!(s.apply(Event::Resume).is_err());
    }

    #[test]
    fn provider_failure_only_counts_while_the_reply_is_being_made() {
        let mut s = text();
        assert!(s.apply(Event::ProviderFailed).is_err());
        s.apply(Event::TextSent).unwrap();
        s.apply(Event::ProviderFailed).unwrap();
        assert_eq!(s.phase(), ProviderUnavailable);
        s.apply(Event::ProviderRecovered).unwrap();
        assert_eq!(s.phase(), active(Waiting));
    }

    #[test]
    fn an_engine_fault_can_happen_whenever_the_session_has_not_ended() {
        let mut s = voice();
        s.apply(Event::EngineFailed(Microphone)).unwrap();
        assert_eq!(s.phase(), Phase::EngineError { fault: Microphone });
        assert!(s.apply(Event::UtteranceEnded).is_err());
        s.apply(Event::EngineRecovered).unwrap();
        assert_eq!(s.phase(), active(Listening));
        // While paused, and while the provider is away, too.
        s.apply(Event::Pause).unwrap();
        s.apply(Event::EngineFailed(Microphone)).unwrap();
        assert_eq!(s.phase(), Phase::EngineError { fault: Microphone });
        s.apply(Event::EngineRecovered).unwrap();
        s.apply(Event::TextSent).unwrap();
        s.apply(Event::ProviderFailed).unwrap();
        s.apply(Event::EngineFailed(Microphone)).unwrap();
        assert_eq!(s.phase(), Phase::EngineError { fault: Microphone });
    }

    #[test]
    fn finish_and_cancel_end_the_session_from_any_phase() {
        let mut s = text();
        s.apply(Event::TextSent).unwrap();
        s.apply(Event::Finish).unwrap();
        assert_eq!(s.phase(), Phase::Ended { reason: Finished });
        assert!(s.is_ended());
        let mut s = voice();
        s.apply(Event::Pause).unwrap();
        s.apply(Event::Cancel).unwrap();
        assert_eq!(s.phase(), Phase::Ended { reason: Cancelled });
    }

    #[test]
    fn an_ended_session_accepts_nothing() {
        let mut s = text();
        s.apply(Event::Finish).unwrap();
        for event in [Event::TextSent, Event::Resume, Event::Cancel, Event::Finish] {
            assert!(s.apply(event).is_err());
        }
        assert_eq!(s.phase(), Phase::Ended { reason: Finished });
    }

    #[test]
    fn events_out_of_turn_are_refused_and_change_nothing() {
        let mut s = voice();
        let before = s.phase();
        for event in [
            Event::TranscriptReady,
            Event::ReplyStarted,
            Event::ReplyFinished,
            Event::StopSpeaking,
            Event::Resume,
        ] {
            let error = s.apply(event).unwrap_err();
            assert_eq!(s.phase(), before);
            assert_eq!(error.from, before);
            assert_eq!(error.event, event);
        }
        let mut s = text();
        assert!(s.apply(Event::UtteranceEnded).is_err());
        s.apply(Event::TextSent).unwrap();
        s.apply(Event::ReplyStarted).unwrap();
        assert!(s.apply(Event::TextSent).is_err());
        assert!(s.apply(Event::UtteranceEnded).is_err());
    }
}
