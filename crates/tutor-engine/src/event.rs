//! The events one session publishes for the UI (`context_pack.md` section 3.5,
//! session and turn subset).
//!
//! These are the payloads, not the wire messages: the server adds the sequence
//! number and the session id when it forwards them, and the TypeScript types
//! are generated from the server's own types, never from these. The engine
//! produces them in the order a turn happens, so a caller can forward them
//! as they arrive.

use crate::session::{Phase, TurnState};

/// One thing that happened in a session. The variants mirror the session and
/// turn events of `context_pack.md` section 3.5; the server adds the sequence
/// number and the session id when it forwards them, and the TypeScript types
/// are generated from the server's own types, never from these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// The session started, paused, resumed, lost its provider, hit an engine
    /// fault, or ended (`Phase::Ended` carries the reason).
    SessionState { phase: Phase },
    /// The turn moved to another phase.
    TurnState { turn: TurnState },
    /// The learner's final transcript or typed message. On the voice channel
    /// `edited` is true when the learner corrected the transcript by hand.
    TranscriptFinal { text: String, edited: bool },
    /// A piece of the tutor's reply, as it streams in.
    TutorTextDelta { delta: String },
    /// One complete sentence of the reply, in the order it is spoken.
    TutorSentenceSpoken { index: u32, text: String },
}

impl UiEvent {
    /// The session-state event for a phase, with no other payload.
    pub fn session_state(phase: Phase) -> Self {
        Self::SessionState { phase }
    }

    /// The turn-state event for a turn state.
    pub fn turn_state(turn: TurnState) -> Self {
        Self::TurnState { turn }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{EndReason, EngineFault, Phase};

    #[test]
    fn constructors_make_the_expected_variants() {
        assert_eq!(
            UiEvent::session_state(Phase::Paused),
            UiEvent::SessionState {
                phase: Phase::Paused
            }
        );
        assert_eq!(
            UiEvent::turn_state(TurnState::Thinking),
            UiEvent::TurnState {
                turn: TurnState::Thinking
            }
        );
        // An ended session is a session-state event, not a variant of its own.
        assert_eq!(
            UiEvent::session_state(Phase::Ended {
                reason: EndReason::Cancelled
            }),
            UiEvent::SessionState {
                phase: Phase::Ended {
                    reason: EndReason::Cancelled
                }
            }
        );
        let fault = UiEvent::session_state(Phase::EngineError {
            fault: EngineFault::Playback,
        });
        assert!(matches!(
            fault,
            UiEvent::SessionState {
                phase: Phase::EngineError { .. }
            }
        ));
    }
}
