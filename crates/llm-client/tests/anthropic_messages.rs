#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! Contract tests for the `anthropic_messages` adapter against hand-written
//! fixtures that follow the documented event and error formats.

mod common;

use std::time::Duration;

use common::{Chunk, Reply, TEST_KEY, anthropic_rig, collect, complete, test_schema, text_request};
use futures_util::StreamExt;
use llm_client::{
    FinishReason, Format, LlmError, ProtocolAdapter, SchemaRef, StreamEvent, TimeoutKind,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn streams_text_by_event_type_and_sends_the_documented_request() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_basic.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.text, "Hello, nice to meet you.");
    assert_eq!(collected.summary.finish, FinishReason::Stop);
    let usage = collected.summary.usage.expect("usage");
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(25), Some(12))
    );
    assert!(collected.summary.time_to_first_token.is_some());

    let requests = rig.server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.headers["x-api-key"], TEST_KEY);
    assert_eq!(request.headers["anthropic-version"], "2023-06-01");
    assert_eq!(request.headers["content-type"], "application/json");
    assert!(!request.headers.contains_key("authorization"));
    let body = request.json();
    assert_eq!(body["model"], "test-model");
    assert_eq!(
        body["max_tokens"], 80,
        "max_tokens is required by the protocol"
    );
    assert_eq!(body["stream"], true);
    assert_eq!(body["system"], "You are a tutor.");
    assert_eq!(body["temperature"], 0.7);
    assert_eq!(
        body["messages"],
        json!([
            { "role": "user", "content": "Hi" },
            { "role": "assistant", "content": "Hello" },
            { "role": "user", "content": "How are you?" }
        ])
    );
}

#[tokio::test]
async fn thinking_blocks_are_not_speech() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_thinking.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.text, "Good morning!");
    assert_eq!(
        collected.summary.usage.expect("usage").output_tokens,
        Some(30)
    );
}

#[tokio::test]
async fn max_tokens_stop_reason_keeps_the_text_and_says_length() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_max_tokens.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.summary.finish, FinishReason::Length);
    assert_eq!(
        collected.text,
        "First sentence. Second one is cut in the mid"
    );
}

#[tokio::test]
async fn refusal_stop_reason_is_reported_as_refusal() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_refusal.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.summary.finish, FinishReason::Refusal);
}

#[tokio::test]
async fn an_error_event_is_a_typed_error_after_the_text_already_received() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_error.sse"));

    let mut stream = adapter
        .stream_text(&text_request(), &CancellationToken::new())
        .await
        .expect("stream");
    assert_eq!(
        stream.next().await.expect("first").expect("delta"),
        StreamEvent::Delta("Partial".to_owned())
    );
    let error = stream
        .next()
        .await
        .expect("second")
        .expect_err("overloaded");
    match error {
        LlmError::Stream { message } => assert!(message.contains("overloaded_error"), "{message}"),
        other => panic!("unexpected {other:?}"),
    }
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_stream_that_ends_before_the_message_is_complete_is_an_error() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_truncated.sse"));

    let mut stream = adapter
        .stream_text(&text_request(), &CancellationToken::new())
        .await
        .expect("stream");
    assert!(matches!(
        stream.next().await,
        Some(Ok(StreamEvent::Delta(_)))
    ));
    let error = stream.next().await.expect("item").expect_err("truncated");

    assert!(matches!(error, LlmError::Transport(_)), "{error:?}");
}

#[tokio::test]
async fn a_stop_reason_without_message_stop_is_a_complete_reply() {
    let (rig, adapter) = anthropic_rig().await;
    let body = concat!(
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
    );
    rig.server
        .enqueue(Reply::sse_bytes(body.as_bytes().to_vec()));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.text, "Hi");
    assert_eq!(collected.summary.finish, FinishReason::Stop);
}

#[tokio::test]
async fn events_the_client_does_not_know_are_ignored() {
    let (rig, adapter) = anthropic_rig().await;
    let body = concat!(
        "event: future_event\ndata: {\"type\":\"future_event\",\"x\":1}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"citations_delta\",\"citation\":{}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    rig.server
        .enqueue(Reply::sse_bytes(body.as_bytes().to_vec()));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.text, "ok");
}

