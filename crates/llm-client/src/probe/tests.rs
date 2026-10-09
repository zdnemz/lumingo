//! Probe tests with a fake transport.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::client::LlmClient;
use crate::error::{TimeoutKind, TransportKind};
use crate::fake::{FakeAdapter, Reply, Seen, rejected, stream_of, text};
use crate::profile::Protocol;
use crate::types::{FinishReason, StreamSummary, StructuredRequest};

fn probe_json() -> Reply {
    text(
        &json!({"title": "A short test", "word_count": 3, "is_ok": true, "check": "schema_received"})
            .to_string(),
    )
}

fn turn_analysis() -> Value {
    json!({"turns": [{
        "turn_seq": 1,
        "errors": [{"category": "article", "quote": "a apple", "correction": "an apple", "severity": "minor", "addressed_in_reply": false}],
        "objective_evidence": [{"objective_id": "o1", "status": "partial", "quote": "a apple"}],
        "understood_tutor": "yes",
        "note_for_next_turn": "Practise a and an."
    }]})
}

fn rubric_score() -> Value {
    json!({
        "dimension_scores": [{"dimension": "accuracy", "band": "3", "evidence_quotes": ["I like it"], "reason": "Mostly correct."}],
        "content_points": [{"point": "likes it", "covered": true, "quote": "I like it"}],
        "on_task": true,
        "feedback_en": "Good.",
        "feedback_l1": "Bagus."
    })
}

fn practice_items() -> Value {
    json!({"items": [{
        "type": "mcq", "objective_id": "o1", "grammar_id": "g1", "stem": "She ___ tea.",
        "options": ["drink", "drinks"], "answer_index": 1, "text": "", "answers": [], "tokens": [],
        "answer": "", "explanation_en": "Third person.", "explanation_l1": "Orang ketiga."
    }]})
}

fn reading_passage() -> Value {
    json!({
        "title": "A day",
        "passage": "Ana wakes up. She eats rice.",
        "glossary": [{"word": "wakes up", "gloss_l1": "bangun", "example": "He wakes up late."}],
        "questions": [{"stem": "What does Ana eat?", "options": ["Rice", "Bread"], "answer_index": 0,
                       "explanation_en": "She eats rice.", "explanation_l1": "Dia makan nasi."}]
    })
}

fn contract_replies() -> Vec<Reply> {
    [
        turn_analysis(),
        rubric_score(),
        practice_items(),
        reading_passage(),
    ]
    .iter()
    .map(|v| text(&v.to_string()))
    .collect()
}

fn full_script() -> Vec<Reply> {
    let mut replies = vec![text("ok"), probe_json()];
    replies.extend(contract_replies());
    replies
}

fn setup(
    protocol: Protocol,
    replies: Vec<Reply>,
    streams: Vec<crate::fake::StreamReply>,
) -> (ProviderClient, Arc<FakeAdapter>) {
    let adapter = Arc::new(FakeAdapter::new(protocol, replies).with_streams(streams));
    let client = ProviderClient::from_adapter(adapter.clone(), crate::caps::CapsHandle::default());
    (client, adapter)
}

async fn run_probe(client: &ProviderClient) -> Result<Capabilities, LlmError> {
    client.probe(&CancellationToken::new()).await
}

#[tokio::test]
async fn a_healthy_provider_fills_the_capabilities_object() {
    let (client, adapter) = setup(
        Protocol::OpenAiChat,
        full_script(),
        vec![stream_of(&["Sunny", " today."])],
    );

    let caps = run_probe(&client).await.expect("probe");

    assert_eq!(caps.probe_version, Capabilities::PROBE_VERSION);
    assert!(caps.auth_ok);
    assert!(caps.stream_ok);
    assert_eq!(caps.ttft_ms, Some(120));
    assert!(caps.supports_usage_in_stream, "the stream reported usage");
    assert_eq!(caps.structured_level, Some(LadderLevel::NativeSchema));
    assert_eq!(
        caps.contracts_ok,
        [
            "turn_analysis",
            "rubric_score",
            "practice_items",
            "reading_passage"
        ]
    );
    assert_eq!(client.capabilities(), caps, "the result is stored");
    let seen: Vec<Seen> = adapter.requests().into_iter().map(|r| r.format).collect();
    assert_eq!(
        seen,
        vec![
            Seen::Plain,
            Seen::Native,
            Seen::Native,
            Seen::Native,
            Seen::Native,
            Seen::Native
        ],
        "step 1, the test schema, then one request per contract"
    );
}

#[tokio::test]
async fn the_probe_serialises_to_the_documented_json_shape() {
    let (client, _) = setup(
        Protocol::OpenAiChat,
        full_script(),
        vec![stream_of(&["Sunny", " today."])],
    );

    let caps = run_probe(&client).await.expect("probe");
    let value = serde_json::to_value(&caps).expect("json");

    let mut keys: Vec<&str> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "auth_ok",
            "contracts_ok",
            "probe_version",
            "rate_limit",
            "stream_ok",
            "structured_level",
            "supports_temperature",
            "supports_usage_in_stream",
            "token_limit_param",
            "tokens_per_second",
            "ttft_ms"
        ]
    );
    assert_eq!(value["structured_level"], 1);
    assert_eq!(value["token_limit_param"], "max_tokens");
    let back: Capabilities = serde_json::from_value(value).expect("round trip");
    assert_eq!(back, caps);
}

