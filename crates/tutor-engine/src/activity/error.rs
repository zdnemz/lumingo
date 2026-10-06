//! Why an activity refused a response or a request.
//!
//! These are mistakes of the caller (a response of the wrong shape, a replay
//! that is not allowed), never a low score: a wrong answer is scored, a malformed
//! one is refused before anything is stored.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivityError {
    #[error("the response does not fit this activity: it needs {expected}")]
    WrongKind { expected: &'static str },
    #[error("the response has {got} entries and the activity has {expected}")]
    WrongLength { expected: usize, got: usize },
    #[error("choice {index} is outside the {options} options")]
    OutOfRange { index: usize, options: usize },
    #[error("the response is empty")]
    Empty,
    #[error("the response is longer than {max} {unit}")]
    TooLong { max: usize, unit: &'static str },
    #[error("this activity is scored by {0}, not by a deterministic scorer")]
    NotDeterministic(&'static str),
    #[error("no replays are left: {allowed} are allowed")]
    NoReplaysLeft { allowed: u8 },
    #[error("the unit has no activity {0}")]
    UnknownActivity(String),
    #[error("the activity {0} was already answered in this run")]
    AlreadyAnswered(String),
    #[error("the activity {0} has no audio to play")]
    NoAudio(String),
    #[error("the roleplay allows {max} learner turns and they are used")]
    NoTurnsLeft { max: u8 },
    #[error("the roleplay is not open: open it before the first learner turn")]
    NotOpen,
}
