#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The delete behaviour of `docs/DATA_MODEL.md` section 4, checked against the
//! real schema: deleting a session removes its text everywhere and keeps the
//! numeric scores; deleting the profile removes everything.

mod common;

use common::{count, make_profile, make_session, new_attempt, raw_conn, temp_db, ts};
use serde_json::json;
use storage::{
    Database, EstimateLevel, EstimateStatus, EvidenceKind, GeneratedKind, InputMode, LocalDate,
    NewErrorEvent, NewEvidence, NewGeneratedContent, NewPerfSample, NewSkillEstimate, NewTurn,
    NewXp, Session, SessionKind, Severity, StorageError, TurnAnalysis, TurnRole, XpSourceKind,
};

const MARKER: &str = "zqxj-marker-7731";

/// Everything one session can own, so a delete has something to remove from
/// every table the data model lists.
struct Seeded {
    session: Session,
    turn_id: i64,
    attempt_id: i64,
}

async fn seed(db: &Database, profile_id: i64, kind: SessionKind, marker: &str) -> Seeded {
    let session = make_session(db, profile_id, kind).await;
    let turn = db
        .turns()
        .append(&NewTurn {
            session_id: session.id,
            role: TurnRole::Learner,
            input_mode: InputMode::Voice,
            text: format!("I like {marker} very much"),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: Some(1_200),
            pause_ms: Some(300),
            word_count: Some(5),
            created_at: ts(20),
        })
        .await
        .expect("turn");
    db.analysis()
        .store(
            &TurnAnalysis {
                turn_id: turn.id,
                analysis: json!({ "quote": marker }),
                contract_version: "t2/1".to_owned(),
                ladder_level: 1,
                model: "test-model".to_owned(),
                created_at: ts(21),
            },
            &[NewErrorEvent {
                turn_id: turn.id,
                profile_id,
                category: "word_choice".to_owned(),
                quote: marker.to_owned(),
                correction: "like it".to_owned(),
                severity: Severity::Minor,
                created_at: ts(21),
            }],
        )
        .await
        .expect("analysis");
    db.generated_content()
        .add(&NewGeneratedContent {
            session_id: session.id,
            kind: GeneratedKind::ReadingPassage,
            content: json!({ "text": marker }),
            contract_version: "r1/1".to_owned(),
            model: "test-model".to_owned(),
            created_at: ts(22),
        })
        .await
        .expect("generated content");
    db.audio_clips()
        .add(
            turn.id,
            &format!("audio/{}.wav", session.id),
            1_500,
            &ts(23),
        )
        .await
        .expect("clip");

    let attempt = db
        .attempts()
        .insert(&storage::NewAttempt {
            session_id: Some(session.id),
            ..new_attempt(profile_id, &format!("resp-{}", session.id), "task")
        })
        .await
        .expect("attempt");
    for (kind, content, data) in [
        (EvidenceKind::ResponseText, Some(marker), None),
        (EvidenceKind::Quote, Some(marker), None),
        (EvidenceKind::Metric, None, Some(json!({ "words": 5 }))),
        (EvidenceKind::ScorerReason, Some("clear and on topic"), None),
    ] {
        db.evidence()
            .add(&NewEvidence {
                attempt_id: attempt.id,
                kind,
                content: content.map(str::to_owned),
                data,
                created_at: ts(24),
            })
            .await
            .expect("evidence");
    }
    db.pending_scoring()
        .enqueue(attempt.id, &json!({ "response": marker }), &ts(25))
        .await
        .expect("pending");
    db.diagnostics()
        .record_perf_sample(&NewPerfSample {
            session_id: Some(session.id),
            turn_seq: Some(1),
            metric: "e2e_ms".to_owned(),
            value_ms: 1_450.0,
            profile_tag: "dev".to_owned(),
            created_at: ts(26),
        })
        .await
        .expect("perf sample");

    // Free-speech pronunciation belongs to the turn; a drill result belongs to the
    // attempt. Only the first is learner speech tied to the session.
    let mut conn = raw_conn(db).await;
    for (attempt_ref, turn_ref, mode) in [
        (None, Some(turn.id), "free_speech"),
        (Some(attempt.id), None, "drill"),
    ] {
        sqlx::query(
            "INSERT INTO pron_results (attempt_id, turn_id, mode, word, word_index, phone_expected, \
             phone_heard, gop, flagged, start_ms, end_ms, engine_version) \
             VALUES (?1, ?2, ?3, 'hello', 0, 'h', 'h', 0.9, 0, 0, 100, 'v1')",
        )
        .bind(attempt_ref)
        .bind(turn_ref)
        .bind(mode)
        .execute(&mut conn)
        .await
        .expect("pron result");
    }

    // Game rows that mention the session by label only.
    db.game()
        .award_xp(&NewXp {
            profile_id,
            created_at: ts(27),
            source_kind: XpSourceKind::Writing,
            source_id: format!("session:{}", session.id),
            amount: 15,
            reason: "free.writing".to_owned(),
        })
        .await
        .expect("xp");

    Seeded {
        session,
        turn_id: turn.id,
        attempt_id: attempt.id,
    }
}

