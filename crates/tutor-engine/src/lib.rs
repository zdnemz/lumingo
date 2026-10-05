//! Tutoring logic. Handlers in `apps/server` call into this crate and never
//! contain tutoring, scoring or prompt logic themselves.
#![forbid(unsafe_code)]

mod chunker;
mod drafts;
mod prompt;
mod review;
mod session;

pub use chunker::SentenceChunker;
pub use drafts::{DraftComparison, DraftError, EarlierError, Resolution, compare_drafts};
pub use prompt::{
    FALLBACK_LINE, FeedbackMode, Focus, HISTORY_MESSAGES, MAX_NOTES, PronFinding, ReplyLimits,
    TUTOR_TURN_VERSION, TutorContext, bounded_history, reply_limits, system_prompt, user_message,
};
pub use review::{Grade, ReviewState, due_order, update_mastery};
pub use session::{
    Channel, EndReason, EngineFault, Event, Phase, Session, SessionKind, TransitionError, TurnState,
};
