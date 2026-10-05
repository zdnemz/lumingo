//! Sending one JSON POST and turning every failure into a typed error.

use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER};
use reqwest::{Response, StatusCode, Url};
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::adapter::{Limits, RetryPolicy};
use crate::error::{LlmError, TimeoutKind};
use crate::http::{GuardedClient, map_reqwest_error};
use crate::key::ApiKey;
use crate::redact::sanitize_message;
use crate::types::RateLimit;

/// Largest error body that is read. Providers send a few hundred bytes.
const MAX_ERROR_BODY: usize = 8 * 1024;
/// Largest successful non-streaming reply. A full reading passage is a few KiB.
const MAX_REPLY_BODY: usize = 8 * 1024 * 1024;
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(5);

/// The clock of one call. All deadlines are fixed when the call starts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallBudget {
    pub started: Instant,
    pub total_deadline: Instant,
    pub first_token_deadline: Option<Instant>,
}

impl CallBudget {
    pub(crate) fn start(limits: &Limits) -> Self {
        let started = Instant::now();
        Self {
            started,
            total_deadline: started + limits.total,
            first_token_deadline: limits.first_token.map(|d| started + d.min(limits.total)),
        }
    }

    /// Deadline for the response headers and which limit it stands for. For a
    /// streaming call no token can have arrived before the headers, so the
    /// first-token limit applies to the wait for them too.
    pub(crate) fn header_deadline(&self) -> (Instant, TimeoutKind) {
        match self.first_token_deadline {
            Some(deadline) if deadline < self.total_deadline => (deadline, TimeoutKind::FirstToken),
            _ => (self.total_deadline, TimeoutKind::Total),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Transport {
    http: GuardedClient,
    url: Url,
    headers: HeaderMap,
    key: Option<ApiKey>,
    retry: RetryPolicy,
}

impl Transport {
    pub(crate) fn new(
        http: GuardedClient,
        url: Url,
        headers: HeaderMap,
        key: Option<ApiKey>,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            http,
            url,
            headers,
            key,
            retry,
        }
    }

    fn key_text(&self) -> Option<&str> {
        self.key.as_ref().map(ApiKey::expose)
    }

    /// Sends the body and returns the response when its status is 2xx. Retries follow
    /// `context_pack.md` section 13:
    ///
    /// - A transport error (including a connect timeout) or a 5xx reply is retried
    ///   once after a jittered pause.
    /// - HTTP 429 is retried up to twice, after `Retry-After` when the reply has it
    ///   and after an exponential backoff from 2 s otherwise. A wait that would
    ///   outlast the call's total limit is not taken: the error is returned at once
    ///   with the `Retry-After` value, so the caller can switch to batched mode.
    /// - Every other 4xx, a first-token or total timeout, and a cancellation are
    ///   final. A stream is retried only here, before its first byte of content, never
    ///   after, because a repeated reply would repeat spoken words.
    pub(crate) async fn post(
        &self,
        body: &Value,
        stream: bool,
        budget: &CallBudget,
        cancel: &CancellationToken,
    ) -> Result<Response, LlmError> {
        let mut transport_retries = 0;
        let mut rate_limit_retries = 0;
        loop {
            let error = match self.post_once(body, stream, budget, cancel).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            let pause = match &error {
                LlmError::Transport(_) | LlmError::Timeout(TimeoutKind::Connect)
                    if transport_retries < self.retry.transport_retries =>
                {
                    transport_retries += 1;
                    Some(jittered(self.retry.jitter_base))
                }
                LlmError::Server { .. } if transport_retries < self.retry.transport_retries => {
                    transport_retries += 1;
                    Some(jittered(self.retry.jitter_base))
                }
                LlmError::RateLimited { retry_after }
                    if rate_limit_retries < self.retry.rate_limit_retries =>
                {
                    let backoff = self.retry.rate_limit_backoff * 2u32.pow(rate_limit_retries);
                    rate_limit_retries += 1;
                    Some(retry_after.unwrap_or(backoff))
                }
                _ => None,
            };
            let Some(pause) = pause else {
                return Err(error);
            };
            if budget
                .total_deadline
                .saturating_duration_since(Instant::now())
                <= pause
            {
                return Err(error);
            }
            tracing::debug!(
                target: "llm_client::transport",
                pause_ms = u64::try_from(pause.as_millis()).unwrap_or(u64::MAX),
                "retrying after a failed request"
            );
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
                () = tokio::time::sleep(pause) => {}
            }
        }
    }

    /// Sends the body once and returns the response when its status is 2xx.
    async fn post_once(
        &self,
        body: &Value,
        stream: bool,
        budget: &CallBudget,
        cancel: &CancellationToken,
    ) -> Result<Response, LlmError> {
        let bytes = serde_json::to_vec(body).map_err(|_| {
            LlmError::InvalidRequest("the request body could not be serialised".to_owned())
        })?;
        let mut builder = self
            .http
            .post(&self.url)?
            .headers(self.headers.clone())
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(bytes);
        if stream {
            builder = builder.header(ACCEPT, HeaderValue::from_static("text/event-stream"));
        }

        let (header_deadline, header_kind) = budget.header_deadline();
        let sent = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(LlmError::Cancelled),
            result = tokio::time::timeout_at(header_deadline, builder.send()) => result,
        };
        let response = match sent {
            Err(_elapsed) => return Err(LlmError::Timeout(header_kind)),
            Ok(Err(error)) => return Err(map_reqwest_error(&error)),
            Ok(Ok(response)) => response,
        };
        if response.status().is_success() {
            return Ok(response);
        }
        Err(self.error_from_response(response, budget, cancel).await)
    }

