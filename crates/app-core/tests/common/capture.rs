#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

//! A local server that records every request it receives, for the allowlist
//! tests. It speaks just enough of an OpenAI-compatible chat endpoint for one
//! tutor turn, and it can redirect every request somewhere else.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// What the server does with a request.
#[derive(Clone)]
pub enum Behaviour {
    /// A streamed reply with this text.
    Reply(&'static str),
    /// HTTP 307 to this location.
    Redirect(String),
    /// HTTP 503.
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub host: String,
}

struct Inner {
    behaviour: Behaviour,
    seen: Mutex<Vec<Seen>>,
}

pub struct CaptureServer {
    addr: SocketAddr,
    inner: Arc<Inner>,
    task: JoinHandle<()>,
}

impl Drop for CaptureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn chunk(delta: &str, finish: Option<&str>) -> String {
    let finish = finish.map_or("null".to_owned(), |f| format!("\"{f}\""));
    format!(
        "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{{delta}}},\"finish_reason\":{finish}}}]}}\n\n"
    )
}

async fn handle(State(inner): State<Arc<Inner>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let _ = axum::body::to_bytes(body, 1 << 20)
        .await
        .unwrap_or_default();
    inner
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Seen {
            method: parts.method.to_string(),
            path: parts.uri.path().to_owned(),
            host: parts
                .headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
        });
    match &inner.behaviour {
        Behaviour::Reply(text) => {
            let body = format!(
                "{}{}{}data: [DONE]\n\n",
                chunk("\"role\":\"assistant\",\"content\":\"\"", None),
                chunk(&format!("\"content\":{}", serde_json::json!(text)), None),
                chunk("", Some("stop")),
            );
            Response::builder()
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from(Bytes::from(body)))
                .expect("response")
        }
        Behaviour::Redirect(location) => Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header(header::LOCATION, location)
            .body(Body::empty())
            .expect("response"),
        Behaviour::Unavailable => Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Body::empty())
            .expect("response"),
    }
}

impl CaptureServer {
    pub async fn start(behaviour: Behaviour) -> Self {
        let inner = Arc::new(Inner {
            behaviour,
            seen: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&inner));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { addr, inner, task }
    }

    /// `host:port`, as a `Host` header carries it.
    pub fn authority(&self) -> String {
        self.addr.to_string()
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.inner
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
