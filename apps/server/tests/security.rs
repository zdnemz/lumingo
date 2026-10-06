#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Every route group and the WebSocket must refuse a foreign Host, a foreign
//! Origin, and a request without the session cookie (context pack section 10).

mod common;

use axum::http::{Method, StatusCode, header};
use common::{DEV_ORIGIN, EVIL_ORIGIN, Harness, OWN_ORIGIN, Req, body_text, harness, send};
use serde_json::json;
use tower::ServiceExt;

/// One request per route group: the UI, every JSON route (reads and changes), and
/// the WebSocket. The cookie is attached; each test then takes away what it
/// wants to refuse. Ids and bodies are valid, so a request that got through would
/// really act.
fn route_groups(cookie: &str) -> Vec<Req> {
    let mut requests = vec![
        Req::get("/"),
        // State
        Req::get("/api/state"),
        // Curriculum
        Req::get("/api/units"),
        Req::get("/api/units/a1-u01"),
        // Providers
        Req::get("/api/providers"),
        Req::post(
            "/api/providers",
            json!({"name": "x", "protocol": "openai_chat",
                   "base_url": "https://api.example.test/v1", "model": "m"}),
        ),
        Req::post("/api/providers/1/test", json!({})),
        Req::post("/api/providers/1/activate", json!({})),
        Req::delete("/api/providers/1"),
        // Progress
        Req::get("/api/progress"),
        Req::get("/api/attempts/1/evidence"),
        // Game
        Req::get("/api/game"),
        Req::post("/api/game/equip", json!({"slot": "accessory", "id": null})),
        // Settings
        Req::get("/api/settings"),
        Req::put(
            "/api/settings",
            json!({"display_name": "X", "ui_language": "en", "l1": "id", "l1_help_mode": "auto",
                   "adaptive_timing": "auto", "keep_recordings": false}),
        ),
        // Data
        Req::delete("/api/sessions/1"),
        Req::delete("/api/data"),
        Req::get("/api/export"),
        // Diagnostics
        Req::get("/api/diagnostics"),
        Req::get("/api/inspector"),
        // Sessions and turns
        Req::post(
            "/api/sessions",
            json!({"kind": "text_chat", "topic": {"kind": "typed", "text": "food"}}),
        ),
        Req::post("/api/sessions/1/stop", json!({"cancel": false})),
        Req::post("/api/sessions/1/pause", json!({})),
        Req::post("/api/sessions/1/resume", json!({})),
        Req::post("/api/sessions/1/text", json!({"text": "hello"})),
        Req::post("/api/sessions/1/turns/1/edit", json!({"text": "hello"})),
        Req::post("/api/sessions/1/push-to-talk", json!({"pressed": true})),
        Req::post("/api/tutor/stop-speaking", json!({})),
        // Activities
        Req::get("/api/sessions/1/next-activity"),
        Req::post(
            "/api/activities/submit",
            json!({"session_id": 1, "activity_id": "a02-greeting-by-time",
                   "answer": {"kind": "choice", "index": 1}}),
        ),
        // Free modes
        Req::post("/api/writing/1/drafts", json!({"text": "I am Dewi."})),
        Req::post(
            "/api/reading/generate",
            json!({"session_id": 1, "topic": {"kind": "typed", "text": "food"}}),
        ),
        Req::post(
            "/api/reading/1/answers",
            json!({"target": {"kind": "generated", "content_id": 1}, "answers": [0]}),
        ),
        // Speech
        Req::post(
            "/api/tts/speak",
            json!({"source": "turn", "session_id": 1, "turn_seq": 1}),
        ),
        Req::get("/api/audio/devices"),
        Req::post("/api/audio/test", json!({"duration_ms": 500})),
        // Models
        Req::get("/api/models"),
        Req::post(
            "/api/models/stt-test/download",
            json!({"accept_licence": true, "license": "MIT", "license_url": "https://example.org"}),
        ),
        Req::post("/api/models/stt-test/cancel", json!({})),
    ];
    for request in &mut requests {
        request.cookie = Some(cookie.to_owned());
    }
    let mut ws = Req::get("/ws");
    ws.cookie = Some(cookie.to_owned());
    ws.origin = Some(OWN_ORIGIN);
    ws.websocket = true;
    requests.push(ws);
    requests
}

/// Every refusal must also leave the data as it was: a refused `DELETE /api/data`
/// that still deleted would pass a status-only check.
async fn assert_nothing_changed(h: &Harness) {
    assert!(
        h.core.snapshot().active_session.is_none(),
        "a refused request started a session"
    );
    assert!(
        h.core.inspector().entries.is_empty(),
        "a refused request reached the provider"
    );
    assert_eq!(h.core.settings().display_name, "Learner");
    assert_eq!(h.core.list_providers().providers.len(), 1);
    assert!(h.core.game_state().await.unwrap().xp_total > 0);
}

