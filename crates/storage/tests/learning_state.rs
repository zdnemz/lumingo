#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{make_profile, make_session, make_turn, new_attempt, temp_db, ts};
use storage::{
    NewPronResult, NewReviewItem, ObjectiveMastery, PronMode, ReviewKind, SessionKind,
    StorageError, TurnRole, UnitProgress, UnitStatus,
};

#[tokio::test]
async fn unit_progress_and_mastery_are_replaced_in_place() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;

    let mut progress = UnitProgress {
        profile_id: profile.id,
        unit_id: "a1-u01".to_owned(),
        status: UnitStatus::InProgress,
        best_checkpoint: None,
        updated_at: ts(1),
    };
    db.unit_progress().set(&progress).await.expect("set");
    progress.status = UnitStatus::Passed;
    progress.best_checkpoint = Some(0.85);
    progress.updated_at = ts(2);
    db.unit_progress().set(&progress).await.expect("set again");
    assert_eq!(
        db.unit_progress()
            .get(profile.id, "a1-u01")
            .await
            .expect("get"),
        Some(progress.clone())
    );
    assert_eq!(
        db.unit_progress().list(profile.id).await.expect("list"),
        vec![progress]
    );

    let mastery = ObjectiveMastery {
        profile_id: profile.id,
        objective_id: "a1-u01/o1".to_owned(),
        mastery: 0.4,
        attempts: 3,
        last_attempt_at: Some(ts(3)),
    };
    db.mastery().set(&mastery).await.expect("set");
    let raised = ObjectiveMastery {
        mastery: 0.7,
        attempts: 4,
        ..mastery
    };
    db.mastery().set(&raised).await.expect("set again");
    assert_eq!(
        db.mastery()
            .get(profile.id, "a1-u01/o1")
            .await
            .expect("get"),
        Some(raised)
    );
}

#[tokio::test]
async fn error_counts_accumulate_and_only_accept_catalog_codes() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let stats = db.error_stats();
    stats
        .add(profile.id, "agreement.subject-verb", 1, &ts(1))
        .await
        .expect("add");
    stats
        .add(profile.id, "agreement.subject-verb", 2, &ts(5))
        .await
        .expect("add");
    stats
        .add(profile.id, "articles", 1, &ts(2))
        .await
        .expect("add");

    let listed = stats.list(profile.id).await.expect("list");
    let shape: Vec<(&str, i64)> = listed
        .iter()
        .map(|s| (s.category.as_str(), s.count))
        .collect();
    assert_eq!(shape, [("agreement.subject-verb", 3), ("articles", 1)]);
    assert_eq!(listed[0].last_seen, Some(ts(5)));

    assert!(matches!(
        stats.add(profile.id, "She go home", 1, &ts(6)).await,
        Err(StorageError::Rule(_))
    ));
}

#[tokio::test]
async fn the_review_schedule_keeps_one_row_per_item_and_lists_what_is_due() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let item = |reference: &str, due: i64| NewReviewItem {
        profile_id: profile.id,
        item_kind: ReviewKind::Vocab,
        item_ref: reference.to_owned(),
        due_at: ts(due),
        interval_days: 1.0,
        ease: 2.5,
        reps: 0,
        lapses: 0,
        last_reviewed_at: None,
    };
    let first = db
        .review_schedule()
        .put(&item("a1-u01/hello", 100))
        .await
        .expect("put");
    db.review_schedule()
        .put(&item("a1-u01/goodbye", 50))
        .await
        .expect("put");
    db.review_schedule()
        .put(&item("a1-u02/thanks", 500))
        .await
        .expect("put");

    // Reviewing the first item again moves its row instead of adding one.
    let again = db
        .review_schedule()
        .put(&NewReviewItem {
            reps: 1,
            interval_days: 3.0,
            due_at: ts(900),
            last_reviewed_at: Some(ts(101)),
            ..item("a1-u01/hello", 100)
        })
        .await
        .expect("put again");
    assert_eq!(again.id, first.id);

    let due = db
        .review_schedule()
        .due(profile.id, &ts(600), 10)
        .await
        .expect("due");
    let refs: Vec<&str> = due.iter().map(|d| d.item_ref.as_str()).collect();
    assert_eq!(refs, ["a1-u01/goodbye", "a1-u02/thanks"]);
    let stored = db
        .review_schedule()
        .get(profile.id, ReviewKind::Vocab, "a1-u01/hello")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!((stored.reps, stored.due_at), (1, ts(900)));
}

fn pron(
    attempt_id: Option<i64>,
    turn_id: Option<i64>,
    mode: PronMode,
    index: i64,
) -> NewPronResult {
    NewPronResult {
        attempt_id,
        turn_id,
        mode,
        word: "hello".to_owned(),
        word_index: index,
        phone_expected: "h".to_owned(),
        phone_heard: Some("h".to_owned()),
        gop: 0.82,
        flagged: false,
        start_ms: index * 100,
        end_ms: index * 100 + 90,
        engine_version: "pron/1".to_owned(),
    }
}

#[tokio::test]
async fn pronunciation_results_belong_to_an_attempt_or_a_turn() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::Conversation).await;
    let turn = make_turn(&db, session.id, TurnRole::Learner, "hello there").await;
    let attempt = db
        .attempts()
        .insert(&new_attempt(profile.id, "resp-drill", "pronunciation"))
        .await
        .expect("attempt");

    db.pron_results()
        .add_many(&[
            pron(Some(attempt.id), None, PronMode::Drill, 1),
            pron(Some(attempt.id), None, PronMode::Drill, 0),
            pron(None, Some(turn.id), PronMode::FreeSpeech, 0),
        ])
        .await
        .expect("add");
    let drill = db
        .pron_results()
        .for_attempt(attempt.id)
        .await
        .expect("drill");
    assert_eq!(
        drill.iter().map(|r| r.word_index).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(
        db.pron_results()
            .for_turn(turn.id)
            .await
            .expect("free speech")
            .len(),
        1
    );

    // All or nothing: one result tied to nothing refuses the whole batch.
    let refused = db
        .pron_results()
        .add_many(&[
            pron(Some(attempt.id), None, PronMode::Drill, 5),
            pron(None, None, PronMode::Drill, 6),
        ])
        .await;
    assert!(matches!(refused, Err(StorageError::Rule(_))));
    assert_eq!(
        db.pron_results()
            .for_attempt(attempt.id)
            .await
            .expect("drill")
            .len(),
        2
    );

    // Deleting the session removes the free-speech row and keeps the drill rows.
    db.sessions().delete(session.id).await.expect("delete");
    assert!(
        db.pron_results()
            .for_turn(turn.id)
            .await
            .expect("free speech")
            .is_empty()
    );
    assert_eq!(
        db.pron_results()
            .for_attempt(attempt.id)
            .await
            .expect("drill")
            .len(),
        2
    );
}