async fn estimate(db: &Database, profile_id: i64) -> storage::SkillEstimate {
    db.estimates()
        .insert(&NewSkillEstimate {
            profile_id,
            skill: "writing".to_owned(),
            level: Some(EstimateLevel::A1),
            status: EstimateStatus::Estimated,
            confidence: Some(0.62),
            evidence_count: 9,
            algorithm_version: "est/1".to_owned(),
            detail: None,
            computed_at: ts(30),
        })
        .await
        .expect("estimate")
}

#[tokio::test]
async fn deleting_a_session_removes_its_text_and_keeps_the_numbers() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let doomed = seed(&db, profile.id, SessionKind::Writing, MARKER).await;
    let kept = seed(&db, profile.id, SessionKind::Reading, "other-learner-words").await;
    // An attempt that never had a session, with its own evidence.
    let loose = db
        .attempts()
        .insert(&new_attempt(profile.id, "resp-loose", "task"))
        .await
        .expect("loose attempt");
    db.evidence()
        .add(&NewEvidence {
            attempt_id: loose.id,
            kind: EvidenceKind::ResponseText,
            content: Some("loose text".to_owned()),
            data: None,
            created_at: ts(28),
        })
        .await
        .expect("loose evidence");
    let estimate_before = estimate(&db, profile.id).await;
    let score_before = db
        .attempts()
        .get(doomed.attempt_id)
        .await
        .expect("get")
        .expect("exists");

    // The application removes the recordings first; the database cannot.
    let audio = db
        .sessions()
        .audio_paths(doomed.session.id)
        .await
        .expect("paths");
    assert_eq!(audio, [format!("audio/{}.wav", doomed.session.id)]);
    db.sessions()
        .delete(doomed.session.id)
        .await
        .expect("delete");

    let mut raw = raw_conn(&db).await;
    // Conversation content of the deleted session is gone ...
    assert!(
        db.sessions()
            .get(doomed.session.id)
            .await
            .expect("get")
            .is_none()
    );
    assert!(
        db.turns()
            .list(doomed.session.id)
            .await
            .expect("turns")
            .is_empty()
    );
    assert!(
        db.analysis()
            .get(doomed.turn_id)
            .await
            .expect("analysis")
            .is_none()
    );
    assert!(
        db.analysis()
            .error_events(doomed.turn_id)
            .await
            .expect("events")
            .is_empty()
    );
    assert!(
        db.generated_content()
            .for_session(doomed.session.id)
            .await
            .expect("generated")
            .is_empty()
    );
    assert!(
        db.audio_clips()
            .for_turn(doomed.turn_id)
            .await
            .expect("clips")
            .is_empty()
    );
    // ... and so are the evidence and the pending scoring of its attempts,
    assert!(
        db.evidence()
            .for_attempt(doomed.attempt_id)
            .await
            .expect("evidence")
            .is_empty()
    );
    assert!(
        db.pending_scoring()
            .oldest(10)
            .await
            .expect("pending")
            .iter()
            .all(|p| p.attempt_id != doomed.attempt_id)
    );
    // ... together with the free-speech pronunciation rows of its turns.
    let free_speech: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pron_results WHERE mode = 'free_speech' AND turn_id = ?1",
    )
    .bind(doomed.turn_id)
    .fetch_one(&mut raw)
    .await
    .expect("count");
    assert_eq!(free_speech, 0);

    // The attempt stays with its score; only the session link is cut.
    let score_after = db
        .attempts()
        .get(doomed.attempt_id)
        .await
        .expect("get")
        .expect("attempt survives");
    assert_eq!(score_after.session_id, None);
    assert_eq!(
        storage::Attempt {
            session_id: None,
            ..score_before
        },
        score_after
    );
    // The drill pronunciation row hangs on the attempt, so it stays too.
    let drill: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pron_results WHERE mode = 'drill' AND attempt_id = ?1",
    )
    .bind(doomed.attempt_id)
    .fetch_one(&mut raw)
    .await
    .expect("count");
    assert_eq!(drill, 1);
    // Estimates are untouched.
    assert_eq!(
        db.estimates()
            .latest(profile.id, "writing")
            .await
            .expect("latest"),
        Some(estimate_before)
    );
    // The latency number stays with the session link cut.
    let samples = db
        .diagnostics()
        .perf_samples("e2e_ms", &ts(0))
        .await
        .expect("samples");
    assert_eq!(samples.len(), 2);
    assert!(samples.iter().any(|s| s.session_id.is_none()));

    // Nothing belonging to other sessions or to session-less attempts was touched.
    assert_eq!(
        db.turns().list(kept.session.id).await.expect("turns").len(),
        1
    );
    assert_eq!(
        db.evidence()
            .for_attempt(kept.attempt_id)
            .await
            .expect("evidence")
            .len(),
        4
    );
    assert_eq!(
        db.evidence()
            .for_attempt(loose.id)
            .await
            .expect("evidence")
            .len(),
        1
    );
    assert!(
        db.analysis()
            .get(kept.turn_id)
            .await
            .expect("analysis")
            .is_some()
    );
    assert!(
        db.pending_scoring()
            .oldest(10)
            .await
            .expect("pending")
            .iter()
            .any(|p| p.attempt_id == kept.attempt_id)
    );
    // The game layer never referred to the session by key, so it is untouched.
    assert_eq!(db.game().xp_total(profile.id).await.expect("xp"), 30);
}

