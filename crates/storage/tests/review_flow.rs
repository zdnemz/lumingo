//! S4-09 end to end: the `assessment-engine` scheduler drives the stored
//! `review_schedule` rows and the mastery average drives `objective_mastery`.
//! The example unit's `review_items` are enrolled, reviewed across days, and
//! read back; the day numbers and the stored dates agree.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use assessment_engine::review::{
    ReviewItem as Scheduled, date_from_day, day_from_date, due, update_mastery,
};
use common::TempDir;
use curriculum::{ReviewKind as UnitReviewKind, UnitLoader};
use storage::{
    Database, L1HelpMode, NewProfile, NewReviewItem, ReviewKind, ReviewUpdate, UiLanguage,
};

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

/// The day the test's "today" stands for.
const TODAY: &str = "2026-10-07";

fn storage_kind(kind: UnitReviewKind) -> ReviewKind {
    match kind {
        UnitReviewKind::Vocab => ReviewKind::Vocab,
        UnitReviewKind::Grammar => ReviewKind::Grammar,
        UnitReviewKind::Pron => ReviewKind::Pron,
    }
}

#[tokio::test]
async fn the_example_units_review_items_enrol_review_and_come_back_due() {
    let unit = UnitLoader::new().load_str(EXAMPLE).unwrap();
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: "2026-10-07T08:00:00.000Z".to_owned(),
        })
        .await
        .unwrap();

    let today = day_from_date(TODAY).unwrap();
    // Enrol every review item of the unit, due today, with the schedule key
    // `<unit id>/<item id>`.
    for entry in &unit.review_items {
        db.enroll_review_item(&NewReviewItem {
            profile_id: profile.id,
            kind: storage_kind(entry.kind),
            item_ref: format!("{}/{}", unit.id, entry.target),
            due_at: TODAY.to_owned(),
        })
        .await
        .unwrap();
    }
    assert_eq!(unit.review_items.len(), 6);
    let due_today = db.due_review_items(profile.id, TODAY).await.unwrap();
    assert_eq!(due_today.len(), 6, "everything is due on enrolment day");

    // Review the first item: one good review moves it to tomorrow (the first
    // interval of the scheduler is one day).
    let mut scheduled = Scheduled::new(today);
    scheduled.review(1.0, today);
    assert_eq!(scheduled.interval_days, 1);
    let stored = &due_today[0];
    db.record_review(
        stored.id,
        &ReviewUpdate {
            due_at: date_from_day(scheduled.due_day),
            interval_days: f64::from(scheduled.interval_days),
            ease: scheduled.ease,
            reps: i64::from(scheduled.repetitions),
            lapses: i64::from(scheduled.lapses),
            last_reviewed_at: TODAY.to_owned(),
        },
    )
    .await
    .unwrap();

    // The reviewed item left today's due list; the other five still wait.
    // (Tomorrow's list holds all six: the five are overdue, the reviewed one is
    // due that day — "due on or before" includes what was missed.)
    let due_today = db.due_review_items(profile.id, TODAY).await.unwrap();
    assert_eq!(due_today.len(), 5);
    assert!(!due_today.iter().any(|i| i.item_ref == stored.item_ref));
    assert!(due_today.iter().any(|i| i.item_ref == "a1-u01/g-be-i-am"));

    let tomorrow = date_from_day(today + 1);
    let due_tomorrow = db.due_review_items(profile.id, &tomorrow).await.unwrap();
    assert_eq!(due_tomorrow.len(), 6);
    assert!(due_tomorrow.iter().any(|i| i.item_ref == stored.item_ref));

    // The whole schedule in the engine's day terms agrees with the stored dates.
    let rows = db.review_items(profile.id).await.unwrap();
    let engine_rows: Vec<Scheduled> = rows
        .iter()
        .map(|row| {
            let mut item = Scheduled::new(day_from_date(&row.due_at).unwrap());
            item.ease = row.ease;
            item.interval_days = row.interval_days.round() as u32;
            item.repetitions = row.reps.max(0) as u32;
            item.lapses = row.lapses.max(0) as u32;
            item
        })
        .collect();
    let due_indexes = due(&engine_rows, today);
    assert_eq!(
        due_indexes.len(),
        5,
        "the reviewed item left the engine's due list too"
    );
    let reviewed = rows.iter().find(|r| r.id == stored.id).unwrap();
    assert_eq!(reviewed.last_reviewed_at.as_deref(), Some(TODAY));
    assert_eq!(reviewed.reps, 1);

    // A lapse brings the item back to one day and counts the miss.
    let mut scheduled = Scheduled::new(today + 1);
    scheduled.review(0.2, today + 1);
    assert_eq!((scheduled.repetitions, scheduled.lapses), (0, 1));
    assert_eq!(scheduled.interval_days, 1);
}

#[tokio::test]
async fn mastery_stored_through_the_moving_average_matches_the_engine() {
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();
    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: "2026-10-07T08:00:00.000Z".to_owned(),
        })
        .await
        .unwrap();

    // Two results on one objective: the caller keeps the running average and
    // stores it; storage only counts attempts and dates.
    let mut mastery: Option<f64> = None;
    for (day, score) in [("2026-10-07", 0.5), ("2026-10-08", 1.0)] {
        mastery = Some(update_mastery(mastery, score));
        db.upsert_objective_mastery(profile.id, "a1-u01/o1-greet", mastery.unwrap(), day)
            .await
            .unwrap();
    }
    let stored = db
        .objective_mastery(profile.id, "a1-u01/o1-greet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.attempts, 2);
    assert!((stored.mastery - 0.65).abs() < 1e-9);
    assert_eq!(stored.last_attempt_at.as_deref(), Some("2026-10-08"));
}
