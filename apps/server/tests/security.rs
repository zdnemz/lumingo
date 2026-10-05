#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Every route group and the WebSocket must refuse a foreign Host, a foreign
//! Origin, and a request without the session cookie (context pack section 10).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tutor_server::events::EventHub;
use tutor_server::security::Guard;
use tutor_server::{AppState, build_router};

const PORT: u16 = 4321;
const HOST: &str = "127.0.0.1:4321";
const OWN_ORIGIN: &str = "http://127.0.0.1:4321";
const DEV_ORIGIN: &str = "http://localhost:3000";
const EVIL_ORIGIN: &str = "https://evil.example";

struct Harness {
    router: Router,
    cookie: String,
}

fn harness(dev: bool) -> Harness {
    let dev_origin = dev.then(|| DEV_ORIGIN.to_owned());
    let guard = Arc::new(Guard::new(PORT, dev_origin).expect("guard"));
    let cookie = guard
        .set_cookie_value()
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    let state = Arc::new(AppState {
        hub: EventHub::new(dev),
        guard,
        shutdown: CancellationToken::new(),
        dev_mode: dev,
    });
    Harness {
        router: build_router(state),
        cookie,
    }
}

struct Req {
    method: Method,
    uri: &'static str,
    host: Option<&'static str>,
    origin: Option<&'static str>,
    cookie: Option<String>,
    content_type: Option<&'static str>,
    websocket: bool,
}

impl Req {
    fn get(uri: &'static str) -> Self {
        Self {
            method: Method::GET,
            uri,
            host: Some(HOST),
            origin: None,
            cookie: None,
            content_type: None,
            websocket: false,
        }
    }

    fn build(self) -> Request<Body> {
        let mut builder = Request::builder().method(self.method).uri(self.uri);
        if let Some(host) = self.host {
            builder = builder.header(header::HOST, host);
        }
        if let Some(origin) = self.origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(cookie) = self.cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if let Some(content_type) = self.content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        if self.websocket {
            builder = builder
                .header(header::UPGRADE, "websocket")
                .header(header::CONNECTION, "Upgrade")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==");
        }
        builder.body(Body::empty()).expect("request")
    }
}

async fn send(router: &Router, req: Req) -> Response<Body> {
    router.clone().oneshot(req.build()).await.expect("response")
}

async fn body_text(response: Response<Body>) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// One request per route group: the UI, the JSON API, and the WebSocket.
fn route_groups(cookie: &str) -> Vec<Req> {
    let mut ui = Req::get("/");
    ui.cookie = Some(cookie.to_owned());
    let mut api = Req::get("/api/state");
    api.cookie = Some(cookie.to_owned());
    let mut ws = Req::get("/ws");
    ws.cookie = Some(cookie.to_owned());
    ws.origin = Some(OWN_ORIGIN);
    ws.websocket = true;
    vec![ui, api, ws]
}

#[tokio::test]
async fn foreign_host_is_refused_everywhere() {
    let h = harness(false);
    for host in [
        "evil.example",
        "127.0.0.1:9999",
        "localhost",
        "evil.example:4321",
    ] {
        for mut req in route_groups(&h.cookie) {
            req.host = Some(host);
            let uri = req.uri;
            let response = send(&h.router, req).await;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{uri} with Host {host}"
            );
        }
    }
}

