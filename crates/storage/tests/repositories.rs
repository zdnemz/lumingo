//! Repository round trips and the constraint refusals that must come back as
//! typed errors, not raw sqlx errors.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use common::TempDir;
use storage::{
    AttemptOrigin, AttemptStatus, Database, EvidenceKind, InputMode, L1HelpMode, Level, NewAttempt,
    NewEvidence, NewPendingScoring, NewProfile, NewSession, NewTurn, Scorer, SessionKind,
    SessionMode, SessionStatus, Skill, StorageError, TurnAnalysis, TurnRole, UiLanguage,
};

const NOW: &str = "2026-10-07T08:00:00.000Z";

async fn db_with_profile(dir: &TempDir) -> (Database, i64) {
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
    (db, profile.id)
}

fn new_session(profile_id: i64) -> NewSession {
    NewSession {
        profile_id,
        kind: SessionKind::Lesson,
        // No unit row yet: the curriculum index arrives with S4-02, and
        // sessions.unit_id has a foreign key to units (id).
        unit_id: None,
        activity_id: Some("act-1".to_owned()),
        mode: Some(SessionMode::Fluency),
        provider_profile_id: None,
        app_version: "0.0.0".to_owned(),
        started_at: NOW.to_owned(),
    }
}

fn new_attempt(profile_id: i64, session_id: Option<i64>, response_id: &str) -> NewAttempt {
    NewAttempt {
        profile_id,
        session_id,
        unit_id: Some("a1-u01".to_owned()),
        activity_id: "act-1".to_owned(),
        activity_type: "gap_fill".to_owned(),
        response_id: response_id.to_owned(),
        origin: AttemptOrigin::Authored,
        level: Level::A1,
        skill: Skill::Reading,
        dimension: "accuracy".to_owned(),
        scorer: Scorer::Deterministic,
        scorer_version: "det/1".to_owned(),
        raw_score: Some(1.0),
        max_score: Some(1.0),
        normalized: Some(1.0),
        confidence: Some(1.0),
        status: AttemptStatus::Scored,
        counts_toward_estimate: true,
        created_at: NOW.to_owned(),
    }
}

#[tokio::test]
async fn profile_round_trip_and_delete() {
    let dir = TempDir::new();
    let (db, id) = db_with_profile(&dir).await;

    let profile = db.profile(id).await.unwrap();
    assert_eq!(profile.display_name, "Learner");
    assert_eq!(profile.ui_language, UiLanguage::Id);
    assert_eq!(profile.l1_help_mode, L1HelpMode::Auto);
    assert_eq!(db.profiles().await.unwrap().len(), 1);

    db.delete_learner_data(id).await.unwrap();
    assert!(matches!(
        db.profile(id).await.unwrap_err(),
        StorageError::NotFound { .. }
    ));
}

#[tokio::test]
async fn session_round_trip_lifecycle_and_ordering() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;

    let session = db.create_session(new_session(profile_id)).await.unwrap();
    assert_eq!(session.status, SessionStatus::Active);
    assert_eq!(session.mode, Some(SessionMode::Fluency));
    assert_eq!(session.kind, SessionKind::Lesson);

    db.end_session(session.id, SessionStatus::Completed, NOW)
        .await
        .unwrap();
    let session = db.session(session.id).await.unwrap();
    assert_eq!(session.status, SessionStatus::Completed);
    assert_eq!(session.ended_at.as_deref(), Some(NOW));

    // An active status is not a way to end a session.
    let error = db
        .end_session(session.id, SessionStatus::Active, NOW)
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Invalid { .. }));

    db.set_session_summary(session.id, r#"{"turns":3}"#)
        .await
        .unwrap();
    assert_eq!(
        db.session(session.id)
            .await
            .unwrap()
            .summary_json
            .as_deref(),
        Some(r#"{"turns":3}"#)
    );

    // A second session, started earlier, must come second in the listing.
    let mut second = new_session(profile_id);
    second.started_at = "2026-10-06T08:00:00.000Z".to_owned();
    db.create_session(second).await.unwrap();
    let listed = db.sessions_for_profile(profile_id, 10).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, session.id, "newest first");

    // An invalid summary is refused by the schema.
    let error = db
        .set_session_summary(session.id, "not json")
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Invalid { .. }));
}

