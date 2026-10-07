//! Contract tests against a fake provider on loopback. No live calls.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

use llm_client::{
    ApiKey, LlmClient, LlmError, Message, Protocol, ProviderConfig, Role, StreamEvent, TextRequest,
    Timeouts,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

const KEY: &str = "test-key-1234567890";

struct Reply {
    bytes: String,
    /// Keep the connection open after writing, to simulate a stalled provider.
    stall: bool,
}

fn http(status: &str, headers: &str, body: &str) -> Reply {
    let bytes = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    Reply {
        bytes,
        stall: false,
    }
}

fn sse(body: &str) -> Reply {
    http("200 OK", "Content-Type: text/event-stream\r\n", body)
}

fn status(code: &str, body: &str) -> Reply {
    http(code, "", body)
}

/// Serves one canned reply per connection, in order, and records each request.
async fn serve(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    tokio::spawn(async move {
        for reply in replies {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head_end, len) = loop {
                let n = sock.read(&mut chunk).await.unwrap_or(0);
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_lowercase();
                if let Some(i) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0usize);
                    break (i + 4, len);
                }
                if n == 0 {
                    return;
                }
            };
            while buf.len() < head_end + len {
                let n = sock.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            record
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf).into_owned());
            let _ = sock.write_all(reply.bytes.as_bytes()).await;
            if reply.stall {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            let _ = sock.shutdown().await;
        }
    });
    (format!("http://{addr}"), seen)
}

fn fast() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(2),
        first_token: Duration::from_millis(200),
        total: Duration::from_secs(3),
    }
}

fn client(protocol: Protocol, base: &str) -> LlmClient {
    let cfg = ProviderConfig {
        protocol,
        base_url: base.to_owned(),
        model: "test-model".into(),
        api_key: ApiKey::new(KEY),
    };
    LlmClient::new(cfg, fast()).unwrap()
}

fn request() -> TextRequest {
    TextRequest {
        model: "ignored".into(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: "Hi".into(),
        }],
        max_tokens: 50,
        temperature: Some(0.2),
    }
}

async fn collect(client: &LlmClient) -> Result<Vec<StreamEvent>, LlmError> {
    let mut rx = client
        .stream_text(request(), CancellationToken::new())
        .await?;
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev?);
    }
    Ok(out)
}

const OPENAI_OK: &str =
    "data: {\"choices\":[{\"delta\":{\"content\":\"Hello.\"}}]}\n\ndata: [DONE]\n\n";
const ANTHROPIC_OK: &str = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello.\"}}\n\n";

fn text(s: &str) -> StreamEvent {
    StreamEvent::Text(s.into())
}

#[tokio::test]
async fn openai_streams_text_with_a_bearer_header_on_the_chat_path() {
    let (base, seen) = serve(vec![sse(OPENAI_OK)]).await;
    let events = collect(&client(Protocol::OpenaiChat, &format!("{base}/v1")))
        .await
        .unwrap();
    assert_eq!(events, [text("Hello.")]);
    let req = seen.lock().unwrap()[0].to_lowercase();
    assert!(req.starts_with("post /v1/chat/completions "), "{req}");
    assert!(req.contains(&format!("authorization: bearer {KEY}")));
    assert!(
        req.contains("\"model\":\"test-model\""),
        "the configured model wins"
    );
}

#[tokio::test]
async fn anthropic_streams_text_with_its_own_headers() {
    let (base, seen) = serve(vec![sse(ANTHROPIC_OK)]).await;
    let events = collect(&client(Protocol::AnthropicMessages, &base))
        .await
        .unwrap();
    assert_eq!(events, [text("Hello.")]);
    let req = seen.lock().unwrap()[0].to_lowercase();
    assert!(req.starts_with("post /v1/messages "), "{req}");
    assert!(
        req.contains(&format!("x-api-key: {KEY}")) && req.contains("anthropic-version: 2023-06-01")
    );
}

