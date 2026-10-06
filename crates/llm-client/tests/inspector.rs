#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! The payload log records what was sent and what came back, over both
//! protocols and for every way a request can end, and it never holds the key.

mod common;

use common::{
    Reply, TEST_KEY, anthropic_text_reply, collect, openai_reply, quick_options, rig, text_request,
};
use llm_client::{
    AnthropicMessages, ChatMessage, CompletionRequest, Format, OpenAiChat, PayloadLog,
    PayloadOutcome, ProtocolAdapter,
};
use tokio_util::sync::CancellationToken;

async fn openai_with_log(log: &PayloadLog) -> (common::Rig, OpenAiChat) {
    let rig = rig("/v1", Some(TEST_KEY), quick_options()).await;
    let adapter = OpenAiChat::new(rig.config.clone())
        .expect("adapter")
        .with_payload_log(log.clone());
    (rig, adapter)
}

async fn complete_plain(
    adapter: &impl ProtocolAdapter,
) -> Result<llm_client::Completion, llm_client::LlmError> {
    let messages = [ChatMessage::user("Say hello.")];
    adapter
        .complete(
            &CompletionRequest {
                system: "You are a tutor.",
                messages: &messages,
                max_tokens: 50,
                temperature: None,
                format: Format::Plain,
            },
            &CancellationToken::new(),
        )
        .await
}

#[tokio::test]
async fn a_completion_is_recorded_with_its_request_and_its_response() {
    let log = PayloadLog::default();
    let (rig, adapter) = openai_with_log(&log).await;
    rig.server.enqueue(openai_reply("Hello there"));
    complete_plain(&adapter).await.expect("completion");

    let snap = log.snapshot();
    assert_eq!(snap.entries.len(), 1);
    let entry = &snap.entries[0];
    assert!(
        entry.endpoint.ends_with("/v1/chat/completions"),
        "{}",
        entry.endpoint
    );
    assert!(!entry.streaming);
    assert!(entry.request_body.contains("Say hello."));
    assert!(entry.request_body.contains("You are a tutor."));
    assert_eq!(entry.status, Some(200));
    assert!(
        entry
            .response_body
            .as_deref()
            .is_some_and(|b| b.contains("Hello there"))
    );
    assert_eq!(entry.outcome, PayloadOutcome::Ok);
}

#[tokio::test]
async fn a_streamed_reply_is_recorded_as_the_event_stream() {
    let log = PayloadLog::default();
    let (rig, adapter) = openai_with_log(&log).await;
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));
    let collected = collect(&adapter, &text_request()).await.expect("stream");
    assert!(!collected.text.is_empty());

    let snap = log.snapshot();
    let entry = &snap.entries[0];
    assert!(entry.streaming);
    assert_eq!(entry.outcome, PayloadOutcome::Ok);
    let body = entry.response_body.as_deref().expect("a recorded stream");
    assert!(body.contains("data:"), "{body}");
    assert!(body.contains("[DONE]"), "{body}");
}

#[tokio::test]
async fn an_error_status_is_recorded_with_the_error_body() {
    let log = PayloadLog::default();
    let (rig, adapter) = openai_with_log(&log).await;
    rig.server
        .enqueue(Reply::json_fixture(401, "openai/error_incorrect_key.json"));
    assert!(complete_plain(&adapter).await.is_err());

    let entry = &log.snapshot().entries[0];
    assert_eq!(entry.status, Some(401));
    assert_eq!(entry.outcome, PayloadOutcome::HttpError);
    assert!(entry.response_body.is_some());
}

#[tokio::test]
async fn a_retried_request_shows_both_attempts() {
    let log = PayloadLog::default();
    let (rig, adapter) = openai_with_log(&log).await;
    rig.server.enqueue(Reply::empty(503));
    rig.server.enqueue(openai_reply("second time"));
    complete_plain(&adapter)
        .await
        .expect("completion after a retry");

    let snap = log.snapshot();
    assert_eq!(snap.entries.len(), 2);
    assert_eq!(snap.entries[0].status, Some(503));
    assert_eq!(snap.entries[0].outcome, PayloadOutcome::HttpError);
    assert_eq!(snap.entries[1].status, Some(200));
    assert_eq!(snap.entries[1].outcome, PayloadOutcome::Ok);
}

