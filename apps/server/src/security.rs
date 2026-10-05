//! Request guard for the loopback server (context pack section 10).
//!
//! A local HTTP server can be called by any page open in the learner's browser
//! unless it refuses. Every defence here exists because that is true.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

/// Name of the session cookie.
pub const COOKIE_NAME: &str = "tutor_session";

#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("the operating system refused to provide random bytes: {0}")]
    Random(String),
}

/// Everything the guard needs to decide. Built once at start-up, after the port is known.
#[derive(Debug)]
pub struct Guard {
    hosts: [String; 2],
    origins: [String; 2],
    dev_origin: Option<String>,
    secret: String,
}

impl Guard {
    /// `dev_origin` is the one extra origin allowed in development mode.
    pub fn new(port: u16, dev_origin: Option<String>) -> Result<Self, GuardError> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|err| GuardError::Random(err.to_string()))?;
        let secret = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            hosts: [format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            origins: [
                format!("http://127.0.0.1:{port}"),
                format!("http://localhost:{port}"),
            ],
            dev_origin,
            secret,
        })
    }

    /// The `Set-Cookie` value that hands the session secret to the UI.
    pub fn set_cookie_value(&self) -> String {
        format!(
            "{COOKIE_NAME}={}; HttpOnly; SameSite=Strict; Path=/",
            self.secret
        )
    }

    fn host_allowed(&self, host: &str) -> bool {
        self.hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
    }

    fn origin_allowed(&self, origin: &str) -> bool {
        self.origins.iter().any(|o| o == origin) || self.dev_origin.as_deref() == Some(origin)
    }

    fn is_dev_origin(&self, origin: &str) -> bool {
        self.dev_origin.as_deref() == Some(origin)
    }

    fn cookie_valid(&self, headers: &HeaderMap) -> bool {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .any(|(name, value)| {
                name == COOKIE_NAME && constant_time_eq(value.as_bytes(), self.secret.as_bytes())
            })
    }
}

/// Compares two byte strings without stopping at the first difference. Length is not secret.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0_u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn refuse(status: StatusCode, reason: &'static str) -> Response {
    let body = format!("{{\"error\":\"{reason}\"}}");
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    apply_security_headers(response.headers_mut());
    response
}

fn is_websocket_upgrade(req: &Request) -> bool {
    req.headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
}

fn needs_cookie(path: &str) -> bool {
    path == "/ws" || path == "/api" || path.starts_with("/api/")
}

/// Headers every response carries.
pub fn apply_security_headers(headers: &mut HeaderMap) {
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if !headers.contains_key(header::CONTENT_SECURITY_POLICY) {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        );
    }
}

fn apply_cors(headers: &mut HeaderMap, origin: &str) {
    if let Ok(value) = HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        headers.append(header::VARY, HeaderValue::from_static("Origin"));
    }
}

/// The middleware. It runs before every route, including the embedded UI.
pub async fn guard(State(guard): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let headers = req.headers();

    let host_ok = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| guard.host_allowed(h));
    if !host_ok {
        return refuse(StatusCode::FORBIDDEN, "host_not_allowed");
    }

    let method = req.method().clone();
    let origin = match headers.get(header::ORIGIN) {
        Some(value) => match value.to_str() {
            Ok(text) => Some(text.to_owned()),
            Err(_) => return refuse(StatusCode::FORBIDDEN, "origin_not_allowed"),
        },
        None => None,
    };
    let changes_state = !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
    match &origin {
        Some(o) if !guard.origin_allowed(o) => {
            return refuse(StatusCode::FORBIDDEN, "origin_not_allowed");
        }
        None if changes_state || is_websocket_upgrade(&req) => {
            return refuse(StatusCode::FORBIDDEN, "origin_required");
        }
        _ => {}
    }
    let cors_origin = origin.as_deref().filter(|o| guard.is_dev_origin(o));

    if method == Method::OPTIONS && headers.contains_key(header::ACCESS_CONTROL_REQUEST_METHOD) {
        let Some(allowed) = cors_origin else {
            return refuse(StatusCode::FORBIDDEN, "origin_not_allowed");
        };
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NO_CONTENT;
        let out = response.headers_mut();
        apply_cors(out, allowed);
        out.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PUT, DELETE"),
        );
        out.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type"),
        );
        out.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        apply_security_headers(out);
        return response;
    }

    if needs_cookie(req.uri().path()) && !guard.cookie_valid(headers) {
        return refuse(StatusCode::UNAUTHORIZED, "session_required");
    }

    if changes_state && !is_json_content_type(headers) {
        return refuse(StatusCode::UNSUPPORTED_MEDIA_TYPE, "json_required");
    }

    let is_api = needs_cookie(req.uri().path());
    let cors_origin = cors_origin.map(str::to_owned);
    let mut response = next.run(req).await;

    let out = response.headers_mut();
    if let Some(allowed) = cors_origin {
        apply_cors(out, &allowed);
    }
    let is_html = out
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if is_html && let Ok(cookie) = HeaderValue::from_str(&guard.set_cookie_value()) {
        out.append(header::SET_COOKIE, cookie);
    }
    if is_api {
        out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    apply_security_headers(out);
    response
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_eq_matches_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
