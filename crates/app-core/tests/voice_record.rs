#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Storing a voice session and the background analysis, which must not delay speech.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use app_core::voice::Recording;
use app_core::voice::testing::{ReplyScript, Step};
use common::voice::{Audio, Options, Rig};
use storage::{
    Database, InputMode, L1HelpMode, NewProfile, SessionStatus, Timestamp, TurnRole, UiLanguage,
};

fn clock() -> tutor_engine::Clock {
    let ticks = Arc::new(Mutex::new(0_i64));
    Arc::new(move || {
        let mut n = ticks.lock().unwrap();
        *n += 1;
        Timestamp::from_unix_seconds(1_790_000_000 + *n).expect("timestamp")
    })
}

async fn recording(dir: &tempfile::TempDir) -> (Recording, Database) {
    let db = Database::open(dir.path().join("lumingo.sqlite"))
        .await
        .expect("open the database");
    let profile = db
        .profiles()
        .create(&NewProfile {
            display_name: "Test learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: Timestamp::from_unix_seconds(1_790_000_000).expect("timestamp"),
        })
        .await
        .expect("profile");
    (
        Recording {
            db: db.clone(),
            profile_id: profile.id,
            provider_profile_id: None,
            model: "test-model".to_owned(),
            app_version: "0.0.0".to_owned(),
            link_unit: false,
            clock: clock(),
        },
        db,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn turns_are_stored_and_the_analysis_runs_without_delaying_the_reply() {
    let dir = tempfile::tempdir().expect("dir");
    let (recording, db) = recording(&dir).await;
    let steps = vec![
        Step::Reply(ReplyScript::new(&["Hello there. ", "What is your name?"])),
        Step::Reply(ReplyScript::new(&["Nice to meet you."])),
    ];
    let mut rig = Rig::start(
        steps,
        Options {
            audio: Audio::Playback,
            recording: Some(recording),
            ..Options::default()
        },
    )
    .await;
    // The analysis call takes a second and a half.
    rig.llm.set_structured_delay(Duration::from_millis(1_500));

    rig.handle.open().await.unwrap();
    rig.log.turn_ended(1).await;
    let sent = Instant::now();
    rig.handle.send_text("I am Dewi").await.unwrap();
    rig.log.turn_ended(2).await;
    let reply_took = sent.elapsed();
    // The tutor finished speaking long before the analysis of the learner's turn did.
    assert!(
        reply_took < Duration::from_millis(1_200),
        "the reply waited for the analysis: {reply_took:?}"
    );

    let waited = Instant::now();
    let summary = rig.finish().await;
    assert!(
        waited.elapsed() >= Duration::from_millis(400),
        "finishing waits for the analysis that was still running"
    );
    let record = summary.recording.expect("a recording");
    // The opening line, the learner's turn and the tutor's reply.
    assert_eq!(record.turns_stored, 3);
    assert_eq!(record.unanalysed_turns, 0);
    assert_eq!(
        rig.llm.structured_calls(),
        1,
        "only the learner's turn is analysed"
    );

    let session = db
        .sessions()
        .get(record.session_id)
        .await
        .unwrap()
        .expect("session");
    assert_eq!(session.status, SessionStatus::Completed);
    assert_eq!(
        session.unit_id, None,
        "the unit is not in this database's index"
    );
    let turns = db.turns().list(record.session_id).await.unwrap();
    let roles: Vec<(TurnRole, InputMode)> = turns.iter().map(|t| (t.role, t.input_mode)).collect();
    assert_eq!(
        roles,
        [
            (TurnRole::Tutor, InputMode::None),
            (TurnRole::Learner, InputMode::Text),
            (TurnRole::Tutor, InputMode::None),
        ]
    );
    assert_eq!(turns[1].text, "I am Dewi");
    assert!(
        db.analysis().get(turns[1].id).await.unwrap().is_some(),
        "the learner's turn has its analysis"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spoken_turn_is_stored_as_voice_with_its_speech_time() {
    let dir = tempfile::tempdir().expect("dir");
    let (recording, db) = recording(&dir).await;
    let steps = vec![Step::Reply(ReplyScript::new(&["Good to hear."]))];
    let mut rig = Rig::start(
        steps,
        Options {
            transcripts: vec!["My name is Dewi."],
            recording: Some(recording),
            ..Options::default()
        },
    )
    .await;
    rig.say().await;
    rig.log.turn_ended(1).await;
    let summary = rig.finish().await;
    let record = summary.recording.expect("a recording");
    let turns = db.turns().list(record.session_id).await.unwrap();
    assert_eq!(turns[0].role, TurnRole::Learner);
    assert_eq!(turns[0].input_mode, InputMode::Voice);
    assert_eq!(turns[0].text, "My name is Dewi.");
    assert!(turns[0].speech_ms.unwrap() > 500);
}
