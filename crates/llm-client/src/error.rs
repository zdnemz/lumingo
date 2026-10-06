use std::time::Duration;

/// A stream event that could not be understood. The message names the problem
/// and never repeats the payload, so a key-like string in a server reply cannot
/// reach a log through this type.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("stream event is not valid JSON")]
    InvalidJson,
    #[error("stream event is missing `{0}`")]
    Missing(&'static str),
}

/// Every failure a call can end in. No variant holds a key, a URL with
/// credentials or a raw server body: messages go through `redact` first.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("invalid base URL: {0}")]
    InvalidBaseUrl(&'static str),
    #[error("host is not on the allowlist")]
    ForbiddenHost,
    #[error("the provider refused the credentials (HTTP {0})")]
    Auth(u16),
    #[error("rate limited by the provider")]
    RateLimited { retry_after: Option<Duration> },
    #[error("provider returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("network error: {0}")]
    Transport(String),
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    #[error("cancelled")]
    Cancelled,
    #[error("the output did not match the schema, even after one repair (ladder level {level})")]
    InvalidOutput { level: u8 },
    #[error(transparent)]
    Parse(#[from] ParseError),
}