#[tokio::test]
async fn after_deleting_a_writing_and_a_reading_session_only_numbers_remain() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let writing = seed(&db, profile.id, SessionKind::Writing, "draft words").await;
    let reading = seed(&db, profile.id, SessionKind::Reading, "passage words").await;
    db.sessions()
        .delete(writing.session.id)
        .await
        .expect("delete writing");
    db.sessions()
        .delete(reading.session.id)
        .await
        .expect("delete reading");

    let mut raw = raw_conn(&db).await;
    for table in [
        "turns",
        "turn_analysis",
        "error_events",
        "generated_content",
        "audio_clips",
        "assessment_evidence",
        "pending_scoring",
        "sessions",
    ] {
        assert_eq!(count(&mut raw, table).await, 0, "{table}");
    }
    let free_speech: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pron_results WHERE mode = 'free_speech'")
            .fetch_one(&mut raw)
            .await
            .expect("count");
    assert_eq!(free_speech, 0);
    assert_eq!(count(&mut raw, "assessment_attempts").await, 2);
    let rows = db
        .attempts()
        .for_skill_since(profile.id, "writing", &ts(0))
        .await
        .expect("attempts");
    assert!(
        rows.iter()
            .all(|a| a.session_id.is_none() && a.normalized == Some(0.75))
    );
    assert_eq!(count(&mut raw, "perf_samples").await, 2);
}

#[tokio::test]
async fn deleting_a_session_that_does_not_exist_is_not_found() {
    let (_dir, db) = temp_db().await;
    assert!(matches!(
        db.sessions().delete(404).await,
        Err(StorageError::NotFound { .. })
    ));
}

