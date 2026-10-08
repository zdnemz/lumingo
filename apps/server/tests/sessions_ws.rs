#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! A session driven over HTTP while a WebSocket client watches: the order of the
//! events, the one-session rule, and a client that connects in the middle.

mod common;

use std::sync::Arc;
use std::time::Duration;

use app_core::voice::testing::{ReplyScript, Step};
use axum::Router;
use axum::http::Method;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tutor_server::security::Guard;
use tutor_server::{AppState, build_router};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Running {
    port: u16,
    router: Router,
    cookie: String,
    state: Arc<AppState>,
    _dir: tempfile::TempDir,
}

fn reply(text: &str) -> Step {
    Step::Reply(ReplyScript::new(&[text]))
}

async fn start(steps: Vec<Step>) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let guard = Arc::new(Guard::new(port, None).expect("guard"));
    let (core, dir) = common::open_core(&common::Options::default()).await;
    common::attach_sessions(&core, common::Sessions::text_only(steps)).await;
    let state = Arc::new(AppState::new(core, guard));
    let app = build_router(Arc::clone(&state));
    let router = app.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let cookie = session_cookie(port).await;
    Running {
        port,
        router,
        cookie,
        state,
        _dir: dir,
    }
}

async fn session_cookie(port: u16) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let request = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    let text = String::from_utf8_lossy(&raw);
    let line = text
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .expect("Set-Cookie header on the HTML page");
    line.split_once(':')
        .expect("header")
        .1
        .trim()
        .split(';')
        .next()
        .expect("pair")
        .to_owned()
}

impl Running {
    /// One JSON request through the guard and the router, as the UI sends it.
    async fn http(&self, method: Method, uri: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = common::Req::json(method, uri, body).authed(&self.cookie);
        req.host = Some(Box::leak(
            format!("127.0.0.1:{}", self.port).into_boxed_str(),
        ));
        req.origin = Some(Box::leak(
            format!("http://127.0.0.1:{}", self.port).into_boxed_str(),
        ));
        let response = common::send(&self.router, req).await;
        (
            response.status().as_u16(),
            common::body_json(response).await,
        )
    }

    async fn post(&self, uri: &str, body: Value) -> (u16, Value) {
        self.http(Method::POST, uri, Some(body)).await
    }

    async fn connect(&self) -> Ws {
        let mut request = format!("ws://127.0.0.1:{}/ws", self.port)
            .into_client_request()
            .expect("request");
        request.headers_mut().insert(
            "Origin",
            format!("http://127.0.0.1:{}", self.port).parse().unwrap(),
        );
        request
            .headers_mut()
            .insert("Cookie", self.cookie.parse().unwrap());
        connect_async(request).await.expect("handshake").0
    }
}

async fn next_json(ws: &mut Ws) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("timed out waiting for a frame")
        .expect("stream ended")
        .expect("frame");
    match frame {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("json"),
        other => panic!("unexpected frame {other:?}"),
    }
}

/// Reads frames until `done` accepts one, and returns every frame read.
async fn read_until(ws: &mut Ws, done: impl Fn(&Value) -> bool) -> Vec<Value> {
    let mut frames = Vec::new();
    loop {
        let frame = next_json(ws).await;
        let finished = done(&frame);
        frames.push(frame);
        if finished {
            return frames;
        }
    }
}

fn names(frames: &[Value]) -> Vec<&str> {
    frames
        .iter()
        .map(|f| f["type"].as_str().unwrap_or("?"))
        .collect()
}