#[tokio::test]
async fn turns_keep_the_original_transcript_and_refuse_duplicate_sequences() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;
    let session = db.create_session(new_session(profile_id)).await.unwrap();

    let turn = db
        .add_turn(NewTurn {
            session_id: session.id,
            seq: 1,
            role: TurnRole::Learner,
            input_mode: InputMode::Voice,
            text: "I go to school yesterday.".to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: Some(1800),
            pause_ms: Some(200),
            word_count: Some(5),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    assert_eq!(db.next_turn_seq(session.id).await.unwrap(), 2);

    // The learner corrects the transcript: the STT text is preserved once.
    db.edit_turn_text(turn.id, "I went to school yesterday.")
        .await
        .unwrap();
    db.edit_turn_text(turn.id, "I went to school yesterday!")
        .await
        .unwrap();
    let turn = db.turn(turn.id).await.unwrap();
    assert_eq!(turn.text, "I went to school yesterday!");
    assert_eq!(turn.stt_text.as_deref(), Some("I go to school yesterday."));
    assert!(turn.edited_by_learner);

    // The same sequence number again is a conflict.
    let error = db
        .add_turn(NewTurn {
            session_id: session.id,
            seq: 1,
            role: TurnRole::Tutor,
            input_mode: InputMode::NoInput,
            text: "Hello!".to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(1),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        StorageError::Conflict { table: "turns", .. }
    ));

    assert_eq!(db.turns_for_session(session.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn analysis_is_replaced_not_duplicated() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;
    let session = db.create_session(new_session(profile_id)).await.unwrap();
    let turn = db
        .add_turn(NewTurn {
            session_id: session.id,
            seq: 1,
            role: TurnRole::Learner,
            input_mode: InputMode::Text,
            text: "Hello.".to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(1),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();

    let mut analysis = TurnAnalysis {
        turn_id: turn.id,
        analysis_json: r#"{"v":1}"#.to_owned(),
        contract_version: "1".to_owned(),
        ladder_level: 1,
        model: "m1".to_owned(),
        created_at: NOW.to_owned(),
    };
    db.save_turn_analysis(&analysis).await.unwrap();
    analysis.analysis_json = r#"{"v":2}"#.to_owned();
    analysis.ladder_level = 2;
    db.save_turn_analysis(&analysis).await.unwrap();

    let stored = db.turn_analysis(turn.id).await.unwrap();
    assert_eq!(stored.analysis_json, r#"{"v":2}"#);
    assert_eq!(stored.ladder_level, 2);

    // A missing analysis is a typed not-found.
    assert!(matches!(
        db.turn_analysis(999).await.unwrap_err(),
        StorageError::NotFound { .. }
    ));
}

#[tokio::test]
async fn attempts_group_by_response_and_keep_their_evidence() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;
    let session = db.create_session(new_session(profile_id)).await.unwrap();

    // One response scored on two dimensions: two rows, one response id.
    let mut first = new_attempt(profile_id, Some(session.id), "r1");
    first.dimension = "accuracy".to_owned();
    let mut second = new_attempt(profile_id, Some(session.id), "r1");
    second.dimension = "completeness".to_owned();
    second.scorer = Scorer::RubricLlm;
    second.confidence = Some(0.7);

    let a = db.add_attempt(first).await.unwrap();
    let b = db.add_attempt(second).await.unwrap();
    assert_eq!(a.response_id, b.response_id);

    let rows = db.attempts_for_response("r1").await.unwrap();
    assert_eq!(rows.len(), 2);

    // The session's rows read back oldest first, both dimensions included.
    let by_session = db.attempts_for_session(session.id).await.unwrap();
    assert_eq!(by_session.len(), 2);
    assert_eq!(by_session[0].id, a.id);
    assert_eq!(by_session[1].id, b.id);
    assert_eq!(by_session[0].dimension, "accuracy");
    assert!(db.attempts_for_session(999).await.unwrap().is_empty());

    let by_skill = db
        .attempts_for_skill(profile_id, Skill::Reading)
        .await
        .unwrap();
    assert_eq!(by_skill.len(), 2);
    assert_eq!(
        db.attempts_for_skill(profile_id, Skill::Speaking)
            .await
            .unwrap()
            .len(),
        0
    );

    // The supporting dimensions round-trip too (ASSESSMENT_SPEC section 2).
    for skill in [Skill::Grammar, Skill::Vocabulary, Skill::Pronunciation] {
        let mut supporting = new_attempt(profile_id, Some(session.id), "r2");
        supporting.skill = skill;
        let stored = db.add_attempt(supporting).await.unwrap();
        assert_eq!(stored.skill, skill);
        assert_eq!(
            db.attempts_for_skill(profile_id, skill)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    let evidence = db
        .add_evidence(NewEvidence {
            attempt_id: a.id,
            kind: EvidenceKind::Quote,
            content: Some("I go to school yesterday.".to_owned()),
            data_json: None,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let metric = db
        .add_evidence(NewEvidence {
            attempt_id: a.id,
            kind: EvidenceKind::Metric,
            content: None,
            data_json: Some(r#"{"wer":0.1}"#.to_owned()),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(evidence.kind, EvidenceKind::Quote);
    assert_eq!(metric.content, None);
    assert_eq!(db.evidence_for_attempt(a.id).await.unwrap().len(), 2);

    // Bad metric JSON is refused by the schema.
    let error = db
        .add_evidence(NewEvidence {
            attempt_id: a.id,
            kind: EvidenceKind::Metric,
            content: None,
            data_json: Some("not json".to_owned()),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Invalid { .. }));
}

#[tokio::test]
async fn the_pending_queue_round_trips_and_refuses_duplicates() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;
    let session = db.create_session(new_session(profile_id)).await.unwrap();

    let mut pending = new_attempt(profile_id, Some(session.id), "r9");
    pending.status = AttemptStatus::PendingLlm;
    pending.scorer = Scorer::RubricLlm;
    pending.normalized = None;
    pending.confidence = None;
    let attempt = db.add_attempt(pending).await.unwrap();

    let queued = db
        .enqueue_pending_scoring(NewPendingScoring {
            attempt_id: attempt.id,
            payload_json: r#"{"text":"draft"}"#.to_owned(),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(queued.tries, 0);

    // The same attempt cannot be queued twice.
    let error = db
        .enqueue_pending_scoring(NewPendingScoring {
            attempt_id: attempt.id,
            payload_json: "{}".to_owned(),
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::Conflict { .. }));

    db.bump_pending_tries(queued.id).await.unwrap();
    assert_eq!(db.pending_scoring(10).await.unwrap()[0].tries, 1);

    db.remove_pending_scoring(queued.id).await.unwrap();
    assert!(db.pending_scoring(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_level_the_schema_does_not_allow_is_refused_at_write_time() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;

    // The CHECK constraint is the first line of defence: a level this build
    // does not know cannot even be written. The read path maps an unknown
    // stored value to a typed error as well (unit-tested in rows.rs), so a
    // file that predates a constraint cannot leak an unknown value.
    let result = sqlx::query(
        "INSERT INTO assessment_attempts \
         (profile_id, activity_id, activity_type, response_id, origin, level, skill, dimension, \
          scorer, scorer_version, status, created_at) \
         VALUES (?, 'act', 'gap_fill', 'r1', 'authored', 'C3', 'reading', 'd', 'deterministic', \
                 'det/1', 'scored', ?)",
    )
    .bind(profile_id)
    .bind(NOW)
    .execute(db.writer())
    .await;

    let error = result.unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("CHECK") || message.contains("constraint"),
        "unexpected error: {message}"
    );
}

#[tokio::test]
async fn a_writer_transaction_does_not_block_readers() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;

    // Hold an open write transaction, then read from the reader pool: in WAL
    // mode the reader must not wait for the writer to commit.
    let mut conn = db.writer().acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sessions (profile_id, kind, app_version, started_at) \
         VALUES (?, 'lesson', '0.0.0', ?)",
    )
    .bind(profile_id)
    .bind(NOW)
    .execute(&mut *conn)
    .await
    .unwrap();

    let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(db.readers())
        .await
        .unwrap();
    assert_eq!(seen, 0, "uncommitted row must not be visible");

    sqlx::query("COMMIT").execute(&mut *conn).await.unwrap();
    drop(conn);

    let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(db.readers())
        .await
        .unwrap();
    assert_eq!(seen, 1, "committed row must be visible");
}
