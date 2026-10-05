#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! Contract tests for the `openai_chat` adapter against hand-written fixtures
//! that follow the documented chunk and error formats.

mod common;

use std::time::Duration;

use common::{
    Chunk, Reply, TEST_KEY, collect, complete, fixture_text, openai_rig, test_schema, text_request,
};
use futures_util::StreamExt;
use llm_client::{
    Capabilities, FinishReason, Format, LlmError, ProtocolAdapter, SchemaRef, StreamEvent,
    TokenLimitParam,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn streams_text_and_sends_the_documented_request() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.text, "Hello, nice to meet you.");
    assert_eq!(collected.summary.finish, FinishReason::Stop);
    let usage = collected.summary.usage.expect("usage chunk");
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(42), Some(7))
    );
    assert!(collected.summary.time_to_first_token.is_some());

    let requests = rig.server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(
        request.headers["authorization"],
        format!("Bearer {TEST_KEY}")
    );
    assert_eq!(request.headers["accept"], "text/event-stream");
    let body = request.json();
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 80);
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(body["temperature"], 0.7);
    assert_eq!(body["stream_options"], json!({ "include_usage": true }));
    assert_eq!(
        body["messages"],
        json!([
            { "role": "system", "content": "You are a tutor." },
            { "role": "user", "content": "Hi" },
            { "role": "assistant", "content": "Hello" },
            { "role": "user", "content": "How are you?" }
        ])
    );
}

#[tokio::test]
async fn ignores_keep_alives_null_deltas_reasoning_and_a_missing_done() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_quirks.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    // Only `content` counts: the reasoning deltas and the null content are skipped.
    assert_eq!(collected.text, "Good morning!");
    assert_eq!(collected.summary.finish, FinishReason::Stop);
    assert_eq!(
        collected
            .summary
            .usage
            .expect("usage on the last chunk")
            .output_tokens,
        Some(9)
    );
}

#[tokio::test]
async fn a_stream_that_closes_without_any_finish_marker_ends_cleanly() {
    let (rig, adapter) = openai_rig().await;
    let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Cut\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\" short\"}}]}\n\n";
    rig.server
        .enqueue(Reply::sse_bytes(body.as_bytes().to_vec()));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.text, "Cut short");
    assert_eq!(collected.summary.finish, FinishReason::Unspecified);
}

#[tokio::test]
async fn crlf_line_ends_and_events_split_across_writes_are_handled() {
    let (rig, adapter) = openai_rig().await;
    let wire = fixture_text("openai/stream_basic.sse")
        .replace('\n', "\r\n")
        .into_bytes();
    let chunks = wire
        .chunks(37)
        .map(|piece| Chunk {
            delay: Duration::from_millis(1),
            bytes: piece.to_vec(),
        })
        .collect();
    rig.server.enqueue(Reply::sse_chunks(chunks));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.text, "Hello, nice to meet you.");
}

#[tokio::test]
async fn finish_reason_length_is_reported_with_the_text_received() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_length.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.summary.finish, FinishReason::Length);
    assert_eq!(
        collected.text,
        "First sentence. Second sentence is cut off in the mid"
    );
}

#[tokio::test]
async fn a_refusal_delta_ends_as_refusal() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_refusal.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("stream");

    assert_eq!(collected.summary.finish, FinishReason::Refusal);
    assert_eq!(collected.text, "", "refusal text is not tutor speech");
}

#[tokio::test]
async fn an_error_event_inside_the_stream_is_a_typed_error_without_the_key() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_error.sse"));

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
        .expect_err("the stream failed");
    assert!(matches!(error, LlmError::Stream { .. }), "{error:?}");
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains("ZZZZYYYY"),
        "the key-like text was not removed: {shown}"
    );
    assert!(
        stream.next().await.is_none(),
        "the stream ends after an error"
    );
}