#[tokio::test]
async fn a_5xx_is_retried_once_then_succeeds() {
    let (base, seen) = serve(vec![
        status("503 Service Unavailable", "busy"),
        sse(OPENAI_OK),
    ])
    .await;
    assert_eq!(
        collect(&client(Protocol::OpenaiChat, &base)).await.unwrap(),
        [text("Hello.")]
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_second_5xx_is_final() {
    let (base, seen) = serve(vec![
        status("500 Internal Server Error", "x"),
        status("500 Internal Server Error", "x"),
    ])
    .await;
    let err = collect(&client(Protocol::OpenaiChat, &base))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Http { status: 500, .. }), "{err}");
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_429_follows_retry_after() {
    let (base, seen) = serve(vec![
        http("429 Too Many Requests", "Retry-After: 0\r\n", "slow down"),
        sse(OPENAI_OK),
    ])
    .await;
    assert_eq!(
        collect(&client(Protocol::OpenaiChat, &base)).await.unwrap(),
        [text("Hello.")]
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_429_with_a_long_retry_after_is_reported_without_waiting() {
    let (base, seen) = serve(vec![http(
        "429 Too Many Requests",
        "Retry-After: 120\r\n",
        "x",
    )])
    .await;
    let err = collect(&client(Protocol::OpenaiChat, &base))
        .await
        .unwrap_err();
    assert!(
        matches!(err, LlmError::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(120))
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn auth_and_other_client_errors_are_not_retried_and_never_carry_the_key() {
    let echo = format!("invalid key {KEY} (also sk-proj-ABCDEFGHIJKL)");
    let (base, seen) = serve(vec![status("401 Unauthorized", &echo)]).await;
    let err = collect(&client(Protocol::OpenaiChat, &base))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Auth(401)));
    assert_eq!(seen.lock().unwrap().len(), 1);
    let shown = format!("{err} {err:?}");
    assert!(
        !shown.contains(KEY) && !shown.contains("ABCDEFGH"),
        "{shown}"
    );

    let (base, _) = serve(vec![status("422 Unprocessable", &echo)]).await;
    let err = collect(&client(Protocol::OpenaiChat, &base))
        .await
        .unwrap_err();
    let shown = format!("{err} {err:?}");
    assert!(matches!(err, LlmError::Http { status: 422, .. }));
    assert!(
        !shown.contains(KEY) && !shown.contains("ABCDEFGH"),
        "{shown}"
    );
}

#[tokio::test]
async fn a_400_naming_max_tokens_switches_the_parameter_and_is_remembered() {
    let (base, seen) = serve(vec![
        status("400 Bad Request", "{\"error\":{\"message\":\"Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens'.\"}}"),
        sse(OPENAI_OK),
        sse(OPENAI_OK),
    ])
    .await;
    let c = client(Protocol::OpenaiChat, &base);
    collect(&c).await.unwrap();
    collect(&c).await.unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen[0].contains("\"max_tokens\":50"));
    assert!(
        seen[1].contains("\"max_completion_tokens\":50") && !seen[1].contains("\"max_tokens\"")
    );
    assert!(
        seen[2].contains("\"max_completion_tokens\":50"),
        "the switch is remembered"
    );
}

#[tokio::test]
async fn a_400_naming_temperature_drops_it() {
    let (base, seen) = serve(vec![
        status("400 Bad Request", "temperature is not supported"),
        sse(OPENAI_OK),
    ])
    .await;
    collect(&client(Protocol::OpenaiChat, &base)).await.unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen[0].contains("\"temperature\"") && !seen[1].contains("\"temperature\""));
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let (base, seen) = serve(vec![http(
        "302 Found",
        "Location: http://127.0.0.1:1/elsewhere\r\n",
        "",
    )])
    .await;
    let err = collect(&client(Protocol::OpenaiChat, &base))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Http { status: 302, .. }), "{err}");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_stream_that_never_sends_a_token_times_out() {
    let (base, _) = serve(vec![Reply {
        bytes: "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n: keep-alive\n\n".into(),
        stall: true,
    }])
    .await;
    let err = collect(&client(Protocol::OpenaiChat, &base))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Timeout("the first token")), "{err}");
}

#[tokio::test]
async fn cancelling_stops_a_stalled_call() {
    let (base, _) = serve(vec![Reply {
        bytes: String::new(),
        stall: true,
    }])
    .await;
    let c = client(Protocol::OpenaiChat, &base);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        trigger.cancel();
    });
    let started = std::time::Instant::now();
    let err = c
        .stream_text(request(), cancel)
        .await
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(err, LlmError::Cancelled), "{err}");
    assert!(started.elapsed() < Duration::from_millis(300));
}

#[tokio::test]
async fn an_unreachable_host_is_a_transport_error_after_one_retry() {
    let c = client(Protocol::OpenaiChat, "http://127.0.0.1:1");
    let err = collect(&c).await.unwrap_err();
    assert!(matches!(err, LlmError::Transport(_)), "{err}");
}

#[test]
fn plain_http_to_a_remote_host_is_refused_at_construction() {
    let cfg = ProviderConfig {
        protocol: Protocol::OpenaiChat,
        base_url: "http://api.example.com/v1".into(),
        model: "m".into(),
        api_key: ApiKey::new(KEY),
    };
    assert!(matches!(
        LlmClient::new(cfg, Timeouts::default()),
        Err(LlmError::InvalidBaseUrl(_))
    ));
}

// ---- structured output, repair and the probe ----

use llm_client::{Level, StructuredRequest};
use serde_json::json;

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": { "word": { "type": "string" }, "count": { "type": "integer" } },
        "required": ["word", "count"],
        "additionalProperties": false
    })
}

fn structured_request() -> StructuredRequest {
    StructuredRequest {
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: "Go".into(),
        }],
        schema_name: "test_schema".into(),
        schema: schema(),
        max_tokens: 100,
        temperature: None,
    }
}

fn chat_reply(content: &str) -> Reply {
    http(
        "200 OK",
        "Content-Type: application/json\r\n",
        &json!({ "choices": [{ "message": { "content": content } }] }).to_string(),
    )
}