    async fn error_from_response(
        &self,
        response: Response,
        budget: &CallBudget,
        cancel: &CancellationToken,
    ) -> LlmError {
        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        let wait = ERROR_BODY_TIMEOUT.min(
            budget
                .total_deadline
                .saturating_duration_since(Instant::now()),
        );
        let body = tokio::select! {
            biased;
            () = cancel.cancelled() => return LlmError::Cancelled,
            read = tokio::time::timeout(wait, read_limited(response, MAX_ERROR_BODY)) => read.ok().and_then(Result::ok).unwrap_or_default(),
        };
        classify_status(status, retry_after, &body, self.key_text())
    }
}

/// Reads a successful non-streaming body within the call budget.
pub(crate) async fn read_reply(
    response: Response,
    budget: &CallBudget,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, LlmError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(LlmError::Cancelled),
        result = tokio::time::timeout_at(budget.total_deadline, read_limited(response, MAX_REPLY_BODY)) => match result {
            Err(_elapsed) => Err(LlmError::Timeout(TimeoutKind::Total)),
            Ok(Err(error)) => Err(error),
            Ok(Ok(body)) => Ok(body),
        },
    }
}

async fn read_limited(mut response: Response, limit: usize) -> Result<Vec<u8>, LlmError> {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let room = limit.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if body.len() >= limit {
                    return Ok(body);
                }
            }
            Ok(None) => return Ok(body),
            Err(error) => return Err(map_reqwest_error(&error)),
        }
    }
}

/// A pause between half of `base` and one and a half times it. The randomness
/// only has to spread simultaneous retries apart, so the per-instance random keys
/// of the standard library's `RandomState` are enough.
fn jittered(base: Duration) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    let random = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    base / 2 + base.mul_f64((random % 1000) as f64 / 1000.0)
}

/// `retry-after-ms` (sent by some providers) wins over `retry-after` in seconds.
/// The HTTP-date form is not parsed; the caller then falls back to its backoff.
pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    if let Some(ms) = headers
        .get("retry-after-ms")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
    {
        if ms.is_finite() && ms >= 0.0 {
            return Some(Duration::from_secs_f64(ms / 1000.0));
        }
    }
    let seconds = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

pub(crate) fn classify_status(
    status: StatusCode,
    retry_after: Option<Duration>,
    body: &[u8],
    key: Option<&str>,
) -> LlmError {
    let code = status.as_u16();
    let (message, param) = error_text(body);
    let message = sanitize_message(&message, key);
    match code {
        401 | 403 => LlmError::Auth { status: code },
        429 => LlmError::RateLimited { retry_after },
        500..=599 => LlmError::Server {
            status: code,
            message,
        },
        _ => LlmError::Rejected {
            status: code,
            message,
            param: param.map(|p| sanitize_message(&p, key)),
        },
    }
}

