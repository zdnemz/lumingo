use std::path::PathBuf;

use thiserror::Error;

use crate::manifest::{ManifestError, NotDownloadable};

#[derive(Debug, Error)]
pub enum ModelError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("no model with id {0:?} in the manifest")]
    UnknownModel(String),
    #[error(transparent)]
    NotDownloadable(#[from] NotDownloadable),
    #[error("the license of {model_id} must be shown and accepted before it is downloaded")]
    LicenceNotAccepted { model_id: String },
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("the download could not start: {0}")]
    Client(String),
    #[error("{url} answered with HTTP status {status}")]
    Status { url: String, status: u16 },
    #[error("the server sent an answer that cannot be resumed from: {0}")]
    BadResponse(String),
    /// The connection ended before the whole file arrived. The partial file is
    /// kept, so the next attempt resumes from it.
    #[error("the transfer ended after {received} bytes of {}; run it again to resume", expected.map_or_else(|| "an unknown size".to_owned(), |e| format!("{e}")))]
    Interrupted {
        received: u64,
        expected: Option<u64>,
    },
    #[error("no data arrived for {seconds} seconds; run it again to resume")]
    Stalled { seconds: u64 },
    #[error("{path}: expected SHA-256 {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("{path}: the download grew past its declared size of {limit} bytes")]
    TooLarge { path: String, limit: u64 },
    #[error("the download was cancelled")]
    Cancelled,
    #[error("the installed-model record {path} is damaged: {reason}")]
    Record { path: PathBuf, reason: String },
}

impl ModelError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
