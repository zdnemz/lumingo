//! Tutoring logic. Handlers in `apps/server` call into this crate and never
//! contain tutoring, scoring or prompt logic themselves.
#![forbid(unsafe_code)]

mod analysis;
mod chunker;
mod drafts;
mod error;
mod practice;
mod prompt;
mod review;
mod session;
mod support;

pub use analysis::*;
pub use chunker::SentenceChunker;
pub use drafts::{DraftComparison, DraftError, EarlierError, Resolution, compare_drafts};
pub use error::{EngineError, Result};
pub use practice::*;
pub use prompt::{
    FALLBACK_LINE, FeedbackMode, Focus, HISTORY_MESSAGES, MAX_NOTES, PronFinding, ReplyLimits,
    TUTOR_TURN_VERSION, TutorContext, bounded_history, reply_limits, system_prompt, user_message,
};
pub use review::{Grade, ReviewState, due_order, update_mastery};
pub use session::{
    Channel, EndReason, EngineFault, Event, Phase, Session, SessionKind, TransitionError, TurnState,
};
pub use support::{Clock, system_clock};