#[tokio::test]
async fn level_two_is_chosen_when_the_native_schema_reply_is_not_valid() {
    let mut replies = vec![text("ok"), text("Sure, here is some JSON."), probe_json()];
    replies.extend(contract_replies());
    let (client, adapter) = setup(Protocol::OpenAiChat, replies, vec![stream_of(&["Hi"])]);

    let caps = run_probe(&client).await.expect("probe");

    assert_eq!(caps.structured_level, Some(LadderLevel::ForcedTool));
    assert_eq!(caps.contracts_ok.len(), 4);
    let seen: Vec<Seen> = adapter.requests().into_iter().map(|r| r.format).collect();
    assert_eq!(&seen[..3], [Seen::Plain, Seen::Native, Seen::Tool]);
    assert!(
        seen[3..].iter().all(|s| *s == Seen::Tool),
        "the contracts are probed at the chosen level"
    );
}

#[tokio::test]
async fn the_probe_walks_every_level_on_openai_chat_and_skips_json_mode_on_anthropic() {
    let mut openai = vec![
        text("ok"),
        rejected("a"),
        rejected("b"),
        rejected("c"),
        probe_json(),
    ];
    openai.extend(contract_replies());
    let (client, _) = setup(Protocol::OpenAiChat, openai, vec![stream_of(&["Hi"])]);
    assert_eq!(
        run_probe(&client).await.expect("probe").structured_level,
        Some(LadderLevel::PromptOnly)
    );

    let mut anthropic = vec![text("ok"), rejected("a"), rejected("b"), probe_json()];
    anthropic.extend(contract_replies());
    let (client, adapter) = setup(
        Protocol::AnthropicMessages,
        anthropic,
        vec![stream_of(&["Hi"])],
    );
    let caps = run_probe(&client).await.expect("probe");
    assert_eq!(caps.structured_level, Some(LadderLevel::PromptOnly));
    let seen: Vec<Seen> = adapter.requests().into_iter().map(|r| r.format).collect();
    assert_eq!(
        &seen[..4],
        [Seen::Plain, Seen::Native, Seen::Tool, Seen::Plain]
    );
}

#[tokio::test]
async fn a_provider_with_no_working_level_reports_none_and_probes_no_contracts() {
    let replies = vec![text("ok"), text("x"), text("x"), text("x"), text("x")];
    let (client, adapter) = setup(Protocol::OpenAiChat, replies, vec![stream_of(&["Hi"])]);

    let caps = run_probe(&client).await.expect("the probe itself works");

    assert!(caps.auth_ok);
    assert_eq!(caps.structured_level, None);
    assert!(caps.contracts_ok.is_empty());
    assert_eq!(adapter.requests().len(), 5);
}

#[tokio::test]
async fn a_contract_that_fails_is_left_out_of_contracts_ok() {
    let mut replies = vec![text("ok"), probe_json()];
    replies.push(text(&turn_analysis().to_string()));
    replies.push(text(&json!({"on_task": true}).to_string()));
    replies.push(text(&practice_items().to_string()));
    replies.push(text(&reading_passage().to_string()));
    let (client, _) = setup(Protocol::OpenAiChat, replies, vec![stream_of(&["Hi"])]);

    let caps = run_probe(&client).await.expect("probe");

    assert_eq!(
        caps.contracts_ok,
        ["turn_analysis", "practice_items", "reading_passage"]
    );
}

#[tokio::test]
async fn a_bad_key_is_the_error_and_marks_auth_as_failed() {
    let (client, adapter) = setup(
        Protocol::OpenAiChat,
        vec![Err(LlmError::Auth { status: 401 })],
        vec![],
    );
    client.caps.update(|caps| caps.auth_ok = true);

    let error = run_probe(&client).await.expect_err("auth");

    assert!(matches!(error, LlmError::Auth { status: 401 }));
    assert!(!client.capabilities().auth_ok);
    assert_eq!(
        adapter.requests().len(),
        1,
        "nothing else is sent with a rejected key"
    );
}

