//! The S4-06 path end to end with a scripted client and a real database: one
//! learner turn is analysed, filtered, stored as an analysis plus error events
//! plus stats, and the note comes back for the next T1 turn. A fake client
//! exists only in test code (AGENTS.md).
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use common::TempDir;
use llm_client::{LlmError, StructuredOutput, StructuredRequest, TextRequest};
use storage::{
    Database, InputMode as StoredInputMode, L1HelpMode, NewProfile, NewSession, NewTurn,
    SessionKind, TurnRole, UiLanguage,
};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisCadence, AnalysisInput, AnalysisTurn, CONVERSATION_ERROR_CAP, InputMode, LlmClient,
    NotesForNextTurn, ObjectivePair, ReliabilityWindow, TURN_ANALYSIS_VERSION, TextStream,
    run_analysis,
};

const NOW: &str = "2026-10-07T08:00:00.000Z";

struct ScriptedStructured {
    answers: Mutex<Vec<Result<StructuredOutput, LlmError>>>,
}

impl ScriptedStructured {
    fn new(answers: Vec<Result<StructuredOutput, LlmError>>) -> Self {
        Self {
            answers: Mutex::new(answers),
        }
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
        _request: StructuredRequest,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<StructuredOutput, LlmError>> + Send + '_>> {
        let answer = self.answers.lock().unwrap().remove(0);
        Box::pin(async move { answer })
    }
}

fn answer(value: serde_json::Value) -> Result<StructuredOutput, LlmError> {
    Ok(StructuredOutput {
        value,
        level: llm_client::Level::NativeSchema,
        repaired: false,
    })
}

#[tokio::test]
async fn one_turn_becomes_a_stored_analysis_events_stats_and_a_note() {
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let session = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Conversation,
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let turn = db
        .add_turn(NewTurn {
            session_id: session.id,
            seq: 1,
            role: TurnRole::Learner,
            input_mode: StoredInputMode::Voice,
            text: "I am go to school yesterday.".to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(6),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    // The caller queues the turn; normal cadence makes it due at once.
    let mut cadence = AnalysisCadence::new();
    cadence.enqueue(turn.seq);
    assert_eq!(cadence.due(), [turn.seq]);

    let client = ScriptedStructured::new(vec![answer(serde_json::json!({
        "turns": [{
            "turn_seq": turn.seq,
            "errors": [{
                "category": "verb_tense", "quote": "go", "correction": "went",
                "severity": "major", "addressed_in_reply": false
            }],
            "objective_evidence": [],
            "understood_tutor": "yes",
            "note_for_next_turn": "Ask about yesterday."
        }]
    }))]);
    let input = AnalysisInput {
        level: curriculum::Level::A1,
        l1: "Indonesian".to_owned(),
        input_mode: InputMode::Voice,
        objectives: vec![ObjectivePair {
            id: "a1-u01/o1-greet".to_owned(),
            can_do: "Greet someone".to_owned(),
        }],
        target_language: vec!["hello".to_owned()],
        turns: vec![AnalysisTurn {
            turn_seq: turn.seq,
            tutor_before: "What did you do?".to_owned(),
            learner_text: turn.text.clone(),
            tutor_reply: "Almost! Say: I went to school yesterday.".to_owned(),
        }],
    };
    let cancel = CancellationToken::new();
    let outcome = run_analysis(&client, &input, CONVERSATION_ERROR_CAP, &cancel)
        .await
        .unwrap();
    cadence.mark_analysed(&cadence.due());

    // Store what the run produced, mapping the filtered entries to rows.
    let analysis = storage::TurnAnalysis {
        turn_id: turn.id,
        analysis_json: outcome.filtered.analysis_json().unwrap(),
        contract_version: TURN_ANALYSIS_VERSION.to_owned(),
        ladder_level: i64::from(outcome.ladder_level),
        model: "scripted-model".to_owned(),
        created_at: NOW.to_owned(),
    };
    let events: Vec<storage::NewErrorEvent> = outcome.filtered.turns[0]
        .errors
        .iter()
        .map(|error| storage::NewErrorEvent {
            turn_id: turn.id,
            profile_id: profile.id,
            category: error.category.clone(),
            quote: error.quote.clone(),
            correction: error.correction.clone(),
            severity: match error.severity.as_str() {
                "minor" => storage::Severity::Minor,
                "major" => storage::Severity::Major,
                _ => storage::Severity::Blocking,
            },
            addressed: error.addressed_in_reply,
            created_at: NOW.to_owned(),
        })
        .collect();
    db.save_analysis_bundle(&analysis, &events).await.unwrap();

    // Read back: the row, the events, the stats.
    let stored = db.turn_analysis(turn.id).await.unwrap();
    assert_eq!(stored.contract_version, "turn_analysis/1");
    assert!(stored.analysis_json.contains("\"turn_seq\":1"));
    let stored_events = db.error_events_for_turn(turn.id).await.unwrap();
    assert_eq!(stored_events.len(), 1);
    assert_eq!(stored_events[0].category, "verb_tense");
    assert_eq!(stored_events[0].severity, storage::Severity::Major);
    let stats = db.error_stats(profile.id).await.unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].count, 1);

    // The note is ready for the next T1 turn.
    let mut notes = NotesForNextTurn::new();
    notes.update(&outcome.filtered);
    assert_eq!(notes.notes(), ["Ask about yesterday."]);

    // And the reliability window saw the run.
    let mut window = ReliabilityWindow::new();
    window.record(outcome.filtered.counts);
    assert_eq!(window.analyses(), 1);
    assert!(!window.unreliable());
    assert!(cadence.pending() == 0);
}
