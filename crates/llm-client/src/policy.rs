//! Where requests may go and when a failed one is tried again.

use crate::LlmError;
use reqwest::Url;
use std::time::Duration;

/// Parses a provider base URL. HTTPS is required, except for loopback hosts
/// (a local model server). Credentials inside the URL are refused.
pub fn parse_base_url(raw: &str) -> Result<Url, LlmError> {
    let url = Url::parse(raw.trim()).map_err(|_| LlmError::InvalidBaseUrl("not a URL"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(LlmError::InvalidBaseUrl(
            "credentials in the URL are not allowed",
        ));
    }
    let host = url.host_str().ok_or(LlmError::InvalidBaseUrl("no host"))?;
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        "http" => Err(LlmError::InvalidBaseUrl(
            "plain HTTP is only allowed for loopback hosts",
        )),
        _ => Err(LlmError::InvalidBaseUrl("scheme must be https")),
    }
}

/// The outbound allowlist: the active provider's host, and nothing else.
pub fn check_allowed_host(url: &Url, allowed: &[&str]) -> Result<(), LlmError> {
    match url.host_str() {
        Some(h) if allowed.iter().any(|a| a.eq_ignore_ascii_case(h)) => Ok(()),
        _ => Err(LlmError::ForbiddenHost),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Connection or read failure before a usable response.
    Transport,
    Status {
        code: u16,
        retry_after: Option<Duration>,
    },
}

/// Longest `Retry-After` the client waits for inside one tutor turn (the turn has 30 s).
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);
const DEFAULT_429_WAIT: Duration = Duration::from_secs(1);

/// `attempt` counts tries already made. A failed call is retried once: transport
/// errors and 5xx after a short jittered pause, 429 after `Retry-After`. Any other
/// 4xx is final. `jitter` is a value in [0, 1).
pub fn retry_wait(attempt: u32, failure: Failure, jitter: f64) -> Option<Duration> {
    if attempt >= 1 {
        return None;
    }
    let jittered = Duration::from_millis(200 + (jitter.clamp(0.0, 1.0) * 300.0) as u64);
    match failure {
        Failure::Transport => Some(jittered),
        Failure::Status {
            code: 429,
            retry_after,
        } => {
            let wait = retry_after.unwrap_or(DEFAULT_429_WAIT);
            (wait <= MAX_RETRY_WAIT).then_some(wait)
        }
        Failure::Status { code, .. } if (500..600).contains(&code) => Some(jittered),
        Failure::Status { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_and_loopback_http_are_accepted_everything_else_is_not() {
        assert!(parse_base_url("https://api.example.com/v1").is_ok());
        assert!(parse_base_url("http://127.0.0.1:8080/v1").is_ok());
        assert!(parse_base_url("http://localhost:11434").is_ok());
        assert!(parse_base_url("http://[::1]:1234").is_ok());
        for bad in [
            "http://api.example.com",
            "ftp://x.com",
            "https://user:pw@x.com",
            "nonsense",
            "http://127.0.0.1.evil.com",
        ] {
            assert!(parse_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_the_provider_host_is_allowed() {
        let url = Url::parse("https://api.example.com/v1").unwrap();
        assert!(check_allowed_host(&url, &["api.example.com"]).is_ok());
        assert!(check_allowed_host(&url, &["API.EXAMPLE.COM"]).is_ok());
        assert!(matches!(
            check_allowed_host(&url, &["other.com"]),
            Err(LlmError::ForbiddenHost)
        ));
    }

    #[test]
    fn transport_errors_and_5xx_retry_once_with_a_short_jittered_wait() {
        for f in [
            Failure::Transport,
            Failure::Status {
                code: 503,
                retry_after: None,
            },
        ] {
            let w = retry_wait(0, f, 0.5).unwrap();
            assert!((200..=500).contains(&(w.as_millis() as u64)), "{w:?}");
            assert_eq!(retry_wait(1, f, 0.5), None);
        }
    }

    #[test]
    fn a_429_follows_retry_after_and_gives_up_on_long_waits() {
        let s = |retry_after| Failure::Status {
            code: 429,
            retry_after,
        };
        assert_eq!(
            retry_wait(0, s(Some(Duration::from_secs(3))), 0.0),
            Some(Duration::from_secs(3))
        );
        assert_eq!(retry_wait(0, s(None), 0.0), Some(DEFAULT_429_WAIT));
        assert_eq!(retry_wait(0, s(Some(Duration::from_secs(60))), 0.0), None);
        assert_eq!(retry_wait(1, s(Some(Duration::ZERO)), 0.0), None);
    }

    #[test]
    fn other_client_errors_are_final() {
        for code in [400, 401, 403, 404, 422] {
            assert_eq!(
                retry_wait(
                    0,
                    Failure::Status {
                        code,
                        retry_after: None
                    },
                    0.0
                ),
                None
            );
        }
    }
}
