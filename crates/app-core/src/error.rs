//! The one error type of the core, and how it reaches the browser.
//!
//! Every message that leaves the core through [`CoreError::body`] is written
//! here or comes from a crate that promises key-free, value-free messages
//! (`storage`, `llm-client`). Storage and internal failures are logged with their
//! cause and answered with a fixed sentence, so a path or a row value cannot
//! travel to the UI by accident.

use crate::api::{ApiErrorBody, ErrorCode, Feature};

/// Why a command or a query could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The thing asked for does not exist.
    #[error("{what} not found")]
    NotFound { what: &'static str },
    /// The request is well formed JSON but its content is not acceptable.
    #[error("{0}")]
    InvalidInput(String),
    /// The request is fine, but the current state does not allow it.
    #[error("{0}")]
    Conflict(String),
    /// The target is read-only, for example the `env` provider profile.
    #[error("{0}")]
    ReadOnly(&'static str),
    /// A single-slot operation is already running; the second caller is refused
    /// instead of queued.
    #[error("another request of this kind is still running")]
    Busy,
    /// The feature belongs to a part of the program that is not built into this
    /// executable yet.
    #[error("{} is not available in this build", .0.label())]
    NotAvailable(Feature),
    /// A feature needs the active provider and there is none.
    #[error("no provider is configured")]
    ProviderNotConfigured,
    /// The program is stopping and takes no new work.
    #[error("the program is shutting down")]
    ShuttingDown,
    /// The database refused or failed the operation.
    #[error(transparent)]
    Storage(#[from] storage::StorageError),
    /// Anything else, such as a failed file operation. The text is for the log
    /// only; the browser gets a fixed sentence.
    #[error("{0}")]
    Internal(String),
}

/// The result type of the core.
pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    /// The machine-readable code the UI switches on.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::Conflict(_) => ErrorCode::Conflict,
            Self::ReadOnly(_) => ErrorCode::ReadOnly,
            Self::Busy => ErrorCode::Busy,
            Self::NotAvailable(_) => ErrorCode::NotAvailable,
            Self::ProviderNotConfigured => ErrorCode::ProviderNotConfigured,
            Self::ShuttingDown => ErrorCode::ShuttingDown,
            Self::Storage(error) => match error {
                storage::StorageError::NotFound { .. } => ErrorCode::NotFound,
                storage::StorageError::Rule(_) => ErrorCode::InvalidInput,
                storage::StorageError::Constraint(_) => ErrorCode::Conflict,
                _ => ErrorCode::Storage,
            },
            Self::Internal(_) => ErrorCode::Internal,
        }
    }

    /// The body to send. Logs the cause of failures the learner cannot act on.
    pub fn body(&self) -> ApiErrorBody {
        let message = match self {
            Self::Storage(error) => match error {
                storage::StorageError::NotFound { what } => format!("{what} not found"),
                storage::StorageError::Rule(rule) => (*rule).to_owned(),
                storage::StorageError::Constraint(_) => {
                    "the request conflicts with data that already exists".to_owned()
                }
                other => {
                    tracing::warn!(error = %other, "a database operation failed");
                    "the database could not complete the request".to_owned()
                }
            },
            Self::Internal(cause) => {
                tracing::warn!(error = %cause, "an internal operation failed");
                "the program could not complete the request".to_owned()
            }
            other => other.to_string(),
        };
        ApiErrorBody {
            error: self.code(),
            message,
        }
    }

    /// Wraps an I/O failure. Only the kind is kept, because the message can hold a path.
    pub fn io(what: &'static str, error: &std::io::Error) -> Self {
        Self::Internal(format!("{what} failed ({:?})", error.kind()))
    }
}
