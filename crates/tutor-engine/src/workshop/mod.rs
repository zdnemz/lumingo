//! The writing workshop engine (S4-12): rule-based findings at once, structured
//! errors and rubric bands when a provider is reachable, a comparison of a
//! revised draft with the earlier one, and a pending queue when it is not.
//!
//! The rubric prompt, the typed reply and the cross-checks are shared with the
//! scored activities and live in [`crate::rubric`].

mod service;

pub use service::{
    CompletedDraft, DraftFeedback, DraftStatus, DraftSubmission, PendingReport, Workshop,
    WorkshopConfig, WorkshopEnv, process_pending_drafts,
};
