#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

//! Learning data written straight through the storage repositories, the way the
//! tutor and assessment code will write it.

use app_core::AppCore;
use storage::{
    Attempt, AttemptOrigin, AttemptStatus, EvidenceKind, Level, NewAttempt, NewEvidence,
    NewSession, ObjectiveMastery, Scorer, Session, SessionKind, UnitProgress, UnitStatus,
};
use tempfile::TempDir;

use super::{ManualClock, TestCore, config};

/// Copies the example unit into the test's unit folder, so the index has a unit
/// that progress rows can point at.
pub fn write_example_unit(dir: &TempDir) {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../curriculum/examples/a1-u01.example.json");
    let units = dir.path().join("units");
    std::fs::create_dir_all(&units).expect("units dir");
    std::fs::copy(source, units.join("a1-u01.json")).expect("copy the example unit");
}

pub async fn test_core_with_unit() -> TestCore {
    let dir = tempfile::tempdir().expect("temp dir");
    write_example_unit(&dir);
    let clock = ManualClock::new("2026-10-05T08:00:00.000Z", "2026-10-05");
    let core = AppCore::open(config(&dir, &clock, &[]))
        .await
        .expect("open the core");
    TestCore { core, dir, clock }
}

/// The words the tests look for in the database file after a delete.
pub const LEARNER_TEXT: &str = "ZEBRAQUILL writes a sentence";

/// A learner turn holding [`LEARNER_TEXT`] with a kept recording: the file
/// exists on disk, inside the data directory. Returns the file's path.
pub async fn seed_turn_with_recording(core: &AppCore, session_id: i64) -> std::path::PathBuf {
    let at = core.clock().now();
    let turn = core
        .database()
        .turns()
        .append(&storage::NewTurn {
            session_id,
            role: storage::TurnRole::Learner,
            input_mode: storage::InputMode::Voice,
            text: LEARNER_TEXT.to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: Some(1200),
            pause_ms: Some(100),
            word_count: Some(4),
            created_at: at,
        })
        .await
        .expect("turn");
    let relative = format!("audio/turn-{}.wav", turn.id);
    let file = core.config().data_dir.join(&relative);
    std::fs::create_dir_all(file.parent().unwrap()).expect("audio dir");
    std::fs::write(&file, b"RIFF....WAVE").expect("recording");
    core.database()
        .audio_clips()
        .add(turn.id, &relative, 1200, &at)
        .await
        .expect("clip");
    file
}

/// A session with one scored attempt and its evidence, plus unit progress and
/// mastery rows. Returns the session and the attempt.
pub async fn seed_session_with_attempt(core: &AppCore, kind: SessionKind) -> (Session, Attempt) {
    let db = core.database();
    let at = core.clock().now();
    let session = db
        .sessions()
        .create(&NewSession {
            profile_id: core.profile_id(),
            kind,
            unit_id: Some("a1-u01".to_owned()),
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: at,
        })
        .await
        .expect("session");
    let attempt = db
        .attempts()
        .insert(&NewAttempt {
            profile_id: core.profile_id(),
            session_id: Some(session.id),
            unit_id: Some("a1-u01".to_owned()),
            activity_id: "act-1".to_owned(),
            activity_type: "mcq".to_owned(),
            response_id: "resp-1".to_owned(),
            origin: AttemptOrigin::Authored,
            level: Level::A1,
            skill: "listening".to_owned(),
            dimension: "accuracy".to_owned(),
            scorer: Scorer::Deterministic,
            scorer_version: "det/1".to_owned(),
            raw_score: Some(1.0),
            max_score: Some(1.0),
            normalized: Some(1.0),
            confidence: Some(1.0),
            status: AttemptStatus::Scored,
            counts_toward_estimate: true,
            created_at: at,
        })
        .await
        .expect("attempt");
    db.evidence()
        .add(&NewEvidence {
            attempt_id: attempt.id,
            kind: EvidenceKind::ResponseText,
            content: Some(format!("I am from Jakarta. {LEARNER_TEXT}")),
            data: None,
            created_at: at,
        })
        .await
        .expect("evidence");
    db.unit_progress()
        .set(&UnitProgress {
            profile_id: core.profile_id(),
            unit_id: "a1-u01".to_owned(),
            status: UnitStatus::InProgress,
            best_checkpoint: Some(0.5),
            updated_at: at,
        })
        .await
        .expect("unit progress");
    db.mastery()
        .set(&ObjectiveMastery {
            profile_id: core.profile_id(),
            objective_id: "a1-u01/o1-greet".to_owned(),
            mastery: 0.75,
            attempts: 4,
            last_attempt_at: Some(at),
        })
        .await
        .expect("mastery");
    (session, attempt)
}
