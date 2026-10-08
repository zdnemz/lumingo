//! The error type of the orchestration services.

use llm_client::LlmError;
use storage::StorageError;

use crate::activity::ActivityError;
use crate::session::TransitionError;

/// Why a service call failed. A provider that is down is an `Llm` error; the
/// services that can carry on without a provider (analysis, writing drafts)
/// handle that case themselves and only return it when the caller needs to know.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("language model: {0}")]
    Llm(#[from] LlmError),
    #[error("{0}")]
    Transition(#[from] TransitionError),
    #[error("activity: {0}")]
    Activity(#[from] ActivityError),
    /// The caller cancelled the call, for work that is not a model call: a
    /// pronunciation analysis. A cancelled model call is `Llm(Cancelled)`.
    #[error("the call was cancelled")]
    Cancelled,
    /// The model's output passed its schema but could not be read into the typed form.
    /// The message names the contract and never carries model text.
    #[error("the {0} output did not fit its typed form")]
    Output(&'static str),
    /// The caller asked for something the session or content does not allow.
    #[error("{0}")]
    Refused(&'static str),
}

pub type Result<T> = std::result::Result<T, EngineError>;
