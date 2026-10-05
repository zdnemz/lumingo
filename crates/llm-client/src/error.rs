//! Typed errors for every call. No variant carries key material: transport
//! errors are reduced to a fixed category (a `reqwest` error can hold the URL),
//! provider messages pass through `redact::sanitize_message`, and validator
//! output keeps only JSON pointers because it can echo model text.

use std::fmt;
use std::time::Duration;

use thiserror::Error;

use crate::types::LadderLevel;

/// Which of the three time limits of `context_pack.md` section 13 fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Connect,
    FirstToken,
    Total,
}

impl fmt::Display for TimeoutKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Connect => "connect",
            Self::FirstToken => "first token",
            Self::Total => "total",
        })
    }
}

/// Coarse network failure category. The original error is dropped on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Connect,
    Tls,
    Timeout,
    Body,
    Other,
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Connect => "could not connect",
            Self::Tls => "TLS failure",
            Self::Timeout => "timed out",
            Self::Body => "the connection broke while reading the reply",
            Self::Other => "request failed",
        })
    }
}

/// Why a structured output was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidReason {
    /// No complete JSON value in the reply.
    NotJson,
    /// JSON that does not match the contract schema.
    SchemaMismatch,
}

/// Details of an `invalid_output` result. `paths` are JSON pointers into the
/// model's reply; the validator's messages are not kept because they echo values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidOutput {
    pub reason: InvalidReason,
    pub paths: Vec<String>,
    pub ladder_level: LadderLevel,
    pub repaired: bool,
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("the call was cancelled")]
    Cancelled,
    #[error("the provider did not answer in time ({0} limit)")]
    Timeout(TimeoutKind),
    #[error("network error: {0}")]
    Transport(TransportKind),
    #[error("the provider refused the credentials (HTTP {status})")]
    Auth { status: u16 },
    #[error("the provider is rate limiting requests{}", retry_after_suffix(*.retry_after))]
    RateLimited { retry_after: Option<Duration> },
    #[error("provider error (HTTP {status}): {message}")]
    Server { status: u16, message: String },
    #[error("the provider rejected the request (HTTP {status}): {message}")]
    Rejected {
        status: u16,
        message: String,
        /// The `error.param` field when the provider names the offending parameter.
        param: Option<String>,
    },
    #[error("the model declined to answer")]
    Refusal,
    #[error("invalid_output at ladder level {} ({} problem(s), repaired: {})", .0.ladder_level.as_u8(), .0.paths.len(), .0.repaired)]
    InvalidOutput(InvalidOutput),
    #[error("unexpected reply from the provider: {0}")]
    Protocol(String),
    /// The provider reported an error inside an event stream that had started
    /// with HTTP 200 (for example Anthropic's `overloaded_error` event).
    #[error("the provider reported an error in the stream: {message}")]
    Stream { message: String },
    #[error("host {host} is not on the allowlist")]
    HostNotAllowed { host: String },
    #[error("plain HTTP is only allowed for loopback hosts, not {host}")]
    InsecureScheme { host: String },
    #[error("this request cannot be sent: {0}")]
    InvalidRequest(String),
}

fn retry_after_suffix(retry_after: Option<Duration>) -> String {
    match retry_after {
        Some(d) => format!(" (retry after {} s)", d.as_secs()),
        None => String::new(),
    }
}

impl LlmError {
    /// True for failures of the provider connection that the application should
    /// show as "provider unreachable" or "rate limited" rather than as a bad answer.
    pub fn is_provider_unavailable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::Transport(_) | Self::Server { .. } | Self::RateLimited { .. }
        )
    }
}