/// A harness with something to lose: a provider, XP and a session.
async fn harness_with_data(dev: bool) -> Harness {
    let h = common::harness_with(common::Options {
        dev,
        with_unit: true,
        // A manager that would start a session if a refused request got through.
        sessions: Some(common::Sessions::text_only(vec![])),
        manifest: Some(common::MANIFEST.to_owned()),
        ..common::Options::default()
    })
    .await;
    let request = serde_json::from_value(json!({
        "name": "mine", "protocol": "openai_chat",
        "base_url": "https://api.example.test/v1", "model": "m", "api_key": "sk-abcdefgh-12345678"
    }))
    .unwrap();
    h.core.save_provider(request).await.unwrap();
    h.core
        .record_practice(app_core::api::XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    h
}

#[tokio::test]
async fn foreign_host_is_refused_everywhere() {
    let h = harness_with_data(false).await;
    for host in [
        "evil.example",
        "127.0.0.1:9999",
        "localhost",
        "evil.example:4321",
    ] {
        for mut req in route_groups(&h.cookie) {
            req.host = Some(host);
            let uri = req.uri.clone();
            let response = send(&h.router, req).await;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{uri} with Host {host}"
            );
        }
    }
    assert_nothing_changed(&h).await;
}

#[tokio::test]
async fn missing_host_is_refused() {
    let h = harness(false).await;
    let mut req = Req::get("/api/state");
    req.host = None;
    req.cookie = Some(h.cookie.clone());
    assert_eq!(send(&h.router, req).await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn foreign_origin_is_refused_everywhere() {
    let h = harness_with_data(false).await;
    for origin in [
        EVIL_ORIGIN,
        "null",
        "http://127.0.0.1:4322",
        "http://localhost:3000",
    ] {
        for mut req in route_groups(&h.cookie) {
            req.origin = Some(origin);
            let uri = req.uri.clone();
            let response = send(&h.router, req).await;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{uri} with Origin {origin}"
            );
        }
    }
    assert_nothing_changed(&h).await;
}

#[tokio::test]
async fn missing_or_wrong_cookie_is_refused_on_api_and_websocket() {
    let h = harness(false).await;
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
async fn every_api_route_refuses_a_missing_or_wrong_cookie_and_changes_nothing() {
    let h = harness_with_data(false).await;
    let mut checked = 0;
    for cookie in [
        None,
        Some("tutor_session=wrong".to_owned()),
        Some("other=1".to_owned()),
    ] {
        // The first entry is the UI page, which is served without a cookie.
        for mut req in route_groups(&h.cookie).into_iter().skip(1) {
            req.cookie.clone_from(&cookie);
            let uri = req.uri.clone();
            let response = send(&h.router, req).await;
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{uri} with {cookie:?}"
            );
            let body = body_text(response).await;
            assert!(body.contains("session_required"), "{uri}: {body}");
            checked += 1;
        }
    }
    assert!(checked >= 3 * 39, "{checked}");
    assert_nothing_changed(&h).await;
}

#[tokio::test]
async fn an_unknown_api_path_is_a_json_not_found_and_still_needs_the_cookie() {
    let h = harness(false).await;
    let response = send(&h.router, Req::get("/api/nothing-here")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let (status, body) = h.get_json("/api/nothing-here").await;
    assert_eq!(status, 404);
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn websocket_upgrade_without_origin_is_refused() {
    let h = harness(false).await;
    let mut req = Req::get("/ws");
    req.cookie = Some(h.cookie.clone());
    req.websocket = true;
    assert_eq!(send(&h.router, req).await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn state_route_answers_with_the_right_headers() {
    let h = harness(false).await;
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
    let h = harness(false).await;
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
    let h = harness(false).await;

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
    let h = harness(false).await;
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
    let h = harness(true).await;

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

/// Every route that changes something refuses a body type a web page can send
/// without a preflight (and a missing one), and acts on nothing.
#[tokio::test]
async fn every_state_changing_route_refuses_a_wrong_content_type() {
    let h = harness_with_data(false).await;
    let mut checked = 0;
    for content_type in [
        None,
        Some("text/plain"),
        Some("application/x-www-form-urlencoded"),
        Some("multipart/form-data"),
    ] {
        for mut req in route_groups(&h.cookie) {
            if req.method == Method::GET {
                continue;
            }
            req.content_type = content_type;
            let uri = req.uri.clone();
            let response = send(&h.router, req).await;
            assert_eq!(
                response.status(),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{uri} with {content_type:?}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 4 * 24, "{checked}");
    assert_nothing_changed(&h).await;
}

/// A body above the limit is refused with 413 before it is read, on every route
/// that takes one, and no session starts.
#[tokio::test]
async fn an_oversized_body_is_refused_with_413_on_the_new_routes() {
    let h = harness_with_data(false).await;
    let big = "x".repeat(tutor_server::MAX_BODY_BYTES + 1);
    for uri in [
        "/api/sessions",
        "/api/sessions/1/stop",
        "/api/sessions/1/text",
        "/api/sessions/1/turns/1/edit",
        "/api/sessions/1/push-to-talk",
        "/api/activities/submit",
        "/api/writing/1/drafts",
        "/api/reading/generate",
        "/api/reading/1/answers",
        "/api/tts/speak",
        "/api/audio/test",
        "/api/models/stt-test/download",
    ] {
        let req = Req::post(uri, json!({ "text": big })).authed(&h.cookie);
        let response = send(&h.router, req).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE, "{uri}");
        let body = body_text(response).await;
        assert!(body.contains("invalid_input"), "{uri}: {body}");
        assert!(!body.contains("xxxx"), "the body is not echoed: {uri}");
    }
    assert_nothing_changed(&h).await;
}
