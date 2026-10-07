//! One T2 run end to end with a scripted client: the request that goes out,
//! the filters on the way back, and the outcome types. The live smoke test is
//! `examples/analysis_live.rs`, owner-run.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use llm_client::{LlmError, StructuredOutput, StructuredRequest, TextRequest};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisFailure, AnalysisInput, AnalysisTurn, CONVERSATION_ERROR_CAP, Channel, FeedbackMode,
    InputMode, LlmClient, TextStream, TutorContext, analysis_request, run_analysis,
};

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

/// A client that answers structured calls from a script and records requests.
struct ScriptedStructured {
    answers: Mutex<Vec<Result<StructuredOutput, LlmError>>>,
    sent: Mutex<Vec<StructuredRequest>>,
}

impl ScriptedStructured {
    fn new(answers: Vec<Result<StructuredOutput, LlmError>>) -> Self {
        Self {
            answers: Mutex::new(answers),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<StructuredRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl LlmClient for ScriptedStructured {
    fn stream_text(
        &self,
        _request: TextRequest,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<TextStream, LlmError>> + Send + '_>> {
        Box::pin(async { Err(LlmError::Transport("stream not scripted".to_owned())) })
    }

    fn structured(
        &self,
        request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<StructuredOutput, LlmError>> + Send + '_>> {
        self.sent.lock().unwrap().push(request);
        let answer = self.answers.lock().unwrap().remove(0);
        Box::pin(async move { answer })
    }
}

fn outcome(value: serde_json::Value) -> Result<StructuredOutput, LlmError> {
    Ok(StructuredOutput {
        value,
        level: llm_client::Level::NativeSchema,
        repaired: false,
    })
}

fn input() -> AnalysisInput {
    AnalysisInput {
        level: curriculum::Level::A1,
        l1: "Indonesian".to_owned(),
        input_mode: InputMode::Voice,
        objectives: vec![tutor_engine::ObjectivePair {
            id: "a1-u01/o1-greet".to_owned(),
            can_do: "Greet someone".to_owned(),
        }],
        target_language: vec!["hello".to_owned()],
        turns: vec![AnalysisTurn {
            turn_seq: 1,
            tutor_before: "Hello! What is your name?".to_owned(),
            learner_text: "I am go to school yesterday.".to_owned(),
            tutor_reply: "Nice to meet you. Say: I went to school.".to_owned(),
        }],
    }
}

#[tokio::test]
async fn one_run_filters_the_scripted_answer_and_reports_the_ladder() {
    let client = ScriptedStructured::new(vec![outcome(serde_json::json!({
        "turns": [{
            "turn_seq": 1,
            "errors": [
                { "category": "verb_tense", "quote": "go", "correction": "went",
                  "severity": "major", "addressed_in_reply": true },
                { "category": "spelling", "quote": "yesterda", "correction": "yesterday",
                  "severity": "minor", "addressed_in_reply": false },
                { "category": "word_choice", "quote": "not in the text", "correction": "x",
                  "severity": "minor", "addressed_in_reply": false }
            ],
            "objective_evidence": [
                { "objective_id": "a1-u01/o1-greet", "status": "partial", "quote": "I am" },
                { "objective_id": "a1-u01/nope", "status": "demonstrated", "quote": "I am" }
            ],
            "understood_tutor": "partly",
            "note_for_next_turn": "Practise the past tense of go."
        }]
    }))]);
    let cancel = CancellationToken::new();
    let result = run_analysis(&client, &input(), CONVERSATION_ERROR_CAP, &cancel)
        .await
        .unwrap();

    // Voice: spelling is dropped. The quote filter drops the invented error.
    assert_eq!(result.filtered.turns[0].errors.len(), 1);
    assert_eq!(result.filtered.turns[0].errors[0].category, "verb_tense");
    assert!(result.filtered.turns[0].errors[0].addressed_in_reply);
    // Unknown objective dropped.
    assert_eq!(result.filtered.turns[0].objective_evidence.len(), 1);
    assert_eq!(result.filtered.counts.produced, 5);
    assert_eq!(result.filtered.counts.dropped, 3);
    assert_eq!(result.filtered.notes(), ["Practise the past tense of go."]);
    assert_eq!(result.ladder_level, 1);
    assert!(!result.repaired);

    // The request: bundled contract schema, temperature 0, the input JSON.
    let sent = client.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].schema_name, "turn_analysis");
    assert_eq!(sent[0].temperature, Some(0.0));
    let schema = &sent[0].schema;
    assert_eq!(
        schema["$id"],
        "urn:local-english-tutor:contract:turn_analysis:1.0"
    );
    let content: serde_json::Value = serde_json::from_str(&sent[0].messages[0].content).unwrap();
    assert_eq!(content["level"], "A1");
    assert_eq!(content["input_mode"], "voice");
    assert_eq!(content["turns"][0]["turn_seq"], 1);
    assert_eq!(content["objectives"][0]["id"], "a1-u01/o1-greet");
    // The system prompt carries the cap of this call.
    let system = sent[0].system.as_deref().unwrap();
    assert!(system.contains("At most 5 errors per turn"));
    // The version is recorded with the stored row, not in the prompt text.
    assert!(!system.contains(tutor_engine::TURN_ANALYSIS_VERSION));
}

#[tokio::test]
async fn the_draft_cap_reaches_the_prompt_and_the_filter() {
    let client = ScriptedStructured::new(vec![outcome(serde_json::json!({
        "turns": [{
            "turn_seq": 1,
            "errors": (1..=21).map(|n| serde_json::json!({
                "category": "word_choice", "quote": format!("w{n}"), "correction": "x",
                "severity": "minor", "addressed_in_reply": false
            })).collect::<Vec<_>>(),
            "objective_evidence": [],
            "understood_tutor": "yes",
            "note_for_next_turn": ""
        }]
    }))]);
    let mut draft = input();
    draft.input_mode = InputMode::Text;
    draft.turns[0].learner_text = (1..=21)
        .map(|n| format!("w{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let cancel = CancellationToken::new();
    let result = run_analysis(&client, &draft, tutor_engine::DRAFT_ERROR_CAP, &cancel)
        .await
        .unwrap();
    assert_eq!(result.filtered.turns[0].errors.len(), 20);
    assert_eq!(result.filtered.counts.dropped, 1);
    let sent = client.requests();
    assert!(
        sent[0]
            .system
            .as_deref()
            .unwrap()
            .contains("At most 20 errors per turn")
    );
}

#[tokio::test]
async fn a_provider_failure_comes_back_typed_for_the_cadence() {
    let client = ScriptedStructured::new(vec![Err(LlmError::RateLimited {
        retry_after: Some(std::time::Duration::from_secs(3)),
    })]);
    let cancel = CancellationToken::new();
    let error = run_analysis(&client, &input(), CONVERSATION_ERROR_CAP, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AnalysisFailure::Provider(LlmError::RateLimited { .. })
    ));
}

#[test]
fn the_request_of_a_roleplay_input_carries_its_targets_and_objectives() {
    let unit = curriculum::UnitLoader::new().load_str(EXAMPLE).unwrap();
    let ctx = TutorContext::from_roleplay(
        &unit,
        Some("a11-roleplay-classmate"),
        Channel::Voice,
        Some(FeedbackMode::Accuracy),
        "Indonesian",
    )
    .unwrap();
    let input = AnalysisInput::from_activity(
        &unit,
        Some("a11-roleplay-classmate"),
        InputMode::Voice,
        "Indonesian",
        vec![AnalysisTurn {
            turn_seq: 3,
            tutor_before: "What's your name?".to_owned(),
            learner_text: "My name Dewi.".to_owned(),
            tutor_reply: "Nice to meet you, Dewi.".to_owned(),
        }],
    )
    .unwrap();

    assert_eq!(input.level, curriculum::Level::A1);
    assert_eq!(input.objectives.len(), 3);
    assert_eq!(input.objectives[0].id, "a1-u01/o1-greet");
    assert_eq!(
        input.objectives[0].can_do,
        "I can greet someone and say goodbye with simple words."
    );
    // The roleplay's "Language to bring out" matches T1's list for the same
    // activity, so both calls describe the session the same way.
    assert_eq!(input.target_language, ctx.target_language);

    let request = analysis_request(&input, CONVERSATION_ERROR_CAP).unwrap();
    let content: serde_json::Value = serde_json::from_str(&request.messages[0].content).unwrap();
    assert_eq!(content["target_language"][0], "hello");
    assert_eq!(content["turns"][0]["tutor_before"], "What's your name?");
}
