#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! What the program around the engine can hear and change while a conversation
//! runs: the chat observer, and amending the text of a turn that is still waiting
//! for its analysis (the learner corrected a transcript).

mod common;

use std::sync::{Arc, Mutex};

use assessment_engine::Level;
use common::{
    FakeLlm, TextReply, make_profile, make_session, make_turn, temp_db, test_clock, wait_for,
};
use llm_client::{LlmError, TimeoutKind};
use serde_json::{Value, json};
use storage::{SessionKind, TurnRole};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    AnalysisKind, AnalyzerConfig, ChatConfig, ChatDeps, ChatObserver, ChatTopic, FeedbackMode,
    InputMode, RunReport, TextChat, TurnAnalyzer, TurnToAnalyse,
};

fn no_errors(seq: i64) -> Value {
    json!({ "turns": [{
        "turn_seq": seq, "errors": [], "objective_evidence": [],
        "understood_tutor": "yes", "note_for_next_turn": ""
    }]})
}

#[derive(Default)]
struct Hear {
    stored: Mutex<Vec<(i64, i64)>>,
    analyses: Mutex<Vec<usize>>,
}

impl ChatObserver for Hear {
    fn learner_turn_stored(&self, turn_id: i64, seq: i64) {
        self.stored.lock().unwrap().push((turn_id, seq));
    }

    fn analysis_finished(&self, report: &RunReport) {
        self.analyses.lock().unwrap().push(report.analysed.len());
    }
}

#[tokio::test]
async fn an_observer_hears_the_stored_message_before_the_reply_and_the_analysis_after() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let llm = FakeLlm::new();
    let hear = Arc::new(Hear::default());
    let mut chat = TextChat::start(
        ChatDeps {
            client: llm.clone(),
            db: db.clone(),
            clock: test_clock(),
        },
        ChatConfig {
            profile_id: profile.id,
            provider_profile_id: None,
            model: "test-model".into(),
            level: Level::A2,
            first_language: "Indonesian".into(),
            mode: FeedbackMode::Fluency,
            topic: ChatTopic::Typed("pets".into()),
            app_version: "0.0.0-test".into(),
        },
    )
    .await
    .unwrap()
    .with_observer(hear.clone());
    llm.queue_text(TextReply::Deltas(vec!["Nice cat!"]));
    llm.queue_structured(Ok(no_errors(1)));

    let at_first_delta = Mutex::new(None);
    chat.send(
        "I has a cat",
        |_| {
            let stored = hear.stored.lock().unwrap().clone();
            at_first_delta.lock().unwrap().get_or_insert(stored);
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(
        at_first_delta.lock().unwrap().clone(),
        Some(vec![(1, 1)]),
        "the message was stored and announced before the first delta"
    );
    wait_for(|| !hear.analyses.lock().unwrap().is_empty()).await;
    assert_eq!(hear.analyses.lock().unwrap().clone(), [1]);
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: storage::Database,
    llm: Arc<FakeLlm>,
    analyzer: TurnAnalyzer,
    session_id: i64,
}

async fn fixture() -> Fixture {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::Conversation).await;
    let llm = FakeLlm::new();
    let analyzer = TurnAnalyzer::new(
        AnalyzerConfig {
            profile_id: profile.id,
            session_id: session.id,
            provider_profile_id: None,
            model: "test-model".into(),
            level: Level::A2,
            first_language: "Indonesian".into(),
            kind: AnalysisKind::Turn,
            objectives: Vec::new(),
            target_language: Vec::new(),
        },
        llm.clone(),
        db.clone(),
        test_clock(),
    );
    Fixture {
        _dir: dir,
        db,
        llm,
        analyzer,
        session_id: session.id,
    }
}

async fn spoken_turn(f: &Fixture, text: &str) -> TurnToAnalyse {
    let turn = make_turn(&f.db, f.session_id, TurnRole::Learner, text).await;
    TurnToAnalyse {
        turn_id: turn.id,
        turn_seq: turn.seq,
        input_mode: InputMode::Voice,
        tutor_before: "What do you want?".into(),
        learner_text: text.into(),
        tutor_reply: "Sure.".into(),
    }
}

#[tokio::test]
async fn a_waiting_turn_can_be_amended_and_the_analysis_is_of_the_new_text() {
    let f = fixture().await;
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    let turn = spoken_turn(&f, "I want a coffe").await;
    let report = f
        .analyzer
        .turn_finished(turn.clone(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.waiting, 1, "the failed call leaves the turn waiting");

    assert!(f.analyzer.amend_waiting(turn.turn_id, "I want a coffee"));
    f.llm.queue_structured(Ok(no_errors(turn.turn_seq)));
    f.analyzer.flush(&CancellationToken::new()).await.unwrap();

    let sent = f.llm.structured_requests();
    let last = sent.last().expect("a call was made");
    let text: String = last.messages.iter().map(|m| m.content.clone()).collect();
    assert!(text.contains("I want a coffee"), "{text}");
    assert!(
        !text.contains("coffe\""),
        "the recogniser's text is not sent: {text}"
    );
}

#[tokio::test]
async fn a_turn_that_is_not_waiting_cannot_be_amended() {
    let f = fixture().await;
    let turn = spoken_turn(&f, "I want a coffe").await;
    f.llm.queue_structured(Ok(no_errors(turn.turn_seq)));
    f.analyzer
        .turn_finished(turn.clone(), &CancellationToken::new())
        .await
        .unwrap();
    assert!(
        !f.analyzer.amend_waiting(turn.turn_id, "I want a coffee"),
        "an analysed turn is no longer waiting"
    );
    assert!(
        !f.analyzer.amend_waiting(9_999, "x"),
        "an unknown turn is not waiting"
    );
}
