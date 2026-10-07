//! The analysis bundle: one transaction writes the analysis row, replaces the
//! turn's error events, and adjusts the per-category tally without double
//! counting a re-analysis.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use common::TempDir;
use storage::{
    Database, InputMode, L1HelpMode, NewErrorEvent, NewProfile, NewSession, NewTurn, SessionKind,
    Severity, TurnAnalysis, TurnRole, UiLanguage,
};

const NOW: &str = "2026-10-07T08:00:00.000Z";
const LATER: &str = "2026-10-07T09:30:00.000Z";

async fn seeded(dir: &TempDir) -> (Database, i64, i64) {
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
            input_mode: InputMode::Voice,
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
    (db, profile.id, turn.id)
}

fn analysis(turn_id: i64) -> TurnAnalysis {
    TurnAnalysis {
        turn_id,
        analysis_json: r#"{"turns":[{"turn_seq":1}]}"#.to_owned(),
        contract_version: "turn_analysis/1".to_owned(),
        ladder_level: 1,
        model: "test-model".to_owned(),
        created_at: NOW.to_owned(),
    }
}

fn event(turn_id: i64, profile_id: i64, category: &str, at: &str) -> NewErrorEvent {
    NewErrorEvent {
        turn_id,
        profile_id,
        category: category.to_owned(),
        quote: "go".to_owned(),
        correction: "went".to_owned(),
        severity: Severity::Major,
        addressed: false,
        created_at: at.to_owned(),
    }
}

#[tokio::test]
async fn a_bundle_round_trips_with_its_stats() {
    let dir = TempDir::new();
    let (db, profile_id, turn_id) = seeded(&dir).await;

    db.save_analysis_bundle(
        &analysis(turn_id),
        &[
            event(turn_id, profile_id, "verb_tense", NOW),
            event(turn_id, profile_id, "preposition", NOW),
        ],
    )
    .await
    .unwrap();

    let stored = db.turn_analysis(turn_id).await.unwrap();
    assert_eq!(stored.contract_version, "turn_analysis/1");
    assert_eq!(stored.ladder_level, 1);

    let events = db.error_events_for_turn(turn_id).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].category, "verb_tense");
    assert_eq!(events[0].severity, Severity::Major);
    assert!(!events[0].addressed);

    let stats = db.error_stats(profile_id).await.unwrap();
    assert_eq!(stats.len(), 2);
    assert!(stats.iter().all(|s| s.count == 1));
    assert!(stats.iter().all(|s| s.last_seen.as_deref() == Some(NOW)));
}

#[tokio::test]
async fn re_analysing_replaces_events_and_corrects_the_tally() {
    let dir = TempDir::new();
    let (db, profile_id, turn_id) = seeded(&dir).await;

    db.save_analysis_bundle(
        &analysis(turn_id),
        &[
            event(turn_id, profile_id, "verb_tense", NOW),
            event(turn_id, profile_id, "preposition", NOW),
        ],
    )
    .await
    .unwrap();

    // The second analysis finds one different error: the tally keeps one
    // preposition (from the first run's view it is gone, from the new one it is
    // present) and gains an article. No double counting anywhere.
    db.save_analysis_bundle(
        &analysis(turn_id),
        &[
            event(turn_id, profile_id, "preposition", LATER),
            event(turn_id, profile_id, "article", LATER),
        ],
    )
    .await
    .unwrap();

    let events = db.error_events_for_turn(turn_id).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events
            .iter()
            .map(|e| e.category.as_str())
            .collect::<Vec<_>>(),
        ["preposition", "article"]
    );

    let stats = db.error_stats(profile_id).await.unwrap();
    let by_category: Vec<(&str, i64)> = stats
        .iter()
        .map(|s| (s.category.as_str(), s.count))
        .collect();
    assert_eq!(by_category, [("article", 1), ("preposition", 1)]);
    // `last_seen` moved forward with the newer event.
    assert_eq!(stats[0].last_seen.as_deref(), Some(LATER));
}

#[tokio::test]
async fn an_empty_re_analysis_removes_the_tally_rows() {
    let dir = TempDir::new();
    let (db, profile_id, turn_id) = seeded(&dir).await;

    db.save_analysis_bundle(
        &analysis(turn_id),
        &[event(turn_id, profile_id, "verb_tense", NOW)],
    )
    .await
    .unwrap();
    assert_eq!(db.error_stats(profile_id).await.unwrap().len(), 1);

    // The corrected analysis finds no errors: events and tally both go.
    db.save_analysis_bundle(&analysis(turn_id), &[])
        .await
        .unwrap();
    assert!(db.error_events_for_turn(turn_id).await.unwrap().is_empty());
    assert!(db.error_stats(profile_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn stats_of_a_deleted_session_survive_and_the_events_do_not() {
    let dir = TempDir::new();
    let (db, profile_id, turn_id) = seeded(&dir).await;

    db.save_analysis_bundle(
        &analysis(turn_id),
        &[event(turn_id, profile_id, "verb_tense", NOW)],
    )
    .await
    .unwrap();

    let session_id: i64 = sqlx::query_scalar("SELECT session_id FROM turns WHERE id = ?")
        .bind(turn_id)
        .fetch_one(db.readers())
        .await
        .unwrap();
    db.delete_session(session_id).await.unwrap();

    // The turn and its events are gone; the numbers stay.
    assert!(db.error_events_for_turn(turn_id).await.unwrap().is_empty());
    let stats = db.error_stats(profile_id).await.unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].count, 1);
}
