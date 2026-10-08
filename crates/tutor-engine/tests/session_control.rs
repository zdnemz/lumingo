#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! What a program that runs a session needs to control: pausing a chat that is
//! waiting, storing a unit run as a checkpoint or drill run, ending a practice
//! run as completed, a draft whose analysis was cancelled, and the summary of a
//! conversation read from what is stored.

mod common;

use std::path::Path;
use std::sync::Arc;

use assessment_engine::Level;
use common::{FakeLlm, TextReply, make_profile, temp_db, test_clock};
use llm_client::LlmError;
use storage::{SessionKind, SessionStatus, UnitStatus};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    ChatConfig, ChatDeps, ChatTopic, FeedbackMode, NoProvider, Phase, RubricCatalog, TextChat,
    TurnState, UnitConfig, UnitEnv, UnitPlayer, Workshop, WorkshopConfig, WorkshopEnv,
    WorkshopTask, ensure_indexed, session_summary,
};

fn cancel() -> CancellationToken {
    CancellationToken::new()
}

#[tokio::test]
async fn a_waiting_chat_pauses_and_resumes_and_a_paused_chat_takes_no_message() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let llm = FakeLlm::new();
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
            level: Level::A1,
            first_language: "Indonesian".into(),
            mode: FeedbackMode::Fluency,
            topic: ChatTopic::Typed("my family".into()),
            app_version: "0.0.0-test".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(chat.pause().unwrap(), Phase::Paused);
    assert!(
        chat.send("hello", |_| {}, &cancel()).await.is_err(),
        "a paused chat refuses a message"
    );
    assert_eq!(
        chat.resume().unwrap(),
        Phase::Active {
            turn: TurnState::Waiting
        }
    );
    llm.queue_text(TextReply::Deltas(vec!["Hi!"]));
    llm.queue_structured(Err(LlmError::Cancelled));
    chat.send("hello", |_| {}, &cancel()).await.unwrap();
}

#[tokio::test]
async fn the_summary_of_a_session_is_read_from_what_is_stored() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = common::make_session(&db, profile.id, SessionKind::Conversation).await;
    common::make_turn(&db, session.id, storage::TurnRole::Learner, "I has a cat").await;
    common::make_turn(&db, session.id, storage::TurnRole::Tutor, "Nice!").await;
    let summary = session_summary(&db, session.id, true).await.unwrap();
    assert_eq!(summary.learner_turns, 1);
    assert_eq!(
        summary.unanalysed_turns,
        [1],
        "the turn has no analysis yet"
    );
    assert!(
        summary.analysis_unreliable,
        "the caller's mark is passed on"
    );
    assert!(summary.top_errors.is_empty());
}

fn example() -> curriculum::LoadedUnit {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../curriculum/examples/a1-u01.example.json");
    curriculum::load_unit_file(&path).expect("the example unit loads")
}

async fn player(kind: SessionKind) -> (tempfile::TempDir, storage::Database, UnitPlayer) {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let loaded = example();
    ensure_indexed(&db, &loaded.unit, &loaded.checksum, common::ts(1))
        .await
        .unwrap();
    let env = UnitEnv {
        client: Arc::new(NoProvider),
        db: db.clone(),
        clock: test_clock(),
        model: "none".into(),
        provider_profile_id: None,
        provider_qualified: false,
        grammar: None,
        word_levels: None,
        rubrics: Arc::new(RubricCatalog::default()),
        drill: None,
        tts: None,
    };
    let player = UnitPlayer::start_as(
        env,
        UnitConfig {
            profile_id: profile.id,
            first_language: "Indonesian".into(),
            app_version: "0.0.0-test".into(),
        },
        loaded.unit,
        kind,
    )
    .await
    .unwrap();
    (dir, db, player)
}

#[tokio::test]
async fn a_unit_run_is_stored_as_the_kind_it_was_started_as() {
    for kind in [
        SessionKind::Lesson,
        SessionKind::Checkpoint,
        SessionKind::Drill,
    ] {
        let (_dir, db, player) = player(kind).await;
        let stored = db
            .sessions()
            .get(player.session_id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.kind, kind);
    }
}

#[tokio::test]
async fn a_practice_run_ends_completed_and_leaves_the_unit_where_it_was() {
    let (_dir, db, mut player) = player(SessionKind::Drill).await;
    let unit_id = player.unit().id.clone();
    player.finish_practice().await.unwrap();
    let stored = db
        .sessions()
        .get(player.session_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, SessionStatus::Completed);
    let progress = db
        .unit_progress()
        .get(stored.profile_id, &unit_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        progress.status,
        UnitStatus::InProgress,
        "a drill run does not pass a unit"
    );
}

#[tokio::test]
async fn a_draft_whose_analysis_is_cancelled_is_queued_for_later() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let llm = FakeLlm::new();
    let env = WorkshopEnv {
        client: llm.clone(),
        db: db.clone(),
        clock: test_clock(),
        model: "test-model".into(),
        provider_profile_id: None,
        grammar: None,
        word_levels: None,
        provider_qualified: false,
    };
    let workshop = Workshop::start(
        env,
        WorkshopConfig {
            profile_id: profile.id,
            level: Level::A2,
            first_language: "Indonesian".into(),
            app_version: "0.0.0-test".into(),
            prompt_id: None,
            rubric: None,
            task: WorkshopTask {
                prompt: "Write a note.".into(),
                content_points: Vec::new(),
                min_words: None,
            },
        },
    )
    .await
    .unwrap();
    let submission = workshop
        .submit_draft("I has a note for you.")
        .await
        .unwrap();
    llm.queue_structured(Err(LlmError::Cancelled));
    let result = workshop.analyse_draft(&submission, &cancel()).await;
    assert!(matches!(
        result,
        Err(tutor_engine::EngineError::Llm(LlmError::Cancelled))
    ));
    assert_eq!(
        db.pending_scoring().oldest(10).await.unwrap().len(),
        1,
        "the stored draft waits for a provider instead of being lost"
    );
}
