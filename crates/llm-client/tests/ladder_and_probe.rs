#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! The probe and the structured-output ladder over real HTTP against the scripted
//! server, with both protocol adapters.

mod common;

use common::{
    CONTRACT_NAMES, Reply, anthropic_rig, anthropic_text_reply, anthropic_tool_reply, client_for,
    openai_reply, openai_rig, sample,
};
use llm_client::{
    Capabilities, ChatMessage, Contract, InvalidReason, LadderLevel, LlmClient, LlmError,
    StructuredRequest,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn request(contract: Contract) -> StructuredRequest {
    StructuredRequest::new(
        contract,
        "You analyse.",
        vec![ChatMessage::user("Go.")],
        400,
    )
}

#[tokio::test]
async fn openai_probe_runs_the_five_steps_in_order() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_text.json"));
    rig.server
        .enqueue(Reply::sse_fixture("openai/stream_basic.sse"));
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_json.json"));
    for name in CONTRACT_NAMES {
        rig.server.enqueue(openai_reply(&sample(name).to_string()));
    }
    let client = client_for(adapter, &rig.caps);

    let caps = client
        .probe(&CancellationToken::new())
        .await
        .expect("probe");

    assert!(caps.auth_ok && caps.stream_ok);
    assert_eq!(caps.structured_level, Some(LadderLevel::NativeSchema));
    assert_eq!(caps.contracts_ok, CONTRACT_NAMES);
    assert!(
        caps.supports_usage_in_stream,
        "the fixture stream has a usage chunk"
    );
    assert!(caps.ttft_ms.is_some());
    assert_eq!(client.capabilities(), caps);

    let requests = rig.server.requests();
    assert_eq!(requests.len(), 7);
    let bodies: Vec<_> = requests.iter().map(|r| r.json()).collect();
    // Step 1: a few tokens, no stream.
    assert_eq!(bodies[0]["stream"], false);
    assert_eq!(bodies[0]["max_tokens"], 16);
    assert_eq!(bodies[0]["temperature"], 0.0);
    // Step 2: streaming.
    assert_eq!(bodies[1]["stream"], true);
    // Step 3: the three-field test schema at level 1.
    assert_eq!(
        bodies[2]["response_format"]["json_schema"]["name"],
        "probe_test"
    );
    // Step 4: one request per contract, in the same order as `Contract::ALL`.
    for (body, name) in bodies[3..].iter().zip(CONTRACT_NAMES) {
        assert_eq!(body["response_format"]["json_schema"]["name"], name);
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert!(
            body["response_format"]["json_schema"]["schema"]
                .get("$schema")
                .is_none()
        );
    }
}

#[tokio::test]
async fn anthropic_probe_runs_the_five_steps_in_order() {
    let (rig, adapter) = common::anthropic_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(200, "anthropic/message_text.json"));
    rig.server
        .enqueue(Reply::sse_fixture("anthropic/stream_basic.sse"));
    rig.server
        .enqueue(Reply::json_fixture(200, "anthropic/message_json.json"));
    for name in CONTRACT_NAMES {
        rig.server
            .enqueue(anthropic_text_reply(&sample(name).to_string()));
    }
    let client = client_for(adapter, &rig.caps);

    let caps = client
        .probe(&CancellationToken::new())
        .await
        .expect("probe");

    assert!(caps.auth_ok && caps.stream_ok);
    assert_eq!(caps.structured_level, Some(LadderLevel::NativeSchema));
    assert_eq!(caps.contracts_ok, CONTRACT_NAMES);
    assert!(caps.supports_usage_in_stream);
    let bodies: Vec<_> = rig.server.requests().iter().map(|r| r.json()).collect();
    assert_eq!(bodies.len(), 7);
    assert_eq!(bodies[2]["output_config"]["format"]["type"], "json_schema");
    for body in &bodies[3..] {
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert!(
            body["output_config"]["format"]["schema"]
                .get("$id")
                .is_none()
        );
    }
}

