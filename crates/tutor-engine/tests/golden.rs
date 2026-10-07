//! Golden tests for the tutor turn (S4-05): fixed inputs through prompt
//! assembly, compared to the stored request bodies. Regenerate a golden file
//! only when the prompt or an adapter changes on purpose; a diff here is the
//! point of the test.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

use curriculum::UnitLoader;
use llm_client::openai::TokenParam;
use serde_json::Value;
use tutor_engine::{Channel, FeedbackMode, TutorContext, text_request};

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

fn golden(name: &str) -> Value {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name),
    )
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn roleplay_request() -> llm_client::TextRequest {
    let unit = UnitLoader::new().load_str(EXAMPLE).unwrap();
    let ctx = TutorContext::from_roleplay(
        &unit,
        Some("a11-roleplay-classmate"),
        Channel::Voice,
        Some(FeedbackMode::Accuracy),
        "Indonesian",
    )
    .unwrap();
    text_request("example-model", &ctx, &[], "Hello!", &[], &[])
}

fn topic_request() -> llm_client::TextRequest {
    let ctx = TutorContext::from_topic(
        "Travel plans",
        curriculum::Level::B1,
        Channel::Text,
        FeedbackMode::Fluency,
        "Indonesian",
    );
    text_request(
        "example-model",
        &ctx,
        &[],
        "I want to go to Japan.",
        &["used 'want to' correctly".to_owned()],
        &[],
    )
}

#[test]
fn the_openai_roleplay_body_matches_the_golden_file() {
    let body = llm_client::openai::request_body(&roleplay_request(), TokenParam::MaxTokens, true);
    assert_eq!(body, golden("tutor_turn_1_a1_voice_accuracy.openai.json"));
}

#[test]
fn the_anthropic_roleplay_body_matches_the_golden_file() {
    let body = llm_client::anthropic::request_body(&roleplay_request());
    assert_eq!(
        body,
        golden("tutor_turn_1_a1_voice_accuracy.anthropic.json")
    );
}

#[test]
fn the_openai_topic_body_matches_the_golden_file() {
    let body = llm_client::openai::request_body(&topic_request(), TokenParam::MaxTokens, true);
    assert_eq!(body, golden("tutor_turn_1_b1_text_fluency.openai.json"));
}

#[test]
fn the_same_prompt_text_serves_both_protocols() {
    // Contract rule 6: only the envelope differs. OpenAI carries the system
    // prompt as the first message; Anthropic carries it at the top level.
    let request = roleplay_request();
    let openai = llm_client::openai::request_body(&request, TokenParam::MaxTokens, true);
    let anthropic = llm_client::anthropic::request_body(&request);
    assert_eq!(openai["messages"][0]["content"], anthropic["system"]);
    assert_eq!(openai["messages"][1], anthropic["messages"][0]);
    assert_eq!(
        openai["messages"].as_array().map(Vec::len),
        anthropic["messages"]
            .as_array()
            .map(Vec::len)
            .map(|n| n + 1)
    );
}