/// Pulls the message and the offending parameter out of the error body shapes the
/// providers use: `{"error":{"message","param"}}`, `{"error":"text"}`,
/// `{"message":..}`, and a JSON array holding one of those (Gemini).
fn error_text(body: &[u8]) -> (String, Option<String>) {
    let raw = String::from_utf8_lossy(body);
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return (raw.into_owned(), None);
    };
    let value = match &value {
        Value::Array(items) => items.first().unwrap_or(&Value::Null),
        other => other,
    };
    let error = value.get("error").unwrap_or(value);
    let message = match error {
        Value::String(text) => Some(text.clone()),
        Value::Object(_) => error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    };
    let param = error
        .get("param")
        .and_then(Value::as_str)
        .map(str::to_owned);
    (message.unwrap_or_else(|| raw.into_owned()), param)
}

/// Parses `x-ratelimit` style headers where a provider sends them. Header names
/// differ per provider and OpenAI-compatible servers may send none, so this reads
/// only the names documented by OpenAI and Anthropic; anything else stays `None`.
pub(crate) fn rate_limit_from_headers(headers: &HeaderMap) -> RateLimit {
    fn number(headers: &HeaderMap, name: &str) -> Option<u32> {
        headers.get(name)?.to_str().ok()?.trim().parse().ok()
    }
    RateLimit {
        rpm: number(headers, "anthropic-ratelimit-requests-limit")
            .or_else(|| number(headers, "x-ratelimit-limit-requests")),
        rpd: number(headers, "x-ratelimit-limit-requests-day"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn retry_after_prefers_milliseconds_and_ignores_dates() {
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after", "3")])),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after", "0.5")])),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            parse_retry_after(&headers(&[("retry-after", "3"), ("retry-after-ms", "250")])),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            parse_retry_after(&headers(&[(
                "retry-after",
                "Wed, 21 Oct 2026 07:28:00 GMT"
            )])),
            None
        );
        assert_eq!(parse_retry_after(&headers(&[("retry-after", "-4")])), None);
        assert_eq!(parse_retry_after(&headers(&[])), None);
    }

    #[test]
    fn classifies_statuses() {
        let body =
            br#"{"error":{"message":"Unsupported parameter: 'max_tokens'","param":"max_tokens"}}"#;
        assert!(matches!(
            classify_status(StatusCode::UNAUTHORIZED, None, b"", None),
            LlmError::Auth { status: 401 }
        ));
        assert!(matches!(
            classify_status(StatusCode::FORBIDDEN, None, b"", None),
            LlmError::Auth { status: 403 }
        ));
        assert!(matches!(
            classify_status(StatusCode::TOO_MANY_REQUESTS, Some(Duration::from_secs(2)), b"", None),
            LlmError::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(2)
        ));
        assert!(matches!(
            classify_status(StatusCode::BAD_GATEWAY, None, b"oops", None),
            LlmError::Server { status: 502, .. }
        ));
        match classify_status(StatusCode::BAD_REQUEST, None, body, None) {
            LlmError::Rejected {
                status: 400,
                message,
                param,
            } => {
                assert!(message.contains("max_tokens"));
                assert_eq!(param.as_deref(), Some("max_tokens"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn error_text_reads_the_known_shapes() {
        assert_eq!(error_text(br#"{"error":"plain"}"#).0, "plain");
        assert_eq!(
            error_text(br#"{"type":"error","error":{"type":"x","message":"anthropic style"}}"#).0,
            "anthropic style"
        );
        assert_eq!(
            error_text(br#"[{"error":{"message":"gemini style"}}]"#).0,
            "gemini style"
        );
        assert_eq!(
            error_text(b"<html>gateway</html>").0,
            "<html>gateway</html>"
        );
    }

    #[test]
    fn the_key_never_survives_into_the_error() {
        let body = br#"{"error":{"message":"bad key sk-live-AAAABBBBCCCCDDDD"}}"#;
        let err = classify_status(
            StatusCode::BAD_REQUEST,
            None,
            body,
            Some("sk-live-AAAABBBBCCCCDDDD"),
        );
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("AAAABBBB"), "{shown}");
    }

    #[test]
    fn rate_limit_headers_are_read_by_documented_names() {
        let limit =
            rate_limit_from_headers(&headers(&[("anthropic-ratelimit-requests-limit", "50")]));
        assert_eq!(
            limit,
            RateLimit {
                rpm: Some(50),
                rpd: None
            }
        );
        assert_eq!(rate_limit_from_headers(&headers(&[])), RateLimit::default());
    }
}
