//! A scripted HTTP server on 127.0.0.1 for the contract tests. The real HTTP
//! client code path runs against it; only the provider is replaced.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use futures_util::stream;
use llm_client::{
    AdapterConfig, AnthropicMessages, ApiKey, Capabilities, CapsHandle, ChatMessage, ClientOptions,
    Completion, CompletionRequest, Format, HttpClientFactory, Limits, LlmError, OpenAiChat,
    ProtocolAdapter, ProviderClient, TextRequest,
};
use reqwest::Url;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const TEST_KEY: &str = "sk-test-0123456789abcdefghij-KEYMATERIAL";

/// Reads a hand-written fixture from `tests/fixtures`.
pub fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

pub fn fixture_text(name: &str) -> String {
    String::from_utf8(fixture(name)).expect("fixture is UTF-8")
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub delay: Duration,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
enum ReplyBody {
    Bytes(Vec<u8>),
    Chunks(Vec<Chunk>),
}

#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: ReplyBody,
}

impl Reply {
    pub fn json_fixture(status: u16, name: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: ReplyBody::Bytes(fixture(name)),
        }
    }

    pub fn json_text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: ReplyBody::Bytes(body.as_bytes().to_vec()),
        }
    }

    /// A 200 event stream sent as one write.
    pub fn sse_fixture(name: &str) -> Self {
        Self::sse_bytes(fixture(name))
    }

    pub fn sse_bytes(bytes: Vec<u8>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: ReplyBody::Bytes(bytes),
        }
    }

    /// A 200 event stream sent in pieces with a pause before each piece.
    pub fn sse_chunks(chunks: Vec<Chunk>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: ReplyBody::Chunks(chunks),
        }
    }

    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: ReplyBody::Bytes(Vec::new()),
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("the request body is JSON")
    }
}

#[derive(Default)]
struct Inner {
    replies: VecDeque<Reply>,
    requests: Vec<Recorded>,
}

pub struct TestServer {
    addr: SocketAddr,
    inner: Arc<Mutex<Inner>>,
    task: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(State(inner): State<Arc<Mutex<Inner>>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 16 * 1024 * 1024)
        .await
        .unwrap_or_default();
    let headers = parts
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                v.to_str().unwrap_or("<binary>").to_owned(),
            )
        })
        .collect();
    let reply = {
        let mut inner = inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.requests.push(Recorded {
            method: parts.method.to_string(),
            path: parts.uri.path().to_owned(),
            headers,
            body: bytes.to_vec(),
        });
        inner.replies.pop_front()
    };
    let Some(reply) = reply else {
        return Response::builder()
            .status(599)
            .body(Body::from("no scripted reply"))
            .expect("response");
    };
    let mut builder = Response::builder().status(reply.status);
    for (name, value) in &reply.headers {
        builder = builder.header(name, value);
    }
    let body = match reply.body {
        ReplyBody::Bytes(bytes) => Body::from(bytes),
        ReplyBody::Chunks(chunks) => Body::from_stream(stream::unfold(
            chunks.into_iter(),
            |mut chunks| async move {
                let chunk = chunks.next()?;
                tokio::time::sleep(chunk.delay).await;
                Some((Ok::<_, std::convert::Infallible>(chunk.bytes), chunks))
            },
        )),
    };
    builder.body(body).expect("response")
}

impl TestServer {
    pub async fn start() -> Self {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let app = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&inner));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { addr, inner, task }
    }

    pub fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{}", self.addr, path)).expect("url")
    }

    pub fn enqueue(&self, reply: Reply) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replies
            .push_back(reply);
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }

    pub fn hits(&self) -> usize {
        self.requests().len()
    }
}

/// Fast limits so timeout tests do not take seconds.
pub fn quick_options() -> ClientOptions {
    ClientOptions {
        tutor: Limits {
            connect: Duration::from_millis(500),
            first_token: Some(Duration::from_secs(2)),
            total: Duration::from_secs(5),
        },
        background: Limits {
            connect: Duration::from_millis(500),
            first_token: None,
            total: Duration::from_secs(5),
        },
    }
}

pub struct Rig {
    pub server: TestServer,
    pub caps: CapsHandle,
    pub config: AdapterConfig,
}