fn tool_reply(arguments: &str) -> Reply {
    let body = json!({ "choices": [{ "message": { "content": null, "tool_calls": [{ "function": { "arguments": arguments } }] } }] });
    http(
        "200 OK",
        "Content-Type: application/json\r\n",
        &body.to_string(),
    )
}

const VALID: &str = "{\"word\":\"tea\",\"count\":3}";

#[tokio::test]
async fn a_valid_native_answer_comes_back_without_repair() {
    let (base, seen) = serve(vec![chat_reply(VALID)]).await;
    let out = client(Protocol::OpenaiChat, &base)
        .structured(structured_request(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        (out.value, out.level, out.repaired),
        (
            json!({ "word": "tea", "count": 3 }),
            Level::NativeSchema,
            false
        )
    );
    let req = seen.lock().unwrap()[0].clone();
    assert!(
        req.contains("\"json_schema\"")
            && req.contains("\"strict\":true")
            && req.contains("\"stream\":false"),
        "{req}"
    );
}

#[tokio::test]
async fn fenced_json_with_prose_is_accepted() {
    let (base, _) = serve(vec![chat_reply(&format!(
        "Here you go:\n```json\n{VALID}\n```"
    ))])
    .await;
    let out = client(Protocol::OpenaiChat, &base)
        .structured(structured_request(), CancellationToken::new())
        .await
        .unwrap();
    assert!(!out.repaired);
}

#[tokio::test]
async fn an_invalid_answer_is_repaired_once_with_the_errors_in_the_prompt() {
    let (base, seen) = serve(vec![chat_reply("{\"word\": 5}"), chat_reply(VALID)]).await;
    let out = client(Protocol::OpenaiChat, &base)
        .structured(structured_request(), CancellationToken::new())
        .await
        .unwrap();
    assert!(out.repaired);
    let second = seen.lock().unwrap()[1].clone();
    assert!(
        second.contains("That JSON was not valid") && second.contains("{\\\"word\\\": 5}"),
        "{second}"
    );
}

#[tokio::test]
async fn a_failed_repair_is_invalid_output_and_makes_no_third_call() {
    let (base, seen) = serve(vec![
        chat_reply("not json"),
        chat_reply("{\"word\": 5}"),
        chat_reply(VALID),
    ])
    .await;
    let err = client(Protocol::OpenaiChat, &base)
        .structured(structured_request(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::InvalidOutput { level: 1 }), "{err}");
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn anthropic_native_structured_output_reads_the_text_block() {
    let body = json!({ "content": [{ "type": "text", "text": VALID }], "stop_reason": "end_turn" })
        .to_string();
    let (base, seen) = serve(vec![http(
        "200 OK",
        "Content-Type: application/json\r\n",
        &body,
    )])
    .await;
    let out = client(Protocol::AnthropicMessages, &base)
        .structured(structured_request(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(out.value["count"], 3);
    assert!(seen.lock().unwrap()[0].contains("\"output_config\""));
}

#[tokio::test]
async fn the_probe_walks_down_the_ladder_and_caches_the_level_that_works() {
    let stream = sse(concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi there.\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":4}}\n\n",
        "data: [DONE]\n\n"
    ));
    let probe_valid = "{\"word\":\"tea\",\"length\":3,\"is_noun\":true}";
    let (base, seen) = serve(vec![
        stream,
        status("400 Bad Request", "response_format is not supported"),
        tool_reply(probe_valid),
        tool_reply(probe_valid),
    ])
    .await;
    let c = client(Protocol::OpenaiChat, &base);
    let caps = c.probe(CancellationToken::new()).await;
    assert!(
        caps.auth_ok && caps.stream_ok && caps.ttft_ms.is_some() && caps.supports_usage_in_stream
    );
    assert_eq!(
        (
            caps.structured_level,
            caps.token_limit_param.as_str(),
            caps.supports_temperature
        ),
        (Some(2), "max_tokens", true)
    );
    assert_eq!(c.structured_level(), Level::ForcedTool);
    // The next structured call starts at the cached level instead of retrying level 1.
    let out = c
        .structured(
            StructuredRequest {
                schema: json!({ "type": "object" }),
                ..structured_request()
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(out.level, Level::ForcedTool);
    assert!(seen.lock().unwrap()[3].contains("\"tool_choice\""));
}

#[tokio::test]
async fn a_probe_with_a_bad_key_reports_auth_failed_and_stops() {
    let (base, seen) = serve(vec![status("401 Unauthorized", "no")]).await;
    let caps = client(Protocol::OpenaiChat, &base)
        .probe(CancellationToken::new())
        .await;
    assert!(!caps.auth_ok && caps.structured_level.is_none());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_probe_where_no_level_works_reports_none() {
    let mut replies = vec![sse(OPENAI_OK)];
    for _ in 0..8 {
        replies.push(chat_reply("sorry, no"));
    }
    let (base, _) = serve(replies).await;
    let caps = client(Protocol::OpenaiChat, &base)
        .probe(CancellationToken::new())
        .await;
    assert!(caps.auth_ok);
    assert_eq!(caps.structured_level, None);
}
