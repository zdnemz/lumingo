#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! No log line, error value, `Display`, `Debug` or serialised output may contain the
//! key (PRD NFR-S1, `context_pack.md` section 10). `tracing` output is captured at
//! TRACE level from every crate, including `reqwest` and `hyper`.

mod common;

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use common::{
    Reply, TEST_KEY, anthropic_rig, client_for, collect, openai_rig, sample, text_request,
};
use llm_client::{
    Capabilities, ChatMessage, CompletionRequest, Contract, Format, LlmClient, LlmError,
    PayloadLog, ProfileSource, Protocol, ProtocolAdapter, ProviderClient, ProviderProfile,
    StructuredRequest,
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::MakeWriter;

/// Pieces of the key that must never appear. Each is long enough to be unique.
const FORBIDDEN: [&str; 4] = [
    TEST_KEY,
    "KEYMATERIAL",
    "0123456789abcdefghij",
    "sk-test-0123",
];

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

fn assert_clean(what: &str, text: &str) {
    for needle in FORBIDDEN {
        assert!(
            !text.contains(needle),
            "{what} contains key material `{needle}`: {text}"
        );
    }
}

fn note(shown: &mut Vec<(String, String)>, what: &str, error: &LlmError) {
    shown.push((what.to_owned(), format!("{error} | {error:?}")));
}

fn echo_error(status: u16, kind: &str) -> Reply {
    Reply::json_text(
        status,
        &format!(
            r#"{{"error":{{"message":"Invalid key {TEST_KEY} (sk-test-0123****KEYMATERIAL) for {kind}","type":"invalid_request_error"}}}}"#
        ),
    )
}

// A current-thread runtime keeps every task, and so every log event, on the thread
// that holds the capturing subscriber.
#[tokio::test(flavor = "current_thread")]
async fn no_log_line_error_or_debug_print_contains_the_key() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let mut shown: Vec<(String, String)> = Vec::new();

    // ---- openai_chat: failures that echo the key, retries, stream errors
    let (rig, adapter) = openai_rig().await;
    shown.push((
        "adapter debug".into(),
        format!("{adapter:?} {:?}", rig.config),
    ));
    for (status, kind) in [
        (400, "bad request"),
        (401, "auth"),
        (403, "forbidden"),
        (404, "missing"),
        (422, "unprocessable"),
    ] {
        rig.server.enqueue(echo_error(status, kind));
        let error = collect(&adapter, &text_request()).await.expect_err("error");
        note(&mut shown, kind, &error);
    }
    rig.server.enqueue(echo_error(500, "server"));
    rig.server.enqueue(echo_error(502, "server again"));
    note(
        &mut shown,
        "5xx",
        &collect(&adapter, &text_request()).await.expect_err("error"),
    );
    rig.server
        .enqueue(Reply::empty(429).with_header("retry-after", "0"));
    rig.server
        .enqueue(Reply::empty(429).with_header("retry-after", "0"));
    rig.server
        .enqueue(Reply::empty(429).with_header("retry-after", "0"));
    note(
        &mut shown,
        "429",
        &collect(&adapter, &text_request()).await.expect_err("error"),
    );
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_error.sse"));
    note(
        &mut shown,
        "stream error",
        &collect(&adapter, &text_request()).await.expect_err("error"),
    );
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));
    collect(&adapter, &text_request())
        .await
        .expect("a good call");

    // ---- anthropic_messages
    let (arig, aadapter) = anthropic_rig().await;
    arig.server.enqueue(Reply::json_text(
        400,
        &format!(r#"{{"type":"error","error":{{"type":"invalid_request_error","message":"x-api-key {TEST_KEY} rejected"}}}}"#),
    ));
    note(
        &mut shown,
        "anthropic 400",
        &collect(&aadapter, &text_request())
            .await
            .expect_err("error"),
    );
    arig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_error.sse"));
    note(
        &mut shown,
        "anthropic stream",
        &collect(&aadapter, &text_request())
            .await
            .expect_err("error"),
    );

    // ---- ladder and probe over HTTP, with invalid output that quotes the key
    let client = client_for(aadapter, &arig.caps);
    let leaky = format!(r#"{{"title": "{TEST_KEY}"}}"#);
    arig.server.enqueue(common::anthropic_text_reply(&leaky));
    arig.server.enqueue(common::anthropic_text_reply(&leaky));
    let request = StructuredRequest::new(
        Contract::ReadingPassage,
        "s",
        vec![ChatMessage::user("u")],
        100,
    );
    note(
        &mut shown,
        "invalid output",
        &client
            .structured(request, CancellationToken::new())
            .await
            .expect_err("invalid"),
    );
    arig.server.enqueue(echo_error(401, "probe"));
    note(
        &mut shown,
        "probe",
        &client
            .probe(&CancellationToken::new())
            .await
            .expect_err("auth"),
    );
    arig.server.enqueue(common::anthropic_text_reply(
        &sample("turn_analysis").to_string(),
    ));
    let ok = StructuredRequest::new(
        Contract::TurnAnalysis,
        "s",
        vec![ChatMessage::user("u")],
        100,
    );
    client
        .structured(ok, CancellationToken::new())
        .await
        .expect("valid");

    // ---- debug and serialised forms of every public type that holds or sees a key
    let profile = ProviderProfile::new(
        "work",
        Protocol::OpenAiChat,
        "https://api.example.com/v1",
        "m",
        Some(TEST_KEY),
        ProfileSource::File,
    )
    .expect("profile");
    shown.push((
        "profile debug".into(),
        format!("{profile:?} {:#?}", profile.key),
    ));
    shown.push((
        "profile info".into(),
        serde_json::to_string(&profile.info()).expect("json"),
    ));
    shown.push((
        "capabilities".into(),
        serde_json::to_string(&Capabilities::default()).expect("json"),
    ));
    let connected = ProviderClient::connect(&profile, llm_client::ClientOptions::default(), None)
        .expect("connect");
    shown.push((
        "client debug".into(),
        format!("{connected:?} {:?}", connected.capabilities()),
    ));

    for (what, text) in &shown {
        assert_clean(what, text);
    }
    let logs = capture.text();
    assert!(
        logs.contains("retrying"),
        "the retry path should have logged something: {logs}"
    );
    assert_clean("captured tracing output", &logs);
    assert!(shown.len() > 15);
}

/// Over one protocol: one answer whose text quotes the key the learner pasted
/// into the request, and one refusal whose body quotes it too. Both bodies are
/// recorded by the payload log, so the caller can scan what was kept.
async fn one_answering_call_and_one_refusal(
    adapter: &impl ProtocolAdapter,
    server: &common::TestServer,
    answer: Reply,
) {
    let messages = [ChatMessage::user(format!("my key is {TEST_KEY}"))];
    let request = CompletionRequest {
        system: "s",
        messages: &messages,
        max_tokens: 50,
        temperature: None,
        format: Format::Plain,
    };
    server.enqueue(answer);
    adapter
        .complete(&request, &CancellationToken::new())
        .await
        .expect("the echoing answer arrives");
    server.enqueue(echo_error(401, "auth"));
    let refused = adapter.complete(&request, &CancellationToken::new()).await;
    assert!(
        matches!(refused, Err(LlmError::Auth { status: 401 })),
        "{refused:?}"
    );
}

// The payload log reads back the request and the response bodies of both
// protocols, so the scan covers what the inspector would show.
#[tokio::test(flavor = "current_thread")]
async fn both_protocols_request_and_response_bodies_hold_no_key() {
    let openai_log = PayloadLog::new(4);
    let (rig, adapter) = openai_rig().await;
    let adapter = adapter.with_payload_log(openai_log.clone());
    one_answering_call_and_one_refusal(
        &adapter,
        &rig.server,
        common::openai_reply(&format!("you wrote {TEST_KEY}")),
    )
    .await;

    let anthropic_log = PayloadLog::new(4);
    let (arig, aadapter) = anthropic_rig().await;
    let aadapter = aadapter.with_payload_log(anthropic_log.clone());
    one_answering_call_and_one_refusal(
        &aadapter,
        &arig.server,
        common::anthropic_text_reply(&format!("you wrote {TEST_KEY}")),
    )
    .await;

    for (protocol, log) in [
        ("openai_chat", &openai_log),
        ("anthropic_messages", &anthropic_log),
    ] {
        let snap = log.snapshot();
        assert_eq!(snap.entries.len(), 2, "{protocol}: an answer and a refusal");
        let shown = format!("{snap:?} {}", serde_json::to_string(&snap).expect("json"));
        assert_clean(&format!("{protocol} request and response bodies"), &shown);
        assert!(
            snap.entries[0].request_body.contains("my key is"),
            "{protocol}: the request was recorded: {shown}"
        );
        assert!(
            snap.entries[0].request_body.contains("[redacted]"),
            "{protocol}: the pasted key is scrubbed from the request: {shown}"
        );
        assert!(
            snap.entries[1]
                .response_body
                .as_deref()
                .is_some_and(|b| b.contains("[redacted]")),
            "{protocol}: the echoed key is scrubbed from the refusal: {shown}"
        );
    }
}