#[tokio::test]
async fn missing_host_is_refused() {
    let h = harness(false);
    let mut req = Req::get("/api/state");
    req.host = None;
    req.cookie = Some(h.cookie.clone());
    assert_eq!(send(&h.router, req).await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn foreign_origin_is_refused_everywhere() {
    let h = harness(false);
    for origin in [
        EVIL_ORIGIN,
        "null",
        "http://127.0.0.1:4322",
        "http://localhost:3000",
    ] {
        for mut req in route_groups(&h.cookie) {
            req.origin = Some(origin);
            let uri = req.uri;
            let response = send(&h.router, req).await;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{uri} with Origin {origin}"
            );
        }
    }
}

#[tokio::test]
async fn missing_or_wrong_cookie_is_refused_on_api_and_websocket() {
    let h = harness(false);
    for uri in ["/api/state", "/api/anything", "/ws"] {
        for cookie in [
            None,
            Some("tutor_session=wrong".to_owned()),
            Some("other=1".to_owned()),
        ] {
            let mut req = Req::get(uri);
            req.cookie = cookie;
            if uri == "/ws" {
                req.origin = Some(OWN_ORIGIN);
                req.websocket = true;
            }
            let response = send(&h.router, req).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
        }
    }
}

#[tokio::test]
async fn websocket_upgrade_without_origin_is_refused() {
    let h = harness(false);
    let mut req = Req::get("/ws");
    req.cookie = Some(h.cookie.clone());
    req.websocket = true;
    assert_eq!(send(&h.router, req).await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn state_route_answers_with_the_right_headers() {
    let h = harness(false);
    let mut req = Req::get("/api/state");
    req.cookie = Some(h.cookie.clone());
    let response = send(&h.router, req).await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .expect("csp")
            .contains("frame-ancestors 'none'")
    );
    assert!(
        !headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        "no CORS headers in normal mode"
    );
    let body = body_text(response).await;
    assert!(body.contains("\"server_version\""), "{body}");
    assert!(body.contains("\"dev_mode\":false"), "{body}");
}

#[tokio::test]
async fn html_sets_a_strict_http_only_session_cookie() {
    let h = harness(false);
    let response = send(&h.router, Req::get("/")).await;
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .expect("cookie")
        .to_owned();
    assert!(cookie.starts_with("tutor_session="), "{cookie}");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    // 256 bits of randomness, written as 64 hex characters.
    let value = cookie
        .split(';')
        .next()
        .expect("pair")
        .trim_start_matches("tutor_session=");
    assert_eq!(value.len(), 64);
    assert!(value.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn state_changing_requests_need_origin_and_json() {
    let h = harness(false);

    let mut no_origin = Req::get("/api/state");
    no_origin.method = Method::POST;
    no_origin.cookie = Some(h.cookie.clone());
    no_origin.content_type = Some("application/json");
    assert_eq!(
        send(&h.router, no_origin).await.status(),
        StatusCode::FORBIDDEN
    );

    for content_type in [
        None,
        Some("text/plain"),
        Some("application/x-www-form-urlencoded"),
        Some("multipart/form-data"),
    ] {
        let mut req = Req::get("/api/state");
        req.method = Method::POST;
        req.cookie = Some(h.cookie.clone());
        req.origin = Some(OWN_ORIGIN);
        req.content_type = content_type;
        assert_eq!(
            send(&h.router, req).await.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{content_type:?}"
        );
    }

    // A correct request reaches the router, which has no POST handler here.
    let mut ok = Req::get("/api/state");
    ok.method = Method::POST;
    ok.cookie = Some(h.cookie.clone());
    ok.origin = Some(OWN_ORIGIN);
    ok.content_type = Some("application/json; charset=utf-8");
    assert_eq!(
        send(&h.router, ok).await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn normal_mode_has_no_cors_and_no_dev_route() {
    let h = harness(false);
    let mut preflight = Req::get("/api/state");
    preflight.method = Method::OPTIONS;
    preflight.origin = Some(DEV_ORIGIN);
    let response = send(&h.router, preflight).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );

    let response = send(&h.router, Req::get("/dev/session")).await;
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json"));
    assert!(
        !is_json,
        "the development session route must not exist in normal mode"
    );
}

#[tokio::test]
async fn dev_mode_allows_exactly_one_extra_origin() {
    let h = harness(true);

    let mut preflight = Req::get("/api/state");
    preflight.method = Method::OPTIONS;
    preflight.origin = Some(DEV_ORIGIN);
    let mut preflight = preflight.build();
    preflight.headers_mut().insert(
        header::ACCESS_CONTROL_REQUEST_METHOD,
        "POST".parse().expect("value"),
    );
    let response = h.router.clone().oneshot(preflight).await.expect("response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        DEV_ORIGIN
    );
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
        "true"
    );

    let mut allowed = Req::get("/api/state");
    allowed.origin = Some(DEV_ORIGIN);
    allowed.cookie = Some(h.cookie.clone());
    let response = send(&h.router, allowed).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        DEV_ORIGIN
    );

    let mut evil = Req::get("/api/state");
    evil.origin = Some(EVIL_ORIGIN);
    evil.cookie = Some(h.cookie.clone());
    let response = send(&h.router, evil).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );

    let mut session = Req::get("/dev/session");
    session.origin = Some(DEV_ORIGIN);
    let response = send(&h.router, session).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .expect("cookie")
            .starts_with("tutor_session=")
    );
}