#[tokio::test]
async fn omits_a_rejected_temperature_and_remembers() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(400, "anthropic/error_temperature.json"));
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_basic.sse"));
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_basic.sse"));

    collect(&adapter, &text_request()).await.expect("first");
    collect(&adapter, &text_request()).await.expect("second");

    let requests = rig.server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].json().get("temperature").is_some());
    assert!(requests[1].json().get("temperature").is_none());
    assert!(requests[2].json().get("temperature").is_none());
    assert!(!rig.caps.snapshot().supports_temperature);
}

#[tokio::test]
async fn bad_credentials_are_a_typed_error() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        401,
        "anthropic/error_authentication.json",
    ));

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("rejected");

    assert!(matches!(error, LlmError::Auth { status: 401 }));
    assert!(!format!("{error} {error:?}").contains(TEST_KEY));
}

#[tokio::test]
async fn native_structured_output_goes_through_output_config_format() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "anthropic/message_json.json"));

    let completion = complete(
        &adapter,
        |schema| {
            Format::NativeSchema(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        None,
    )
    .await
    .expect("completion");

    // The JSON is split over two text blocks and a thinking block precedes them.
    let value: Value = serde_json::from_str(&completion.text).expect("json");
    assert_eq!(value["title"], "A short test");
    assert_eq!(completion.finish, FinishReason::Stop);
    assert_eq!(completion.usage.expect("usage").output_tokens, Some(20));
    let body = rig.server.requests()[0].json();
    assert_eq!(body["stream"], false);
    assert_eq!(
        body["output_config"],
        json!({ "format": { "type": "json_schema", "schema": test_schema() } })
    );
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

#[tokio::test]
async fn an_anthropic_gateway_that_answers_in_the_openai_shape_is_read_too() {
    // Seen live on 2026-10-07 from an Anthropic-compatible gateway: the request
    // used `anthropic_messages`, the body came back OpenAI-shaped.
    let (rig, adapter) = anthropic_rig().await;
    rig.server.enqueue(Reply::json_text(
        200,
        &json!({
            "id": "cmb-1",
            "object": "chat.completion",
            "model": "some-model",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "{\"word\":\"tea\"}" }
            }]
        })
        .to_string(),
    ));

    let completion = complete(
        &adapter,
        |schema| {
            Format::NativeSchema(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        None,
    )
    .await
    .expect("completion");

    assert_eq!(completion.text, "{\"word\":\"tea\"}");

    // The documented Anthropic shape still wins when both are present.
    let both = json!({
        "content": [{ "type": "text", "text": "{\"from\":\"anthropic\"}" }],
        "choices": [{ "index": 0, "message": { "content": "{\"from\":\"openai\"}" } }]
    });
    rig.server.enqueue(Reply::json_text(200, &both.to_string()));
    let completion = complete(
        &adapter,
        |schema| {
            Format::NativeSchema(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        None,
    )
    .await
    .expect("completion");

    assert_eq!(completion.text, "{\"from\":\"anthropic\"}");
}

#[tokio::test]
async fn forced_tool_call_is_the_fallback_and_its_input_is_the_json() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "anthropic/message_tool_use.json"));

    let completion = complete(
        &adapter,
        |schema| {
            Format::ForcedTool(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        None,
    )
    .await
    .expect("completion");

    let value: Value = serde_json::from_str(&completion.text).expect("json");
    assert_eq!(
        value,
        json!({ "title": "From a tool", "word_count": 3, "is_ok": true })
    );
    assert_eq!(completion.finish, FinishReason::ToolCalls);
    let body = rig.server.requests()[0].json();
    assert_eq!(body["tools"][0]["name"], "probe_test");
    assert_eq!(body["tools"][0]["input_schema"], test_schema());
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "tool", "name": "probe_test" })
    );
    assert!(body.get("output_config").is_none());
}