#[tokio::test]
async fn a_probe_with_a_bad_key_returns_the_auth_error_and_sends_nothing_more() {
    let (rig, adapter) = openai_rig().await;
    rig.server
        .enqueue(Reply::json_fixture(401, "openai/error_incorrect_key.json"));
    let client = client_for(adapter, &rig.caps);

    let error = client
        .probe(&CancellationToken::new())
        .await
        .expect_err("auth");

    assert!(matches!(error, LlmError::Auth { status: 401 }));
    assert_eq!(rig.server.hits(), 1);
}

#[tokio::test]
async fn a_probe_learns_the_token_limit_parameter_through_the_adapter() {
    let (rig, adapter) = openai_rig().await;
    rig.server.enqueue(Reply::json_fixture(
        400,
        "openai/error_max_tokens_unsupported.json",
    ));
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_text.json"));
    // Stop after step 1: a stream failure is a finding, and step 3 is rejected everywhere.
    rig.server.enqueue(Reply::empty(500));
    let client = client_for(adapter, &rig.caps);

    let _ = client.probe(&CancellationToken::new()).await;

    let caps = rig.caps.snapshot();
    assert_eq!(
        caps.token_limit_param,
        llm_client::TokenLimitParam::MaxCompletionTokens
    );
    assert!(
        rig.server.requests()[1]
            .json()
            .get("max_completion_tokens")
            .is_some()
    );
}

#[tokio::test]
async fn anthropic_without_output_config_uses_a_forced_tool_call_and_remembers_it() {
    let (rig, adapter) = anthropic_rig().await;
    let client = client_for(adapter, &rig.caps);
    rig.server.enqueue(Reply::json_fixture(
        400,
        "anthropic/error_output_config_unsupported.json",
    ));
    rig.server.enqueue(anthropic_tool_reply(
        "turn_analysis",
        &sample("turn_analysis"),
    ));
    rig.server.enqueue(anthropic_tool_reply(
        "turn_analysis",
        &sample("turn_analysis"),
    ));

    let out = client
        .structured(request(Contract::TurnAnalysis), CancellationToken::new())
        .await
        .expect("level 2");
    assert_eq!(out.ladder_level, LadderLevel::ForcedTool);
    assert_eq!(out.calls, 2);
    assert_eq!(out.value, sample("turn_analysis"));
    assert_eq!(
        client.capabilities().structured_level,
        Some(LadderLevel::ForcedTool)
    );

    let again = client
        .structured(request(Contract::TurnAnalysis), CancellationToken::new())
        .await
        .expect("cached");
    assert_eq!(again.calls, 1, "the cached level is used straight away");

    let bodies: Vec<_> = rig.server.requests().iter().map(|r| r.json()).collect();
    assert!(bodies[0].get("output_config").is_some());
    for body in &bodies[1..] {
        assert_eq!(
            body["tool_choice"],
            json!({"type": "tool", "name": "turn_analysis"})
        );
        assert_eq!(body["tools"][0]["name"], "turn_analysis");
        assert!(body.get("output_config").is_none());
    }
}

#[tokio::test]
async fn openai_json_mode_level_sends_json_object_and_the_schema_text() {
    let (rig, adapter) = openai_rig().await;
    rig.caps.replace(Capabilities {
        structured_level: Some(LadderLevel::JsonMode),
        ..Capabilities::default()
    });
    let client = client_for(adapter, &rig.caps);
    rig.server.enqueue(openai_reply(&format!(
        "```json\n{}\n```",
        sample("rubric_score")
    )));

    let out = client
        .structured(request(Contract::RubricScore), CancellationToken::new())
        .await
        .expect("valid");

    assert_eq!(out.ladder_level, LadderLevel::JsonMode);
    assert_eq!(out.value, sample("rubric_score"));
    let body = rig.server.requests()[0].json();
    assert_eq!(body["response_format"], json!({"type": "json_object"}));
    let system = body["messages"][0]["content"]
        .as_str()
        .expect("system prompt");
    assert!(system.starts_with("You analyse."));
    assert!(system.contains("\"dimension_scores\""));
}

