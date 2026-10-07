//! Review schedule and objective mastery persistence: enrolment, due lists,
//! scheduler updates written back, and the mastery moving average's storage.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use common::TempDir;
use storage::{
    Database, L1HelpMode, NewProfile, NewReviewItem, ReviewKind, ReviewUpdate, UiLanguage,
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

fn item(profile_id: i64, kind: ReviewKind, reference: &str, due_at: &str) -> NewReviewItem {
    NewReviewItem {
        profile_id,
        kind,
        item_ref: reference.to_owned(),
        due_at: due_at.to_owned(),
    }
}

#[tokio::test]
async fn enrolment_round_trips_and_re_enrolment_keeps_the_history() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;

    let stored = db
        .enroll_review_item(&item(
            profile_id,
            ReviewKind::Vocab,
            "a1-u01/v-hello",
            "2026-10-07",
        ))
        .await
        .unwrap();
    assert_eq!(stored.kind, ReviewKind::Vocab);
    assert_eq!(stored.item_ref, "a1-u01/v-hello");
    assert_eq!(stored.due_at, "2026-10-07");
    // The schema defaults: a new item starts at ease 2.5 with no history.
    assert_eq!((stored.interval_days, stored.ease), (0.0, 2.5));
    assert_eq!((stored.reps, stored.lapses), (0, 0));
    assert_eq!(stored.last_reviewed_at, None);

    // One review moves the item along, then a re-enrolment moves only the due date.
    db.record_review(
        stored.id,
        &ReviewUpdate {
            due_at: "2026-10-13".to_owned(),
            interval_days: 6.0,
            ease: 2.6,
            reps: 2,
            lapses: 0,
            last_reviewed_at: "2026-10-07".to_owned(),
        },
    )
    .await
    .unwrap();
    let re_enrolled = db
        .enroll_review_item(&item(
            profile_id,
            ReviewKind::Vocab,
            "a1-u01/v-hello",
            "2026-10-20",
        ))
        .await
        .unwrap();
    assert_eq!(re_enrolled.due_at, "2026-10-20");
    assert_eq!(re_enrolled.ease, 2.6, "history survives");
    assert_eq!(re_enrolled.reps, 2);
    assert_eq!(re_enrolled.last_reviewed_at.as_deref(), Some("2026-10-07"));

    let all = db.review_items(profile_id).await.unwrap();
    assert_eq!(all.len(), 1, "one row per (profile, kind, ref)");
}

#[tokio::test]
async fn the_due_list_is_ordered_and_narrowed_by_date() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;

    for (kind, reference, due) in [
        (ReviewKind::Vocab, "a1-u01/v-hello", "2026-10-10"),
        (ReviewKind::Grammar, "a1-u01/g-be-i-am", "2026-10-07"),
        (ReviewKind::Pron, "a1-u01/p-th-voiceless", "2026-10-05"),
    ] {
        db.enroll_review_item(&item(profile_id, kind, reference, due))
            .await
            .unwrap();
    }

    let due = db.due_review_items(profile_id, "2026-10-07").await.unwrap();
    assert_eq!(
        due.iter().map(|i| i.item_ref.as_str()).collect::<Vec<_>>(),
        ["a1-u01/p-th-voiceless", "a1-u01/g-be-i-am"],
        "most overdue first"
    );
    let due = db.due_review_items(profile_id, "2026-10-09").await.unwrap();
    assert_eq!(due.len(), 2);
    let due = db.due_review_items(profile_id, "2026-10-10").await.unwrap();
    assert_eq!(due.len(), 3);
    let due = db.due_review_items(profile_id, "2026-10-04").await.unwrap();
    assert!(due.is_empty());
}

#[tokio::test]
async fn recording_a_review_of_a_missing_row_is_not_found() {
    let dir = TempDir::new();
    let (db, _) = db_with_profile(&dir).await;
    let error = db
        .record_review(
            999,
            &ReviewUpdate {
                due_at: "2026-10-08".to_owned(),
                interval_days: 1.0,
                ease: 2.5,
                reps: 1,
                lapses: 0,
                last_reviewed_at: "2026-10-07".to_owned(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, storage::StorageError::NotFound { .. }));
}

#[tokio::test]
async fn objective_mastery_accumulates_attempts_and_reads_back() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;

    db.upsert_objective_mastery(profile_id, "a1-u01/o1-greet", 0.5, "2026-10-07")
        .await
        .unwrap();
    db.upsert_objective_mastery(profile_id, "a1-u01/o1-greet", 0.65, "2026-10-08")
        .await
        .unwrap();
    db.upsert_objective_mastery(profile_id, "a1-u01/o2-introduce", 0.9, "2026-10-08")
        .await
        .unwrap();

    let one = db
        .objective_mastery(profile_id, "a1-u01/o1-greet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(one.mastery, 0.65);
    assert_eq!(one.attempts, 2, "each upsert counts one attempt");
    assert_eq!(one.last_attempt_at.as_deref(), Some("2026-10-08"));

    let all = db.objective_masteries(profile_id).await.unwrap();
    assert_eq!(
        all.iter()
            .map(|m| m.objective_id.as_str())
            .collect::<Vec<_>>(),
        ["a1-u01/o2-introduce", "a1-u01/o1-greet"],
        "strongest first"
    );
    assert!(
        db.objective_mastery(profile_id, "a1-u01/nope")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn deleting_learner_data_removes_schedule_and_mastery() {
    let dir = TempDir::new();
    let (db, profile_id) = db_with_profile(&dir).await;
    db.enroll_review_item(&item(
        profile_id,
        ReviewKind::Vocab,
        "a1-u01/v-hello",
        "2026-10-07",
    ))
    .await
    .unwrap();
    db.upsert_objective_mastery(profile_id, "a1-u01/o1-greet", 0.5, "2026-10-07")
        .await
        .unwrap();

    db.delete_learner_data(profile_id).await.unwrap();

    assert!(db.review_items(profile_id).await.unwrap().is_empty());
    assert!(db.objective_masteries(profile_id).await.unwrap().is_empty());
}
