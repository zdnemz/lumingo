//! The writing workshop engine (S4-12): rule-based findings at once, structured
//! errors and rubric bands when a provider is reachable, a comparison of a
//! revised draft with the earlier one, and a pending queue when it is not.

mod rubric;
mod service;

pub use rubric::{
    Alarm, CrossCheck, Dimension, DimensionResult, DimensionStatus, MAX_FEEDBACK_WORDS,
    PointResult, RUBRIC_SCORE_VERSION, RawDimension, RawPoint, RawRubric, RubricDimension,
    RubricResult, WorkshopRubric, WorkshopTask, cross_check, merge_rerun,
    system_prompt as rubric_system_prompt, user_message as rubric_user_message,
};
pub use service::{
    CompletedDraft, DraftFeedback, DraftStatus, DraftSubmission, PendingReport, Workshop,
    WorkshopConfig, WorkshopEnv, process_pending_drafts,
};
