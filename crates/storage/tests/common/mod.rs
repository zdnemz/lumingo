#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use storage::{
    Database, InputMode, L1HelpMode, NewProfile, NewSession, NewTurn, Profile, Session,
    SessionKind, Timestamp, Turn, TurnRole, UiLanguage,
};
use tempfile::TempDir;

/// A fresh database in its own temporary directory. Keep the `TempDir` alive for
/// as long as the database is used.
pub async fn temp_db() -> (TempDir, Database) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("lumingo.sqlite"))
        .await
        .expect("open database");
    (dir, db)
}

/// A timestamp `n` seconds after a fixed start, so tests never read the clock.
pub fn ts(n: i64) -> Timestamp {
    Timestamp::from_unix_seconds(1_790_000_000 + n).expect("valid timestamp")
}

pub async fn make_profile(db: &Database) -> Profile {
    db.profiles()
        .create(&NewProfile {
            display_name: "Test learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: ts(0),
        })
        .await
        .expect("create profile")
}

pub async fn make_session(db: &Database, profile_id: i64, kind: SessionKind) -> Session {
    db.sessions()
        .create(&NewSession {
            profile_id,
            kind,
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0-test".to_owned(),
            started_at: ts(10),
        })
        .await
        .expect("create session")
}

pub fn new_turn(session_id: i64, role: TurnRole, text: &str, at: i64) -> NewTurn {
    NewTurn {
        session_id,
        role,
        input_mode: InputMode::Text,
        text: text.to_owned(),
        stt_text: None,
        edited_by_learner: false,
        speech_ms: None,
        pause_ms: None,
        word_count: None,
        created_at: ts(at),
    }
}

pub async fn make_turn(db: &Database, session_id: i64, role: TurnRole, text: &str) -> Turn {
    db.turns()
        .append(&new_turn(session_id, role, text, 20))
        .await
        .expect("append turn")
}

/// A scored, authored, estimate-eligible attempt row. Tests change the fields
/// they care about with struct update syntax.
pub fn new_attempt(profile_id: i64, response_id: &str, dimension: &str) -> storage::NewAttempt {
    storage::NewAttempt {
        profile_id,
        session_id: None,
        unit_id: Some("a1-u01".to_owned()),
        activity_id: "act-1".to_owned(),
        activity_type: "guided_writing".to_owned(),
        response_id: response_id.to_owned(),
        origin: storage::AttemptOrigin::Authored,
        level: storage::Level::A1,
        skill: "writing".to_owned(),
        dimension: dimension.to_owned(),
        scorer: storage::Scorer::RubricLlm,
        scorer_version: "rubric-w1/1".to_owned(),
        raw_score: Some(3.0),
        max_score: Some(4.0),
        normalized: Some(0.75),
        confidence: Some(0.8),
        status: storage::AttemptStatus::Scored,
        counts_toward_estimate: true,
        created_at: ts(100),
    }
}