/// Both files that can hold database pages: the main file and the write-ahead log.
fn file_bytes(db: &Database) -> Vec<u8> {
    let mut bytes = std::fs::read(db.path()).unwrap_or_default();
    let mut wal = db.path().as_os_str().to_owned();
    wal.push("-wal");
    bytes.extend(std::fs::read(std::path::PathBuf::from(wal)).unwrap_or_default());
    bytes
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[tokio::test]
async fn compact_after_a_delete_leaves_no_trace_of_the_text_in_the_files() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let doomed = seed(&db, profile.id, SessionKind::Writing, MARKER).await;
    // The text really is in the files before the delete, so the check below can fail.
    assert!(contains(&file_bytes(&db), MARKER));

    db.sessions()
        .delete(doomed.session.id)
        .await
        .expect("delete");
    db.compact().await.expect("compact");

    assert!(!contains(&file_bytes(&db), MARKER));
}

#[tokio::test]
async fn deleting_the_profile_removes_everything_including_the_game_layer() {
    let (_dir, db) = temp_db().await;
    let leaving = make_profile(&db).await;
    let staying = db
        .profiles()
        .create(&storage::NewProfile {
            display_name: "Second".to_owned(),
            ui_language: storage::UiLanguage::En,
            l1: "en".to_owned(),
            l1_help_mode: storage::L1HelpMode::Off,
            created_at: ts(1),
        })
        .await
        .expect("second profile");

    for profile_id in [leaving.id, staying.id] {
        seed(&db, profile_id, SessionKind::Writing, "words").await;
        estimate(&db, profile_id).await;
        let game = db.game();
        game.grant_rest_token(profile_id, &ts(40), "welcome")
            .await
            .expect("token");
        let day = |d| LocalDate::from_ymd(2026, 10, d).expect("day");
        game.record_activity(profile_id, day(1), &ts(41))
            .await
            .expect("day 1");
        // Day 3 spends the token on day 2, so a rest day references a token.
        game.record_activity(profile_id, day(3), &ts(42))
            .await
            .expect("day 3");
        game.unlock(profile_id, "accessory.beret", &ts(43))
            .await
            .expect("unlock");
        game.equip_accessory(profile_id, Some("accessory.beret"), &ts(44))
            .await
            .expect("equip");
    }

    let audio = db
        .audio_clips()
        .paths_for_profile(leaving.id)
        .await
        .expect("paths");
    assert_eq!(audio.len(), 1);
    db.profiles()
        .delete(leaving.id)
        .await
        .expect("delete profile");
    db.compact().await.expect("compact");

    assert!(
        db.estimates()
            .latest(leaving.id, "writing")
            .await
            .expect("latest")
            .is_none()
    );
    assert_eq!(db.game().xp_total(leaving.id).await.expect("xp"), 0);
    assert!(
        db.game()
            .streak_days(leaving.id)
            .await
            .expect("days")
            .is_empty()
    );
    assert!(
        db.game()
            .unlocked(leaving.id)
            .await
            .expect("unlocked")
            .is_empty()
    );
    assert!(
        db.game()
            .equipped(leaving.id)
            .await
            .expect("equipped")
            .is_none()
    );
    assert!(
        db.game()
            .available_rest_tokens(leaving.id)
            .await
            .expect("tokens")
            .is_empty()
    );
    assert!(
        db.sessions()
            .list_for_profile(leaving.id, 10)
            .await
            .expect("sessions")
            .is_empty()
    );

    // Exactly the other profile's rows are left: one of each.
    let mut raw = raw_conn(&db).await;
    for table in [
        "sessions",
        "turns",
        "assessment_attempts",
        "skill_estimates",
        "xp_ledger",
        "rest_tokens",
        "unlockables",
        "equipped_cosmetics",
    ] {
        assert_eq!(count(&mut raw, table).await, 1, "{table}");
    }
    assert_eq!(count(&mut raw, "streak_days").await, 3);
    assert_eq!(db.game().xp_total(staying.id).await.expect("xp"), 15);
}