#[tokio::test]
async fn learns_max_completion_tokens_after_one_400_and_remembers_it() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        400,
        "openai/error_max_tokens_unsupported.json",
    ));
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));

    collect(&adapter, &text_request())
        .await
        .expect("first call");
    collect(&adapter, &text_request())
        .await
        .expect("second call");

    let requests = rig.server.requests();
    assert_eq!(
        requests.len(),
        3,
        "one rejected request, one retry, then no more probing"
    );
    assert!(requests[0].json().get("max_tokens").is_some());
    for retry in &requests[1..] {
        let body = retry.json();
        assert_eq!(body["max_completion_tokens"], 80);
        assert!(body.get("max_tokens").is_none());
    }
    assert_eq!(
        rig.caps.snapshot().token_limit_param,
        TokenLimitParam::MaxCompletionTokens
    );
}

#[tokio::test]
async fn learns_max_tokens_when_the_server_rejects_max_completion_tokens() {
    let (rig, adapter) = openai_rig().await;
    rig.caps.replace(Capabilities {
        token_limit_param: TokenLimitParam::MaxCompletionTokens,
        ..Capabilities::default()
    });
    rig.server.enqueue(Reply::json_fixture(
        400,
        "openai/error_max_completion_tokens_unsupported.json",
    ));
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));

    collect(&adapter, &text_request()).await.expect("call");

    assert!(rig.server.requests()[1].json().get("max_tokens").is_some());
    assert_eq!(
        rig.caps.snapshot().token_limit_param,
        TokenLimitParam::MaxTokens
    );
}

#[tokio::test]
async fn omits_a_rejected_temperature_and_remembers() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        400,
        "openai/error_temperature_unsupported.json",
    ));
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));

    collect(&adapter, &text_request())
        .await
        .expect("first call");
    collect(&adapter, &text_request())
        .await
        .expect("second call");

    let requests = rig.server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].json().get("temperature").is_some());
    assert!(requests[1].json().get("temperature").is_none());
    assert!(requests[2].json().get("temperature").is_none());
    assert!(!rig.caps.snapshot().supports_temperature);
}

#[tokio::test]
async fn omits_rejected_stream_options_and_remembers() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        400,
        "openai/error_stream_options_unsupported.json",
    ));
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_quirks.sse"));

    let collected = collect(&adapter, &text_request()).await.expect("call");

    assert_eq!(collected.text, "Good morning!");
    let requests = rig.server.requests();
    assert!(requests[0].json().get("stream_options").is_some());
    assert!(requests[1].json().get("stream_options").is_none());
    assert!(!rig.caps.snapshot().supports_usage_in_stream);
}

#[tokio::test]
async fn a_400_about_something_else_is_returned_and_changes_nothing() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        404,
        "openai/error_model_not_found.json",
    ));

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("rejected");

    match error {
        LlmError::Rejected {
            status, message, ..
        } => {
            assert_eq!(status, 404);
            assert!(message.contains("does not exist"));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(rig.server.hits(), 1);
    assert_eq!(rig.caps.snapshot(), llm_client::Capabilities::default());
}

#[tokio::test]
async fn a_rejection_that_names_max_tokens_for_another_reason_is_not_remembered() {
    let (rig, adapter) = openai_rig().await;
    let too_large = r#"{"error":{"message":"max_tokens is too large: 100000. This model supports at most 4096 completion tokens.","type":"invalid_request_error","param":"max_tokens"}}"#;
    rig.server.enqueue(Reply::json_text(400, too_large));
    rig.server.enqueue(Reply::json_text(400, too_large));

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("rejected");

    assert!(matches!(error, LlmError::Rejected { status: 400, .. }));
    assert_eq!(
        rig.caps.snapshot().token_limit_param,
        TokenLimitParam::MaxTokens,
        "no confirmed success, so nothing is stored"
    );
}

#[tokio::test]
async fn bad_credentials_are_a_typed_error_that_does_not_echo_the_key() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(401, "openai/error_incorrect_key.json"));

    let error = collect(&adapter, &text_request())
        .await
        .expect_err("rejected");

    assert!(matches!(error, LlmError::Auth { status: 401 }));
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(TEST_KEY) && !shown.contains("1234"),
        "{shown}"
    );
}