#[tokio::test]
async fn a_request_that_never_got_an_answer_is_failed_and_has_no_status() {
    let log = PayloadLog::default();
    let (rig, adapter) = openai_with_log(&log).await;
    let url = rig.config.base_url.clone();
    drop(rig);
    // The server is gone: the connection is refused on every attempt.
    let _ = url;
    assert!(complete_plain(&adapter).await.is_err());
    let snap = log.snapshot();
    assert!(!snap.entries.is_empty());
    for entry in &snap.entries {
        assert_eq!(entry.outcome, PayloadOutcome::Failed);
        assert_eq!(entry.status, None);
        assert!(entry.response_body.is_none());
    }
}

#[tokio::test]
async fn a_stream_dropped_early_is_marked_abandoned() {
    let log = PayloadLog::default();
    let (rig, adapter) = openai_with_log(&log).await;
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));
    let stream = adapter
        .stream_text(&text_request(), &CancellationToken::new())
        .await
        .expect("stream");
    drop(stream);
    let entry = &log.snapshot().entries[0];
    assert_eq!(entry.outcome, PayloadOutcome::Abandoned);
}

#[tokio::test]
async fn the_key_is_in_no_entry_even_when_the_learner_pasted_it_and_the_provider_echoed_it() {
    let log = PayloadLog::default();
    let (rig, adapter) = openai_with_log(&log).await;
    rig.server
        .enqueue(openai_reply(&format!("you wrote {TEST_KEY}")));
    let messages = [ChatMessage::user(format!("my key is {TEST_KEY}"))];
    adapter
        .complete(
            &CompletionRequest {
                system: "s",
                messages: &messages,
                max_tokens: 50,
                temperature: None,
                format: Format::Plain,
            },
            &CancellationToken::new(),
        )
        .await
        .expect("completion");

    let snap = log.snapshot();
    let shown = format!("{snap:?} {}", serde_json::to_string(&snap).expect("json"));
    for piece in [
        TEST_KEY,
        "KEYMATERIAL",
        "0123456789abcdefghij",
        "sk-test-0123",
    ] {
        assert!(!shown.contains(piece), "the log holds {piece}: {shown}");
    }
    assert!(snap.entries[0].request_body.contains("[redacted]"));
    let request = rig.server.requests().remove(0);
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some(format!("Bearer {TEST_KEY}").as_str()),
        "the key still reached the provider; only the record is scrubbed"
    );
}

#[tokio::test]
async fn the_anthropic_adapter_records_too() {
    let log = PayloadLog::default();
    let rig = rig("", Some(TEST_KEY), quick_options()).await;
    let adapter = AnthropicMessages::new(rig.config.clone())
        .expect("adapter")
        .with_payload_log(log.clone());
    rig.server
        .enqueue(anthropic_text_reply("Hi from the other protocol"));
    complete_plain(&adapter).await.expect("completion");

    let entry = &log.snapshot().entries[0];
    assert!(
        entry.endpoint.ends_with("/v1/messages"),
        "{}",
        entry.endpoint
    );
    assert!(entry.request_body.contains("Say hello."));
    assert!(
        entry
            .response_body
            .as_deref()
            .is_some_and(|b| b.contains("Hi from the other protocol"))
    );
    let shown = format!("{:?}", log.snapshot());
    assert!(!shown.contains("KEYMATERIAL"));
}

#[tokio::test]
async fn a_client_without_a_log_records_nothing_and_works() {
    let rig = rig("/v1", Some(TEST_KEY), quick_options()).await;
    let adapter = OpenAiChat::new(rig.config.clone()).expect("adapter");
    rig.server.enqueue(openai_reply("no log"));
    complete_plain(&adapter).await.expect("completion");
}

#[tokio::test]
async fn the_ring_keeps_only_the_newest_entries_and_counts_the_rest() {
    let log = PayloadLog::new(2);
    let (rig, adapter) = openai_with_log(&log).await;
    for n in 0..4 {
        rig.server.enqueue(openai_reply(&format!("reply {n}")));
        complete_plain(&adapter).await.expect("completion");
    }
    let snap = log.snapshot();
    assert_eq!(snap.entries.len(), 2);
    assert_eq!(snap.dropped, 2);
    assert!(
        snap.entries[1]
            .response_body
            .as_deref()
            .is_some_and(|b| b.contains("reply 3"))
    );
}
