//! Ladder tests with a fake transport: each level, each failure, the repair call.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::error::{InvalidReason, TimeoutKind};
use crate::fake::{FakeAdapter, Reply, Seen, cut_off, rejected, text, text_with};
use crate::profile::Protocol;
use crate::schema::Contract;

fn valid_reply() -> Value {
    json!({
        "turns": [{
            "turn_seq": 1,
            "errors": [{
                "category": "verb_tense",
                "quote": "He go",
                "correction": "He goes",
                "severity": "minor",
                "addressed_in_reply": false
            }],
            "objective_evidence": [],
            "understood_tutor": "yes",
            "note_for_next_turn": "Practise the third person."
        }]
    })
}

fn valid() -> Reply {
    text(&valid_reply().to_string())
}

fn request() -> StructuredRequest {
    StructuredRequest::new(
        Contract::TurnAnalysis,
        "You analyse turns.",
        vec![ChatMessage::user("Analyse this.")],
        500,
    )
}

fn setup(
    protocol: Protocol,
    replies: Vec<Reply>,
) -> (ProviderClient, Arc<FakeAdapter>, CapsHandle) {
    let adapter = Arc::new(FakeAdapter::new(protocol, replies));
    let caps = CapsHandle::default();
    let client = ProviderClient::from_adapter(adapter.clone(), caps.clone());
    (client, adapter, caps)
}

async fn run(client: &ProviderClient) -> Result<StructuredOutput, LlmError> {
    client.structured(request(), CancellationToken::new()).await
}

fn formats(adapter: &FakeAdapter) -> Vec<Seen> {
    adapter.requests().into_iter().map(|r| r.format).collect()
}

#[tokio::test]
async fn level_one_is_tried_first_and_the_level_is_cached() {
    let (client, adapter, caps) = setup(Protocol::OpenAiChat, vec![valid()]);

    let out = run(&client).await.expect("valid");

    assert_eq!(out.ladder_level, LadderLevel::NativeSchema);
    assert!(!out.repaired);
    assert_eq!(out.calls, 1);
    assert_eq!(out.value, valid_reply());
    assert_eq!(formats(&adapter), vec![Seen::Native]);
    assert_eq!(
        caps.snapshot().structured_level,
        Some(LadderLevel::NativeSchema)
    );
}

#[tokio::test]
async fn a_rejected_level_one_falls_back_to_a_forced_tool_call_and_remembers() {
    let (client, adapter, caps) = setup(
        Protocol::OpenAiChat,
        vec![
            rejected("response_format is not supported"),
            valid(),
            valid(),
        ],
    );

    let out = run(&client).await.expect("valid at level 2");
    assert_eq!(out.ladder_level, LadderLevel::ForcedTool);
    assert_eq!(out.calls, 2, "the rejected request and the one that worked");
    assert_eq!(
        caps.snapshot().structured_level,
        Some(LadderLevel::ForcedTool)
    );

    // The next call goes straight to the cached level.
    run(&client).await.expect("valid again");
    assert_eq!(
        formats(&adapter),
        vec![Seen::Native, Seen::Tool, Seen::Tool]
    );
}

#[tokio::test]
async fn openai_chat_reaches_json_mode_with_the_schema_in_the_prompt() {
    let (client, adapter, _) = setup(
        Protocol::OpenAiChat,
        vec![rejected("no schema"), rejected("no tools"), valid()],
    );

    let out = run(&client).await.expect("valid at level 3");

    assert_eq!(out.ladder_level, LadderLevel::JsonMode);
    assert_eq!(
        formats(&adapter),
        vec![Seen::Native, Seen::Tool, Seen::Json]
    );
    let system = &adapter.requests()[2].system;
    assert!(system.starts_with("You analyse turns."));
    assert!(system.contains("JSON Schema"));
    assert!(
        system.contains("\"understood_tutor\""),
        "the schema text is in the prompt"
    );
    assert!(!system.contains("$schema"), "root annotations are not sent");
}

#[tokio::test]
async fn anthropic_messages_has_no_json_mode_so_level_two_is_followed_by_level_four() {
    let (client, adapter, caps) = setup(
        Protocol::AnthropicMessages,
        vec![rejected("no output_config"), rejected("no tools"), valid()],
    );

    let out = run(&client).await.expect("valid at level 4");

    assert_eq!(out.ladder_level, LadderLevel::PromptOnly);
    assert_eq!(
        formats(&adapter),
        vec![Seen::Native, Seen::Tool, Seen::Plain]
    );
    assert!(adapter.requests()[2].system.contains("JSON Schema"));
    assert_eq!(
        caps.snapshot().structured_level,
        Some(LadderLevel::PromptOnly)
    );
}

#[tokio::test]
async fn prompt_only_output_wrapped_in_a_code_fence_and_prose_is_accepted() {
    let wrapped = format!(
        "Here is the analysis:\n```json\n{}\n```\nLet me know if you need more.",
        valid_reply()
    );
    let (client, _, caps) = setup(Protocol::AnthropicMessages, vec![text(&wrapped)]);
    caps.update(|c| c.structured_level = Some(LadderLevel::PromptOnly));

    let out = run(&client).await.expect("valid");

    assert!(!out.repaired);
    assert_eq!(out.value, valid_reply());
}

