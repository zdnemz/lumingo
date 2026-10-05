//! Tutoring logic. Handlers in `apps/server` call into this crate and never
//! contain tutoring, scoring or prompt logic themselves.
#![forbid(unsafe_code)]

mod chunker;
mod review;

pub use chunker::SentenceChunker;
pub use review::{Grade, ReviewState, due_order, update_mastery};