#[tokio::test]
async fn a_bad_reply_is_repaired_once_over_http() {
    let (rig, adapter) = openai_rig().await;
    let client = client_for(adapter, &rig.caps);
    rig.server
        .enqueue(openai_reply("Sure, the analysis is fine."));
    rig.server
        .enqueue(openai_reply(&sample("practice_items").to_string()));

    let out = client
        .structured(request(Contract::PracticeItems), CancellationToken::new())
        .await
        .expect("repaired");

    assert!(out.repaired);
    assert_eq!(out.calls, 2);
    let repair = rig.server.requests()[1].json();
    let messages = repair["messages"].as_array().expect("messages");
    let roles: Vec<&str> = messages
        .iter()
        .map(|m| m["role"].as_str().expect("role"))
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(messages[2]["content"], "Sure, the analysis is fine.");
}

#[tokio::test]
async fn a_reply_that_stays_invalid_is_invalid_output_after_exactly_two_requests() {
    let (rig, adapter) = anthropic_rig().await;
    let client = client_for(adapter, &rig.caps);
    rig.server
        .enqueue(anthropic_text_reply(r#"{"title": "no passage"}"#));
    rig.server
        .enqueue(anthropic_text_reply(r#"{"title": "still no passage"}"#));

    let error = client
        .structured(request(Contract::ReadingPassage), CancellationToken::new())
        .await
        .expect_err("invalid");

    match error {
        LlmError::InvalidOutput(invalid) => {
            assert_eq!(invalid.reason, InvalidReason::SchemaMismatch);
            assert!(invalid.repaired);
            assert_eq!(invalid.ladder_level, LadderLevel::NativeSchema);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(rig.server.hits(), 2);
}

#[tokio::test]
async fn a_cut_off_reply_is_retried_with_a_larger_limit_over_http() {
    let (rig, adapter) = openai_rig().await;
    let client = client_for(adapter, &rig.caps);
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_length.json"));
    rig.server
        .enqueue(openai_reply(&sample("turn_analysis").to_string()));

    let out = client
        .structured(request(Contract::TurnAnalysis), CancellationToken::new())
        .await
        .expect("valid");

    assert!(!out.repaired);
    let limits: Vec<u64> = rig
        .server
        .requests()
        .iter()
        .map(|r| r.json()["max_tokens"].as_u64().expect("limit"))
        .collect();
    assert_eq!(limits, [400, 800]);
}

#[tokio::test]
async fn a_provider_refusal_is_an_error_without_a_repair() {
    let (rig, adapter) = openai_rig().await;
    let client = client_for(adapter, &rig.caps);
    rig.server
        .enqueue(Reply::json_fixture(200, "openai/chat_refusal.json"));

    let error = client
        .structured(request(Contract::TurnAnalysis), CancellationToken::new())
        .await
        .expect_err("refused");

    assert!(matches!(error, LlmError::Refusal));
    assert_eq!(rig.server.hits(), 1);
}

#[tokio::test]
async fn max_tokens_stop_reason_from_anthropic_is_retried_like_length() {
    let (rig, adapter) = anthropic_rig().await;
    let client = client_for(adapter, &rig.caps);
    rig.server.enqueue(Reply::json_fixture(
        200,
        "anthropic/message_max_tokens.json",
    ));
    rig.server
        .enqueue(anthropic_text_reply(&sample("turn_analysis").to_string()));

    let out = client
        .structured(request(Contract::TurnAnalysis), CancellationToken::new())
        .await
        .expect("valid");

    assert_eq!(out.calls, 2);
    assert!(!out.repaired);
}