#[tokio::test]
async fn when_every_level_is_rejected_the_last_rejection_is_returned() {
    let (client, adapter, caps) = setup(
        Protocol::OpenAiChat,
        vec![rejected("1"), rejected("2"), rejected("3"), rejected("4")],
    );

    let error = run(&client).await.expect_err("nothing works");

    match error {
        LlmError::Rejected { message, .. } => assert_eq!(message, "4"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(adapter.requests().len(), 4);
    assert_eq!(
        caps.snapshot().structured_level,
        None,
        "nothing worked, so nothing is cached"
    );
}

#[tokio::test]
async fn invalid_json_is_repaired_once_with_the_reply_and_the_error_list() {
    let (client, adapter, _) = setup(
        Protocol::OpenAiChat,
        vec![text("I think so, yes."), valid()],
    );

    let out = run(&client).await.expect("repaired");

    assert!(out.repaired);
    assert_eq!(out.calls, 2);
    let repair = &adapter.requests()[1];
    assert_eq!(
        repair.format,
        Seen::Native,
        "the repair uses the same level"
    );
    assert_eq!(repair.messages.len(), 3);
    assert_eq!(
        repair.messages[0],
        ("User".to_owned(), "Analyse this.".to_owned())
    );
    assert_eq!(
        repair.messages[1],
        ("Assistant".to_owned(), "I think so, yes.".to_owned())
    );
    assert_eq!(
        repair.messages[2].0, "User",
        "the conversation must end with a user turn"
    );
    assert!(
        repair.messages[2]
            .1
            .contains("did not contain a JSON value")
    );
    assert!(repair.messages[2].1.contains("only the corrected JSON"));
}

#[tokio::test]
async fn schema_errors_are_listed_by_path_in_the_repair_call() {
    let broken = json!({"turns": [{"turn_seq": "one"}]}).to_string();
    let (client, adapter, _) = setup(Protocol::OpenAiChat, vec![text(&broken), valid()]);

    run(&client).await.expect("repaired");

    let repair_prompt = &adapter.requests()[1].messages[2].1;
    assert!(
        repair_prompt.contains("/turns/0/turn_seq"),
        "{repair_prompt}"
    );
    assert!(repair_prompt.contains("did not match the schema"));
}

#[tokio::test]
async fn a_second_invalid_reply_is_invalid_output_and_carries_no_model_text() {
    let secret = "LEARNER-SECRET-SENTENCE";
    let bad = json!({"turns": [{"turn_seq": secret}]}).to_string();
    let (client, adapter, _) = setup(Protocol::OpenAiChat, vec![text(&bad), text(&bad)]);

    let error = run(&client).await.expect_err("invalid");

    match &error {
        LlmError::InvalidOutput(invalid) => {
            assert_eq!(invalid.reason, InvalidReason::SchemaMismatch);
            assert_eq!(invalid.ladder_level, LadderLevel::NativeSchema);
            assert!(invalid.repaired);
            assert!(
                invalid.paths.iter().any(|p| p == "/turns/0/turn_seq"),
                "{:?}",
                invalid.paths
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(secret),
        "validator messages echo values and must not be kept: {shown}"
    );
    assert_eq!(adapter.requests().len(), 2, "one repair, no more");
}

#[tokio::test]
async fn text_without_json_twice_is_invalid_output_not_json() {
    let (client, _, _) = setup(Protocol::OpenAiChat, vec![text("no"), text("still no")]);

    let error = run(&client).await.expect_err("invalid");

    match error {
        LlmError::InvalidOutput(invalid) => assert_eq!(invalid.reason, InvalidReason::NotJson),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn enum_values_in_another_case_are_accepted_without_a_repair() {
    let mut reply = valid_reply();
    reply["turns"][0]["errors"][0]["category"] = json!("Verb_Tense");
    reply["turns"][0]["errors"][0]["severity"] = json!("MINOR");
    reply["turns"][0]["understood_tutor"] = json!("Yes");
    let (client, _, _) = setup(Protocol::AnthropicMessages, vec![text(&reply.to_string())]);

    let out = run(&client).await.expect("valid after normalisation");

    assert!(!out.repaired);
    assert_eq!(out.value, valid_reply());
}

#[tokio::test]
async fn a_reply_cut_off_by_the_token_limit_is_retried_once_with_a_larger_limit() {
    let (client, adapter, _) = setup(
        Protocol::OpenAiChat,
        vec![cut_off("{\"turns\": [{\"turn_se"), valid()],
    );

    let out = run(&client).await.expect("valid on the retry");

    assert!(!out.repaired, "a retry for length is not a repair");
    assert_eq!(out.calls, 2);
    let limits: Vec<u32> = adapter.requests().iter().map(|r| r.max_tokens).collect();
    assert_eq!(limits, vec![500, 1000]);
}

#[tokio::test]
async fn a_reply_that_is_cut_off_again_goes_to_repair() {
    let (client, adapter, _) = setup(
        Protocol::OpenAiChat,
        vec![cut_off("{\"turns\": ["), cut_off("{\"turns\": [{"), valid()],
    );

    let out = run(&client).await.expect("repaired");

    assert!(out.repaired);
    assert_eq!(out.calls, 3);
    assert_eq!(
        adapter.requests()[2].max_tokens,
        1000,
        "the repair keeps the larger limit"
    );
}

#[tokio::test]
async fn a_refusal_is_returned_without_a_repair_or_a_level_change() {
    let (client, adapter, caps) = setup(Protocol::AnthropicMessages, vec![Err(LlmError::Refusal)]);

    let error = run(&client).await.expect_err("refused");

    assert!(matches!(error, LlmError::Refusal));
    assert_eq!(adapter.requests().len(), 1);
    assert_eq!(caps.snapshot().structured_level, None);
}

#[tokio::test]
async fn credential_rate_limit_and_timeout_errors_do_not_move_the_ladder() {
    let errors = [
        LlmError::Auth { status: 401 },
        LlmError::RateLimited {
            retry_after: Some(Duration::from_secs(3)),
        },
        LlmError::Timeout(TimeoutKind::Total),
        LlmError::Server {
            status: 503,
            message: "busy".to_owned(),
        },
    ];
    for error in errors {
        let (client, adapter, _) = setup(Protocol::OpenAiChat, vec![Err(error)]);

        let result = run(&client).await;

        assert!(result.is_err());
        assert_eq!(
            adapter.requests().len(),
            1,
            "no level change for {result:?}"
        );
    }
}

#[tokio::test]
async fn a_cancelled_token_stops_before_any_request() {
    let (client, adapter, _) = setup(Protocol::OpenAiChat, vec![valid()]);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let error = client
        .structured(request(), cancel)
        .await
        .expect_err("cancelled");

    assert!(matches!(error, LlmError::Cancelled));
    assert!(adapter.requests().is_empty());
}

#[tokio::test]
async fn usage_is_summed_over_every_request_of_the_call() {
    let (client, _, _) = setup(Protocol::OpenAiChat, vec![text("nope"), valid()]);

    let out = run(&client).await.expect("repaired");

    let usage = out.usage.expect("usage");
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        (Some(20), Some(10))
    );
}

#[tokio::test]
async fn a_forced_tool_call_that_comes_back_as_prose_is_still_validated() {
    let (client, _, caps) = setup(
        Protocol::OpenAiChat,
        vec![text_with("", FinishReason::ToolCalls), valid()],
    );
    caps.update(|c| c.structured_level = Some(LadderLevel::ForcedTool));

    let out = run(&client).await.expect("repaired");

    assert!(out.repaired);
    assert_eq!(out.ladder_level, LadderLevel::ForcedTool);
}

async fn feed(client: &ProviderClient, adapter_replies: usize) {
    for _ in 0..adapter_replies {
        run(client).await.expect("structured call");
    }
}

#[tokio::test]
async fn the_validity_window_triggers_a_reprobe_below_ninety_percent() {
    // 44 valid first replies and 6 that needed a repair: 88 percent.
    let mut replies = Vec::new();
    for _ in 0..44 {
        replies.push(valid());
    }
    for _ in 0..6 {
        replies.push(text("not json"));
        replies.push(valid());
    }
    let (client, _, _) = setup(Protocol::OpenAiChat, replies);
    assert_eq!(
        client.validity_rate(),
        None,
        "no rate before the window is full"
    );

    feed(&client, 49).await;
    assert!(!client.needs_reprobe(), "49 calls are not a full window");
    feed(&client, 1).await;

    let rate = client.validity_rate().expect("full window");
    assert!((rate - 0.88).abs() < 1e-9, "{rate}");
    assert!(client.needs_reprobe());
}

#[tokio::test]
async fn exactly_ninety_percent_does_not_trigger_a_reprobe() {
    let mut replies = Vec::new();
    for _ in 0..45 {
        replies.push(valid());
    }
    for _ in 0..5 {
        replies.push(text("not json"));
        replies.push(valid());
    }
    let (client, _, _) = setup(Protocol::OpenAiChat, replies);

    feed(&client, 50).await;

    assert!(!client.needs_reprobe());
    assert!((client.validity_rate().expect("rate") - 0.9).abs() < 1e-9);
}

#[test]
fn ladder_order_depends_on_the_protocol() {
    use LadderLevel::*;
    let walk = |protocol| {
        let mut levels = vec![NativeSchema];
        while let Some(next) = levels.last().and_then(|l| l.next(protocol)) {
            levels.push(next);
        }
        levels
    };
    assert_eq!(
        walk(Protocol::OpenAiChat),
        vec![NativeSchema, ForcedTool, JsonMode, PromptOnly]
    );
    assert_eq!(
        walk(Protocol::AnthropicMessages),
        vec![NativeSchema, ForcedTool, PromptOnly]
    );
}