/// Builds an adapter config pointing at a fresh test server. `base_path` is
/// appended to the server address, for example `/v1`.
pub async fn rig(base_path: &str, key: Option<&str>, options: ClientOptions) -> Rig {
    let server = TestServer::start().await;
    let base_url = server.url(base_path);
    let factory = HttpClientFactory::new(&base_url, options.tutor.connect).expect("factory");
    let caps = CapsHandle::new(Capabilities::default());
    let config = AdapterConfig {
        base_url,
        model: "test-model".to_owned(),
        key: key.map(|k| ApiKey::new(k).expect("test key")),
        http: factory.client(),
        caps: caps.clone(),
        options,
    };
    Rig {
        server,
        caps,
        config,
    }
}

pub async fn openai_rig() -> (Rig, OpenAiChat) {
    let rig = rig("/v1", Some(TEST_KEY), quick_options()).await;
    let adapter = OpenAiChat::new(rig.config.clone()).expect("adapter");
    (rig, adapter)
}

pub fn text_request() -> TextRequest {
    TextRequest::new(
        "You are a tutor.",
        vec![
            ChatMessage::user("Hi"),
            ChatMessage::assistant("Hello"),
            ChatMessage::user("How are you?"),
        ],
        80,
    )
    .with_temperature(0.7)
}

pub async fn collect(
    adapter: &impl ProtocolAdapter,
    request: &TextRequest,
) -> Result<llm_client::CollectedText, LlmError> {
    adapter
        .stream_text(request, &CancellationToken::new())
        .await?
        .collect_text()
        .await
}

pub fn test_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "word_count", "is_ok"],
        "properties": {
            "title": { "type": "string" },
            "word_count": { "type": "integer" },
            "is_ok": { "type": "boolean" }
        }
    })
}

pub async fn complete(
    adapter: &impl ProtocolAdapter,
    format: impl FnOnce(&Value) -> Format<'_>,
    temperature: Option<f32>,
) -> Result<Completion, LlmError> {
    let schema = test_schema();
    let messages = [ChatMessage::user("Make an example.")];
    let request = CompletionRequest {
        system: "Return JSON.",
        messages: &messages,
        max_tokens: 200,
        temperature,
        format: format(&schema),
    };
    // `schema` must outlive the call: the format borrows it.
    adapter.complete(&request, &CancellationToken::new()).await
}

pub async fn anthropic_rig() -> (Rig, AnthropicMessages) {
    let rig = rig("", Some(TEST_KEY), quick_options()).await;
    let adapter = AnthropicMessages::new(rig.config.clone()).expect("adapter");
    (rig, adapter)
}

/// A valid instance of one contract, written by hand in `tests/fixtures/contract_samples`.
pub fn sample(name: &str) -> Value {
    serde_json::from_slice(&fixture(&format!("contract_samples/{name}.json")))
        .expect("sample is JSON")
}

pub const CONTRACT_NAMES: [&str; 4] = [
    "turn_analysis",
    "rubric_score",
    "practice_items",
    "reading_passage",
];

/// An `openai_chat` non-streaming reply whose message content is `content`.
pub fn openai_reply(content: &str) -> Reply {
    Reply::json_text(
        200,
        &json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content, "refusal": null },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        })
        .to_string(),
    )
}

/// An `anthropic_messages` reply with one text block.
pub fn anthropic_text_reply(content: &str) -> Reply {
    Reply::json_text(
        200,
        &json!({
            "id": "msg_test", "type": "message", "role": "assistant", "model": "claude-test",
            "content": [{ "type": "text", "text": content }],
            "stop_reason": "end_turn", "stop_sequence": null,
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        })
        .to_string(),
    )
}

/// An `anthropic_messages` reply that answers a forced tool call.
pub fn anthropic_tool_reply(name: &str, input: &Value) -> Reply {
    Reply::json_text(
        200,
        &json!({
            "id": "msg_test", "type": "message", "role": "assistant", "model": "claude-test",
            "content": [{ "type": "tool_use", "id": "toolu_test", "name": name, "input": input }],
            "stop_reason": "tool_use", "stop_sequence": null,
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        })
        .to_string(),
    )
}

pub fn client_for(adapter: impl ProtocolAdapter + 'static, caps: &CapsHandle) -> ProviderClient {
    ProviderClient::from_adapter(Arc::new(adapter), caps.clone())
}