#[tokio::test]
async fn native_schema_request_and_reply() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_json.json"));

    let completion = complete(
        &adapter,
        |schema| {
            Format::NativeSchema(SchemaRef {
                name: "probe_test",
                schema,
            })
        },
        Some(0.0),
    )
    .await
    .expect("completion");

    assert_eq!(
        serde_json::from_str::<Value>(&completion.text).expect("json")["title"],
        "A short test"
    );
    assert_eq!(completion.finish, FinishReason::Stop);
    assert_eq!(completion.usage.expect("usage").output_tokens, Some(20));
    let body = rig.server.requests()[0].json();
    assert_eq!(body["stream"], false);
    assert!(body.get("stream_options").is_none());
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["name"], "probe_test");
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        body["response_format"]["json_schema"]["schema"],
        test_schema()
    );
    assert_eq!(body["temperature"], 0.0);
}

#[tokio::test]
async fn forced_tool_request_and_reply() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_tool_call.json"));

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

    assert_eq!(
        serde_json::from_str::<Value>(&completion.text).expect("json")["title"],
        "From a tool"
    );
    let body = rig.server.requests()[0].json();
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "probe_test");
    assert_eq!(body["tools"][0]["function"]["parameters"], test_schema());
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "function", "function": { "name": "probe_test" } })
    );
    assert!(body.get("response_format").is_none());
}

#[tokio::test]
async fn json_mode_request() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_json.json"));

    complete(&adapter, |_| Format::JsonMode, None)
        .await
        .expect("completion");

    assert_eq!(
        rig.server.requests()[0].json()["response_format"],
        json!({ "type": "json_object" })
    );
}

#[tokio::test]
async fn a_refusal_in_a_non_streaming_reply_is_an_error() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_refusal.json"));

    let error = complete(&adapter, |_| Format::JsonMode, None)
        .await
        .expect_err("refusal");

    assert!(matches!(error, LlmError::Refusal));
}

#[tokio::test]
async fn length_in_a_non_streaming_reply_is_reported_not_hidden() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_length.json"));

    let completion = complete(&adapter, |_| Format::JsonMode, None)
        .await
        .expect("completion");

    assert_eq!(completion.finish, FinishReason::Length);
}

#[tokio::test]
async fn an_error_object_in_a_200_reply_is_a_protocol_error() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::json_text(
        200,
        r#"{"error":{"message":"upstream overloaded"}}"#,
    ));

    let error = complete(&adapter, |_| Format::Plain, None)
        .await
        .expect_err("error");

    assert!(matches!(error, LlmError::Protocol(_)), "{error:?}");
}

#[tokio::test]
async fn array_content_parts_are_joined() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::json_text(
        200,
        r#"{"choices":[{"message":{"content":[{"type":"text","text":"ab"},{"type":"text","text":"cd"}]},"finish_reason":"stop"}]}"#,
    ));

    let completion = complete(&adapter, |_| Format::Plain, None)
        .await
        .expect("completion");

    assert_eq!(completion.text, "abcd");
}

#[tokio::test]
async fn a_cancelled_stream_ends_with_cancelled_and_closes_the_connection() {
    let (rig, adapter) = openai_rig().await;
    let first = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"}}]}\n\n";
    rig.server.enqueue(Reply::sse_chunks(vec![
        Chunk {
            delay: Duration::ZERO,
            bytes: first.as_bytes().to_vec(),
        },
        Chunk {
            delay: Duration::from_secs(30),
            bytes: b"data: [DONE]\n\n".to_vec(),
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
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "cancellation must act within 300 ms of the request"
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn cancelling_while_the_server_stalls_returns_cancelled() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::sse_chunks(vec![Chunk {
        delay: Duration::from_secs(30),
        bytes: b"data: [DONE]\n\n".to_vec(),
    }]));
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        canceller.cancel();
    });

    let result = adapter.stream_text(&text_request(), &cancel).await;

    // Whether hyper has sent the response head by then is not specified, so both
    // places the cancel can land are accepted.
    match result {
        Ok(stream) => {
            let error = stream.collect_text().await.expect_err("cancelled");
            assert!(matches!(error, LlmError::Cancelled));
        }
        Err(error) => assert!(matches!(error, LlmError::Cancelled)),
    }
}
