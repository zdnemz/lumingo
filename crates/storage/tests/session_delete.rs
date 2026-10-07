//! The session-delete behaviour of DATA_MODEL section 4, on the real schema.
//!
//! Deleting a session removes its text (turns, analysis, error events,
//! generated content, recordings, free-speech pronunciation rows by cascade;
//! evidence and pending scoring of attempts made in it by the schema trigger).
//! The attempt rows stay with `session_id` NULL, so scores and estimates are
//! unchanged. Rows that are not part of the session are untouched.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use common::TempDir;
use storage::{
    AttemptOrigin, AttemptStatus, Database, EvidenceKind, InputMode, L1HelpMode, Level, NewAttempt,
    NewEvidence, NewPendingScoring, NewProfile, NewSession, NewTurn, Scorer, SessionKind, Skill,
    TurnAnalysis, TurnRole, UiLanguage,
};

const NOW: &str = "2026-10-07T08:00:00.000Z";

/// One profile with a writing session and a reading session, each carrying the
/// rows the delete rules cover.
async fn seeded(dir: &TempDir) -> (Database, i64, i64) {
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::En,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    let writing = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Writing,
            unit_id: None,
            activity_id: Some("w1".to_owned()),
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let reading = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Reading,
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
            session_id: writing.id,
            seq: 1,
            role: TurnRole::Learner,
            input_mode: InputMode::Text,
            text: "This is my first draft.".to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(5),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    db.add_turn(NewTurn {
        session_id: reading.id,
        seq: 1,
        role: TurnRole::Learner,
        input_mode: InputMode::Text,
        text: "A reading answer.".to_owned(),
        stt_text: None,
        edited_by_learner: false,
        speech_ms: None,
        pause_ms: None,
        word_count: Some(3),
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();

    // Analysis, an error event, a recording and a free-speech pronunciation
    // row for the writing session; generated content for the reading session.
    db.save_turn_analysis(&TurnAnalysis {
        turn_id: turn.id,
        analysis_json: r#"{"errors":[]}"#.to_owned(),
        contract_version: "1".to_owned(),
        ladder_level: 2,
        model: "test-model".to_owned(),
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO error_events (turn_id, profile_id, category, quote, correction, severity, created_at) \
         VALUES (?, ?, 'articles', 'a apple', 'an apple', 'minor', ?)",
    )
    .bind(turn.id)
    .bind(profile.id)
    .bind(NOW)
    .execute(db.writer())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO generated_content (session_id, kind, content_json, contract_version, model, created_at) \
         VALUES (?, 'reading_passage', '{\"title\":\"x\"}', '1', 'test-model', ?)",
    )
    .bind(reading.id)
    .bind(NOW)
    .execute(db.writer())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO audio_clips (turn_id, path, duration_ms, created_at) VALUES (?, '/tmp/x.wav', 1200, ?)",
    )
    .bind(turn.id)
    .bind(NOW)
    .execute(db.writer())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pron_results (turn_id, mode, word, word_index, phone_expected, gop, flagged, start_ms, end_ms, engine_version) \
         VALUES (?, 'free_speech', 'apple', 0, 'AE1', 0.4, 1, 0, 500, 'test')",
    )
    .bind(turn.id)
    .execute(db.writer())
    .await
    .unwrap();

    // A scored attempt made in the writing session, with evidence and a queue row.
    let attempt = db
        .add_attempt(NewAttempt {
            profile_id: profile.id,
            session_id: Some(writing.id),
            unit_id: None,
            activity_id: "w1".to_owned(),
            activity_type: "guided_writing".to_owned(),
            response_id: "r1".to_owned(),
            origin: AttemptOrigin::Authored,
            level: Level::A1,
            skill: Skill::Writing,
            dimension: "task".to_owned(),
            scorer: Scorer::Deterministic,
            scorer_version: "det/1".to_owned(),
            raw_score: Some(0.8),
            max_score: Some(1.0),
            normalized: Some(0.8),
            confidence: Some(1.0),
            status: AttemptStatus::Scored,
            counts_toward_estimate: true,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    db.add_evidence(NewEvidence {
        attempt_id: attempt.id,
        kind: EvidenceKind::ResponseText,
        content: Some("This is my first draft.".to_owned()),
        data_json: None,
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();
    db.enqueue_pending_scoring(NewPendingScoring {
        attempt_id: attempt.id,
        payload_json: "{}".to_owned(),
        created_at: NOW.to_owned(),
    })
    .await
    .unwrap();

    // A performance sample of the writing session.
    sqlx::query(
        "INSERT INTO perf_samples (session_id, metric, value_ms, profile_tag, created_at) \
         VALUES (?, 'e2e_ms', 1200.0, 'dev', ?)",
    )
    .bind(writing.id)
    .bind(NOW)
    .execute(db.writer())
    .await
    .unwrap();

    (db, writing.id, reading.id)
}

/// `SELECT count(*)` with no parameters.
async fn count(db: &Database, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(db.readers())
        .await
        .unwrap()
}

/// `SELECT count(*)` with one bound id.
async fn count_with(db: &Database, sql: &str, id: i64) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(db.readers())
        .await
        .unwrap()
}

#[tokio::test]
async fn deleting_a_session_removes_its_text_and_keeps_the_scores() {
    let dir = TempDir::new();
    let (db, writing_id, reading_id) = seeded(&dir).await;

    // The application deletes recording files before the session row; the
    // listing is what it works from.
    let clips = db.audio_clips_for_session(writing_id).await.unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].path, "/tmp/x.wav");

    db.delete_session(writing_id).await.unwrap();

    // Text of the deleted session is gone everywhere.
    assert_eq!(
        count_with(
            &db,
            "SELECT count(*) FROM turns WHERE session_id = ?",
            writing_id
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM turn_analysis WHERE turn_id NOT IN (SELECT id FROM turns)"
        )
        .await,
        0
    );
    assert_eq!(count(&db, "SELECT count(*) FROM error_events").await, 0);
    assert_eq!(count(&db, "SELECT count(*) FROM audio_clips").await, 0);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM pron_results WHERE turn_id IS NOT NULL"
        )
        .await,
        0
    );

    // The attempt row survives with its score, and loses its session and text.
    let attempt = db.attempt(1).await.unwrap();
    assert_eq!(attempt.session_id, None);
    assert_eq!(attempt.normalized, Some(0.8));
    assert!(
        db.evidence_for_attempt(attempt.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(db.pending_scoring(10).await.unwrap().is_empty());

    // perf_samples stay with session_id NULL.
    assert_eq!(count(&db, "SELECT count(*) FROM perf_samples").await, 1);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM perf_samples WHERE session_id IS NULL"
        )
        .await,
        1
    );

    // The other session is untouched, including its generated content.
    assert_eq!(
        count_with(
            &db,
            "SELECT count(*) FROM turns WHERE session_id = ?",
            reading_id
        )
        .await,
        1
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM generated_content").await,
        1
    );
    assert_eq!(db.sessions_for_profile(1, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_learner_data_removes_everything_that_belongs_to_the_learner() {
    let dir = TempDir::new();
    let (db, _, _) = seeded(&dir).await;

    // The caller deletes recording files first, from this listing.
    assert_eq!(db.audio_clips_for_profile(1).await.unwrap().len(), 1);

    db.delete_learner_data(1).await.unwrap();

    for table in [
        "sessions",
        "turns",
        "turn_analysis",
        "error_events",
        "generated_content",
        "audio_clips",
        "pron_results",
        "assessment_attempts",
        "assessment_evidence",
        "pending_scoring",
    ] {
        let sql = format!("SELECT count(*) FROM {table}");
        assert_eq!(count(&db, &sql).await, 0, "{table} still holds rows");
    }

    // perf_samples keep their numbers with a null session.
    assert_eq!(count(&db, "SELECT count(*) FROM perf_samples").await, 1);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM perf_samples WHERE session_id IS NULL"
        )
        .await,
        1
    );

    // The file is compacted after the delete, as DATA_MODEL section 4 says.
    db.compact().await.unwrap();
}