#[tokio::test]
async fn a_server_without_output_config_gives_a_rejection_the_ladder_can_act_on() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        400,
        "anthropic/error_output_config_unsupported.json",
    ));

    let error = complete(
        &adapter,
        |schema| {
            Format::NativeSchema(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        None,
    )
    .await
    .expect_err("rejected");

    match error {
        LlmError::Rejected {
            status, message, ..
        } => {
            assert_eq!(status, 400);
            assert!(message.contains("output_config"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn refusal_in_a_non_streaming_reply_is_an_error_not_output() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "anthropic/message_refusal.json"));

    let error = complete(
        &adapter,
        |schema| {
            Format::NativeSchema(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        None,
    )
    .await
    .expect_err("refusal");

    assert!(matches!(error, LlmError::Refusal));
}

#[tokio::test]
async fn max_tokens_in_a_non_streaming_reply_is_reported_with_the_partial_text() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        200,
        "anthropic/message_max_tokens.json",
    ));

    let completion = complete(
        &adapter,
        |schema| {
            Format::NativeSchema(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        None,
    )
    .await
    .expect("completion");

    assert_eq!(completion.finish, FinishReason::Length);
    assert_eq!(completion.text, "{\"title\":\"A short te");
}

#[tokio::test]
async fn json_mode_does_not_exist_in_this_protocol() {
    let (rig, adapter) = anthropic_rig().await;

    let error = complete(&adapter, |_| Format::JsonMode, None)
        .await
        .expect_err("invalid");

    assert!(matches!(error, LlmError::InvalidRequest(_)));
    assert_eq!(
        rig.server.hits(),
        0,
        "nothing is sent for a request that cannot be valid"
    );
}

#[tokio::test]
async fn an_error_object_in_a_200_reply_is_a_protocol_error() {
    let (rig, adapter) = anthropic_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "anthropic/error_overloaded.json"));

    let error = complete(&adapter, |_| Format::Plain, None)
        .await
        .expect_err("error");

    assert!(matches!(error, LlmError::Protocol(_)), "{error:?}");
}

#[tokio::test]
async fn a_cancelled_stream_ends_with_cancelled() {
    let (rig, adapter) = anthropic_rig().await;
    let first = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n";
    rig.server.enqueue(Reply::sse_chunks(vec![
        Chunk {
            delay: Duration::ZERO,
            bytes: first.as_bytes().to_vec(),
        },
        Chunk {
            delay: Duration::from_secs(30),
            bytes: b"data: {\"type\":\"message_stop\"}\n\n".to_vec(),
        },
    ]));
    let cancel = CancellationToken::new();

    let mut stream = adapter
        .stream_text(&text_request(), &cancel)
        .await
        .expect("stream");
    assert_eq!(
        stream.next().await.expect("item").expect("delta"),
        StreamEvent::Delta("Hel".to_owned())
    );
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let started = std::time::Instant::now();
    let error = stream.next().await.expect("item").expect_err("cancelled");

    assert!(matches!(error, LlmError::Cancelled), "{error:?}");
    assert!(started.elapsed() < Duration::from_millis(400));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_stalled_stream_hits_the_total_limit_after_the_first_token() {
    let (rig, _) = anthropic_rig().await;
    let mut options = common::quick_options();
    options.tutor.first_token = Some(Duration::from_millis(400));
    options.tutor.total = Duration::from_millis(900);
    let config = llm_client::AdapterConfig {
        options,
        ..rig.config.clone()
    };
    let adapter = llm_client::AnthropicMessages::new(config).expect("adapter");
    let first = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n";
    rig.server.enqueue(Reply::sse_chunks(vec![
        Chunk {
            delay: Duration::ZERO,
            bytes: first.as_bytes().to_vec(),
        },
        Chunk {
            delay: Duration::from_secs(30),
            bytes: b"data: {\"type\":\"message_stop\"}\n\n".to_vec(),
        },
    ]));

    let mut stream = adapter
        .stream_text(&text_request(), &CancellationToken::new())
        .await
        .expect("stream");
    assert!(matches!(
        stream.next().await,
        Some(Ok(StreamEvent::Delta(_)))
    ));
    let error = stream.next().await.expect("item").expect_err("timeout");

    // The first-token limit no longer applies once a token arrived; the total limit does.
    assert!(
        matches!(error, LlmError::Timeout(TimeoutKind::Total)),
        "{error:?}"
    );
}