#[tokio::test]
async fn a_rejected_model_name_stops_the_probe_with_the_providers_message() {
    let (client, _) = setup(
        Protocol::OpenAiChat,
        vec![rejected("The model `nope` does not exist")],
        vec![],
    );

    let error = run_probe(&client).await.expect_err("rejected");

    match error {
        LlmError::Rejected { message, .. } => assert!(message.contains("does not exist")),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn a_stream_that_times_out_is_a_finding_and_the_probe_goes_on() {
    let mut replies = vec![text("ok"), probe_json()];
    replies.extend(contract_replies());
    let (client, _) = setup(
        Protocol::OpenAiChat,
        replies,
        vec![Err(LlmError::Timeout(TimeoutKind::FirstToken))],
    );

    let caps = run_probe(&client).await.expect("probe");

    assert!(!caps.stream_ok);
    assert_eq!(caps.ttft_ms, None);
    assert_eq!(caps.structured_level, Some(LadderLevel::NativeSchema));
}

#[tokio::test]
async fn a_stream_that_breaks_midway_is_not_ok() {
    let events = vec![
        Ok(StreamEvent::Delta("Sun".to_owned())),
        Err(LlmError::Transport(TransportKind::Body)),
    ];
    let mut replies = vec![text("ok"), probe_json()];
    replies.extend(contract_replies());
    let (client, _) = setup(Protocol::OpenAiChat, replies, vec![Ok(events)]);

    let caps = run_probe(&client).await.expect("probe");

    assert!(!caps.stream_ok);
}

#[tokio::test]
async fn a_stream_without_usage_clears_the_usage_flag() {
    let events = vec![
        Ok(StreamEvent::Delta("Hello there.".to_owned())),
        Ok(StreamEvent::Finished(StreamSummary {
            finish: FinishReason::Stop,
            usage: None,
            time_to_first_token: Some(std::time::Duration::from_millis(300)),
        })),
    ];
    let mut replies = vec![text("ok"), probe_json()];
    replies.extend(contract_replies());
    let (client, _) = setup(Protocol::OpenAiChat, replies, vec![Ok(events)]);

    let caps = run_probe(&client).await.expect("probe");

    assert!(caps.stream_ok);
    assert!(!caps.supports_usage_in_stream);
}

#[tokio::test]
async fn rate_limit_headers_from_any_step_are_kept() {
    let mut step1 = text("ok");
    if let Ok(completion) = &mut step1 {
        completion.rate_limit = RateLimit {
            rpm: Some(50),
            rpd: None,
        };
    }
    let mut step3 = probe_json();
    if let Ok(completion) = &mut step3 {
        completion.rate_limit = RateLimit {
            rpm: Some(999),
            rpd: Some(1000),
        };
    }
    let mut replies = vec![step1, step3];
    replies.extend(contract_replies());
    let (client, _) = setup(
        Protocol::AnthropicMessages,
        replies,
        vec![stream_of(&["Hi"])],
    );

    let caps = run_probe(&client).await.expect("probe");

    assert_eq!(
        caps.rate_limit,
        RateLimit {
            rpm: Some(50),
            rpd: Some(1000)
        }
    );
}

#[tokio::test]
async fn a_timeout_during_the_structured_steps_aborts_the_probe() {
    let replies = vec![text("ok"), Err(LlmError::Timeout(TimeoutKind::Total))];
    let (client, _) = setup(Protocol::OpenAiChat, replies, vec![stream_of(&["Hi"])]);

    let error = run_probe(&client).await.expect_err("timeout");

    assert!(matches!(error, LlmError::Timeout(TimeoutKind::Total)));
}

#[tokio::test]
async fn a_forced_structured_level_is_what_the_next_call_uses() {
    // The live checks force a level (`TUTOR_LLM_FORCE_LEVEL`) to compare what
    // a provider does at each one; the forced level is what the next
    // structured call starts from.
    let (client, adapter) = setup(
        Protocol::OpenAiChat,
        vec![text(&turn_analysis().to_string())],
        vec![],
    );
    client.set_structured_level(LadderLevel::ForcedTool);
    assert_eq!(
        client.capabilities().structured_level,
        Some(LadderLevel::ForcedTool)
    );

    let out = client
        .structured(
            StructuredRequest::new(
                Contract::TurnAnalysis,
                "s",
                vec![ChatMessage::user("u")],
                100,
            ),
            CancellationToken::new(),
        )
        .await
        .expect("valid");
    assert_eq!(out.ladder_level, LadderLevel::ForcedTool);
    let seen: Vec<Seen> = adapter.requests().into_iter().map(|r| r.format).collect();
    assert_eq!(seen, vec![Seen::Tool]);
}

#[tokio::test]
async fn probing_again_resets_the_validity_window() {
    let mut replies: Vec<Reply> = (0..crate::ladder::VALIDITY_WINDOW)
        .map(|_| text(&turn_analysis().to_string()))
        .collect();
    replies.extend(full_script());
    let (client, _) = setup(Protocol::OpenAiChat, replies, vec![stream_of(&["Hi"])]);
    for _ in 0..crate::ladder::VALIDITY_WINDOW {
        client
            .structured(
                StructuredRequest::new(
                    Contract::TurnAnalysis,
                    "s",
                    vec![ChatMessage::user("u")],
                    100,
                ),
                CancellationToken::new(),
            )
            .await
            .expect("valid");
    }
    assert_eq!(client.validity_rate(), Some(1.0));

    run_probe(&client).await.expect("probe");

    assert_eq!(client.validity_rate(), None);
}
