#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The event stream against a real listener on the loopback address.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tokio_util::sync::CancellationToken;
use tutor_server::events::EventHub;
use tutor_server::security::Guard;
use tutor_server::{AppState, build_router};

/// Next text frame, or panics after two seconds.
async fn next_text(ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> String {
    let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timed out waiting for a frame")
        .expect("stream ended")
        .expect("frame");
    match frame {
        Message::Text(text) => text.to_string(),
        other => panic!("unexpected frame {other:?}"),
    }
}

struct Running {
    port: u16,
    state: Arc<AppState>,
}

async fn start() -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let guard = Arc::new(Guard::new(port, None).expect("guard"));
    let state = Arc::new(AppState {
        hub: EventHub::new(false),
        guard,
        shutdown: CancellationToken::new(),
        dev_mode: false,
    });
    let app = build_router(Arc::clone(&state));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Running { port, state }
}

/// Fetches the UI page the way a browser does and returns the `name=value` cookie pair.
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

fn ws_request(
    port: u16,
    origin: &str,
    cookie: Option<&str>,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .expect("request");
    request
        .headers_mut()
        .insert("Origin", origin.parse().expect("origin"));
    if let Some(cookie) = cookie {
        request
            .headers_mut()
            .insert("Cookie", cookie.parse().expect("cookie"));
    }
    request
}

#[tokio::test]
async fn first_message_is_a_snapshot_then_events_follow_in_order() {
    let running = start().await;
    let cookie = session_cookie(running.port).await;
    let origin = format!("http://127.0.0.1:{}", running.port);
    let (mut ws, _) = connect_async(ws_request(running.port, &origin, Some(&cookie)))
        .await
        .expect("handshake");

    let first: serde_json::Value = serde_json::from_str(&next_text(&mut ws).await).expect("json");
    assert_eq!(first["type"], "Snapshot");
    assert_eq!(first["seq"], 0);
    assert_eq!(first["state"]["dev_mode"], false);

    running.state.hub.publish_heartbeat();
    running.state.hub.publish_heartbeat();
    let second: serde_json::Value = serde_json::from_str(&next_text(&mut ws).await).expect("json");
    let third: serde_json::Value = serde_json::from_str(&next_text(&mut ws).await).expect("json");
    assert_eq!(second["type"], "Heartbeat");
    assert_eq!(second["seq"], 1);
    assert_eq!(third["seq"], 2);
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test]
async fn a_late_connection_starts_from_the_current_sequence() {
    let running = start().await;
    for _ in 0..5 {
        running.state.hub.publish_heartbeat();
    }
    let cookie = session_cookie(running.port).await;
    let origin = format!("http://127.0.0.1:{}", running.port);
    let (mut ws, _) = connect_async(ws_request(running.port, &origin, Some(&cookie)))
        .await
        .expect("handshake");
    let first: serde_json::Value = serde_json::from_str(&next_text(&mut ws).await).expect("json");
    assert_eq!(first["type"], "Snapshot");
    assert_eq!(first["seq"], 5);
}

#[tokio::test]
async fn handshake_is_refused_for_foreign_origin_and_missing_cookie() {
    let running = start().await;
    let cookie = session_cookie(running.port).await;
    let own = format!("http://127.0.0.1:{}", running.port);

    for (origin, cookie, expected) in [
        ("https://evil.example", Some(cookie.as_str()), 403_u16),
        (own.as_str(), None, 401),
    ] {
        let result = connect_async(ws_request(running.port, origin, cookie)).await;
        match result {
            Err(WsError::Http(response)) => assert_eq!(response.status().as_u16(), expected),
            other => panic!("expected an HTTP refusal, got {other:?}"),
        }
    }
}