fn reply_stored(frame: &Value) -> bool {
    frame["type"] == "TurnState" && frame["tutor_turn_seq"].is_number()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_text_chat_over_http_sends_its_events_in_order_to_the_websocket() {
    let running = start(vec![
        reply("Hello! What is your name?"),
        reply("Nice to meet you, Dewi."),
    ])
    .await;
    let mut ws = running.connect().await;
    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "Snapshot");
    assert!(first["state"]["active_session"].is_null());
    assert_eq!(
        first["state"]["unavailable"],
        json!(["activities", "speech", "models"])
    );
    let mut last_seq = first["seq"].as_u64().unwrap();

    // Start. The first event is the session becoming active, then the opening turn.
    let (status, view) = running
        .post(
            "/api/sessions",
            json!({"kind": "text_chat", "topic": {"kind": "typed", "text": "food"}}),
        )
        .await;
    assert_eq!(status, 200, "{view}");
    assert_eq!(view["kind"], "text_chat");
    assert_eq!(view["life"], "active");
    let id = view["id"].as_i64().unwrap();
    let opening = read_until(&mut ws, reply_stored).await;
    assert_eq!(
        names(&opening),
        [
            "SessionState",
            "TurnState",
            "TurnState",
            "TutorTextDelta",
            "TurnState"
        ]
    );

    // A second session is refused while one runs.
    let (status, body) = running
        .post(
            "/api/sessions",
            json!({"kind": "text_chat", "topic": {"kind": "typed", "text": "music"}}),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "conflict");

    // A turn.
    let (status, accepted) = running
        .post(
            &format!("/api/sessions/{id}/text"),
            json!({"text": "My name is Dewi."}),
        )
        .await;
    assert_eq!(status, 200, "{accepted}");
    assert_eq!(accepted["session_id"], id);
    assert_eq!(accepted["turn"], 2);
    let turn = read_until(&mut ws, |f| f["type"] == "AnalysisReady").await;
    assert_eq!(
        names(&turn),
        [
            "TranscriptFinal",
            "TurnState",
            "TurnState",
            "TutorTextDelta",
            "TurnState",
            "AnalysisReady"
        ]
    );
    assert_eq!(turn[0]["text"], "My name is Dewi.");
    assert_eq!(turn[0]["source"], "text");
    assert_eq!(turn[1]["state"], "thinking");
    assert_eq!(turn[2]["state"], "replying");
    assert_eq!(turn[3]["delta"], "Nice to meet you, Dewi.");
    assert_eq!(turn[4]["state"], "waiting");
    assert!(turn[4]["tutor_turn_seq"].is_number());
    assert_eq!(turn[5]["analysis"]["turn_seq"], turn[0]["turn_seq"]);

    // Every event has the next sequence number.
    for frame in opening.iter().chain(&turn) {
        let seq = frame["seq"].as_u64().unwrap();
        assert_eq!(seq, last_seq + 1, "{frame}");
        last_seq = seq;
    }

    // Pause and resume.
    let (status, paused) = running
        .post(&format!("/api/sessions/{id}/pause"), json!({}))
        .await;
    assert_eq!(status, 200, "{paused}");
    assert_eq!(paused["life"], "paused");
    let (status, body) = running
        .post(&format!("/api/sessions/{id}/text"), json!({"text": "Hi"}))
        .await;
    assert_eq!(status, 409, "a paused chat takes no message: {body}");
    let (status, resumed) = running
        .post(&format!("/api/sessions/{id}/resume"), json!({}))
        .await;
    assert_eq!(status, 200, "{resumed}");
    assert_eq!(resumed["life"], "active");

    // Stop with an empty body: the summary comes before the end.
    let mut request = common::Req::json(Method::POST, &format!("/api/sessions/{id}/stop"), None)
        .authed(&running.cookie);
    request.host = Some(Box::leak(
        format!("127.0.0.1:{}", running.port).into_boxed_str(),
    ));
    request.origin = Some(Box::leak(
        format!("http://127.0.0.1:{}", running.port).into_boxed_str(),
    ));
    let response = common::send(&running.router, request).await;
    assert_eq!(response.status().as_u16(), 200);
    let ended = common::body_json(response).await;
    assert_eq!(ended["status"], "completed");
    assert_eq!(ended["feedback"]["kind"], "conversation");
    let end = read_until(&mut ws, |f| {
        f["type"] == "SessionState" && f["life"] == "ended"
    })
    .await;
    let tail = names(&end);
    assert!(
        tail.ends_with(&["FeedbackReady", "SessionState"]),
        "the summary, then the end: {tail:?}"
    );
    let (_, state) = running.http(Method::GET, "/api/state", None).await;
    assert!(state["active_session"].is_null());

    // The slot is free again.
    let (status, _) = running.post("/api/sessions/9999/stop", json!({})).await;
    assert_eq!(status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_reconnects_mid_session_starts_from_a_snapshot_with_the_session() {
    let running = start(vec![reply("Hello there. How are you?")]).await;
    let mut first = running.connect().await;
    assert_eq!(next_json(&mut first).await["type"], "Snapshot");
    let (status, view) = running
        .post(
            "/api/sessions",
            json!({"kind": "text_chat", "topic": {"kind": "typed", "text": "food"}}),
        )
        .await;
    assert_eq!(status, 200, "{view}");
    let id = view["id"].as_i64().unwrap();
    // The page is closed once the opening reply is there.
    read_until(&mut first, reply_stored).await;
    drop(first);

    let mut second = running.connect().await;
    let again = next_json(&mut second).await;
    assert_eq!(again["type"], "Snapshot", "the first message is a snapshot");
    let active = &again["state"]["active_session"];
    assert_eq!(active["id"], id);
    assert_eq!(active["kind"], "text_chat");
    assert_eq!(active["life"], "active");
    assert_eq!(
        active["recent"][0]["text"], "Hello there. How are you?",
        "the page can rebuild the conversation: {active}"
    );
    assert_eq!(active["recent"][0]["role"], "tutor");
    assert_eq!(
        again["seq"].as_u64().unwrap(),
        running.state.core.events().current_seq(),
        "the snapshot carries the newest sequence number"
    );
    // The stream goes on from there.
    running.state.core.publish_heartbeat();
    let next = next_json(&mut second).await;
    assert_eq!(next["type"], "Heartbeat");
    assert_eq!(
        next["seq"].as_u64().unwrap(),
        again["seq"].as_u64().unwrap() + 1
    );
}
