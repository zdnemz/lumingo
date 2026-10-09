#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

//! A minimal OpenAI-compatible server on 127.0.0.1 for the provider tests.
//! The real HTTP client code runs against it; only the provider is replaced.
//! It answers what the capability probe asks and nothing else:
//!
//! - a request with the wrong key gets HTTP 401;
//! - a streaming request gets a short event stream;
//! - the probe's schema gets a matching JSON object, including the `check`
//!   canary field that only a provider honouring the schema can produce;
//! - every other request (the contract steps) gets plain text, which the
//!   ladder rejects as invalid output, so those contracts are reported as not
//!   working.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const STREAM: &str = concat!(
    "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello there.\"},\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-test\",\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":3,\"total_tokens\":7}}\n\n",
    "data: [DONE]\n\n",
);

#[derive(Clone)]
struct Shared {
    key: &'static str,
    requests: Arc<AtomicUsize>,
}

pub struct FakeProvider {
    /// `http://127.0.0.1:<port>`
    pub base_url: String,
    requests: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeProvider {
    /// Starts the server. Requests must carry `Authorization: Bearer <key>`.
    pub async fn start(key: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let shared = Shared {
            key,
            requests: Arc::clone(&requests),
        };
        let app = Router::new()
            .route("/chat/completions", post(handle))
            .with_state(shared);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            requests,
            task,
        }
    }

    /// How many requests reached the server.
    pub fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

fn completion(content: &str) -> Response {
    let body = json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "model": "gpt-test",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content, "refusal": null },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13 }
    });
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

async fn handle(State(shared): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    shared.requests.fetch_add(1, Ordering::SeqCst);
    let authorised = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {}", shared.key));
    if !authorised {
        let error = json!({"error": {
            "message": "Incorrect API key provided.",
            "type": "invalid_request_error",
            "param": null,
            "code": "invalid_api_key"
        }});
        return (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "application/json")],
            error.to_string(),
        )
            .into_response();
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if request["stream"] == true {
        return (
            [(header::CONTENT_TYPE, "text/event-stream")],
            STREAM.to_owned(),
        )
            .into_response();
    }
    if request["response_format"]["json_schema"]["name"] == "probe_test" {
        return completion(
            "{\"title\":\"A short test\",\"word_count\":3,\"is_ok\":true,\"check\":\"schema_received\"}",
        );
    }
    completion("ok")
}
