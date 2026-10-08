#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The two additions the scorers need: turning the counting flag off once a
//! late score arrives, and reading the pending queue by kind.

mod common;

use common::{make_profile, new_attempt, temp_db, ts};
use serde_json::json;
use storage::{AttemptOrigin, StorageError};

#[tokio::test]
async fn the_counting_flag_can_be_turned_off_later_and_on_only_for_authored_work() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let authored = db
        .attempts()
        .insert(&new_attempt(profile.id, "resp-flag", "task"))
        .await
        .expect("authored");
    db.attempts()
        .set_counts_toward_estimate(authored.id, false)
        .await
        .expect("turn off");
    let after = db.attempts().get(authored.id).await.unwrap().unwrap();
    assert!(!after.counts_toward_estimate);
    db.attempts()
        .set_counts_toward_estimate(authored.id, true)
        .await
        .expect("turn back on");
    let again = db.attempts().get(authored.id).await.unwrap().unwrap();
    assert!(again.counts_toward_estimate);

    let free = db
        .attempts()
        .insert(&storage::NewAttempt {
            origin: AttemptOrigin::FreeMode,
            counts_toward_estimate: false,
            ..new_attempt(profile.id, "resp-free-flag", "task")
        })
        .await
        .expect("free mode");
    assert!(matches!(
        db.attempts()
            .set_counts_toward_estimate(free.id, true)
            .await,
        Err(StorageError::Rule(_))
    ));
    assert!(matches!(
        db.attempts().set_counts_toward_estimate(9999, true).await,
        Err(StorageError::NotFound { .. })
    ));
    assert!(matches!(
        db.attempts().set_counts_toward_estimate(9999, false).await,
        Err(StorageError::NotFound { .. })
    ));
}

#[tokio::test]
async fn the_queue_can_be_read_by_kind_so_one_service_cannot_hide_another() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    for (index, kind) in ["writing_draft", "writing_draft", "rubric_response"]
        .into_iter()
        .enumerate()
    {
        let attempt = db
            .attempts()
            .insert(&new_attempt(profile.id, &format!("resp-q{index}"), "task"))
            .await
            .expect("insert");
        db.pending_scoring()
            .enqueue(
                attempt.id,
                &json!({ "kind": kind, "n": index }),
                &ts(200 + index as i64),
            )
            .await
            .expect("enqueue");
    }
    let drafts = db
        .pending_scoring()
        .oldest_of_kind("writing_draft", 10)
        .await
        .expect("drafts");
    assert_eq!(drafts.len(), 2);
    // The limit applies after the filter: the rubric entry is found behind two older drafts.
    let responses = db
        .pending_scoring()
        .oldest_of_kind("rubric_response", 1)
        .await
        .expect("responses");
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].payload["n"], 2);
    assert!(
        db.pending_scoring()
            .oldest_of_kind("nothing", 10)
            .await
            .expect("none")
            .is_empty()
    );
}
