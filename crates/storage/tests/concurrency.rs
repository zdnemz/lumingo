#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Two connections on one file must not deadlock under WAL. Every wait below is
//! wrapped in a timeout, so a deadlock fails the test instead of hanging it.

mod common;

use std::time::{Duration, Instant};

use common::{make_profile, make_session, new_turn, raw_conn, temp_db, ts};
use sqlx::Connection;
use storage::{Database, OpenConfig, SessionKind, StorageError, TurnRole};
use tokio::time::{sleep, timeout};

const PATIENCE: Duration = Duration::from_secs(20);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_read_transaction_does_not_block_the_writer_and_keeps_its_snapshot() {
    let (_dir, db) = temp_db().await;
    let mut reader = raw_conn(&db).await;
    let mut snapshot = reader.begin().await.expect("begin read transaction");
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings")
        .fetch_one(&mut *snapshot)
        .await
        .expect("first read takes the snapshot");
    assert_eq!(before, 0);

    // The writer commits while the reader's transaction is still open.
    timeout(PATIENCE, db.settings().set("theme", "dark", &ts(1)))
        .await
        .expect("the writer was blocked by a reader")
        .expect("write");

    let still: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings")
        .fetch_one(&mut *snapshot)
        .await
        .expect("read inside the old snapshot");
    assert_eq!(still, 0, "the open transaction keeps its snapshot");
    snapshot.commit().await.expect("end read transaction");

    assert_eq!(
        db.settings().get("theme").await.expect("get").as_deref(),
        Some("dark")
    );
    let fresh: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings")
        .fetch_one(&mut reader)
        .await
        .expect("fresh read");
    assert_eq!(fresh, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_writer_waits_for_another_writer_instead_of_failing() {
    let (_dir, db) = temp_db().await;
    let mut other = raw_conn(&db).await;
    let mut held = other
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("take the write lock");
    sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES ('held', '1', '2026-01-01T00:00:00.000Z')")
        .execute(&mut *held)
        .await
        .expect("write inside the held transaction");

    let waiting = db.clone();
    let write = tokio::spawn(async move {
        let started = Instant::now();
        waiting
            .settings()
            .set("late", "1", &ts(2))
            .await
            .map(|()| started.elapsed())
    });
    sleep(Duration::from_millis(300)).await;
    assert!(
        !write.is_finished(),
        "the second writer should be waiting on the lock"
    );
    held.commit().await.expect("release the lock");

    let waited = timeout(PATIENCE, write)
        .await
        .expect("deadlock: the waiting writer never finished")
        .expect("join")
        .expect("the waiting writer should succeed once the lock is free");
    assert!(
        waited >= Duration::from_millis(250),
        "it did wait: {waited:?}"
    );
    let keys: Vec<String> = db
        .settings()
        .all()
        .await
        .expect("all")
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(keys, ["held", "late"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_busy_timeout_ends_the_wait_with_an_error_instead_of_hanging() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open_with(
        dir.path().join("lumingo.sqlite"),
        OpenConfig {
            busy_timeout: Duration::from_millis(200),
            read_connections: 2,
        },
    )
    .await
    .expect("open");
    let mut other = raw_conn(&db).await;
    let held = other
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("take the write lock");

    let started = Instant::now();
    let result = timeout(PATIENCE, db.settings().set("k", "v", &ts(1)))
        .await
        .expect("the call hung past its busy timeout");
    assert!(
        matches!(result, Err(StorageError::Database(_))),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    held.rollback().await.expect("release");

    // Once the lock is free the same call works.
    db.settings()
        .set("k", "v", &ts(2))
        .await
        .expect("write after release");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_tasks_reading_and_writing_at_once_all_finish() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::TextChat).await;

    let mut tasks = Vec::new();
    for n in 0..24_i64 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db.turns()
                .append(&new_turn(
                    session.id,
                    TurnRole::Learner,
                    &format!("line {n}"),
                    n,
                ))
                .await
                .expect("append");
            db.settings()
                .set(&format!("key-{n}"), "v", &ts(n))
                .await
                .expect("set");
            let turns = db.turns().list(session.id).await.expect("list");
            assert!(!turns.is_empty());
            db.sessions().get(session.id).await.expect("get");
        }));
    }
    for task in tasks {
        timeout(PATIENCE, task)
            .await
            .expect("deadlock between readers and writers")
            .expect("task");
    }
    assert_eq!(db.turns().list(session.id).await.expect("list").len(), 24);
    assert_eq!(db.settings().all().await.expect("all").len(), 24);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_handles_on_one_file_write_at_the_same_time() {
    let (dir, first) = temp_db().await;
    // A second open of the same file stands in for a second process.
    let second = Database::open(first.path()).await.expect("second open");
    assert_eq!(dir.path(), first.path().parent().expect("parent"));

    let mut tasks = Vec::new();
    for n in 0..20_i64 {
        for (name, db) in [("a", first.clone()), ("b", second.clone())] {
            tasks.push(tokio::spawn(async move {
                db.settings()
                    .set(&format!("{name}-{n}"), "v", &ts(n))
                    .await
                    .expect("write");
            }));
        }
    }
    for task in tasks {
        timeout(PATIENCE, task)
            .await
            .expect("deadlock between two handles")
            .expect("task");
    }
    assert_eq!(first.settings().all().await.expect("all").len(), 40);
    assert_eq!(second.settings().all().await.expect("all").len(), 40);
    second.close().await.expect("close second");
    first.close().await.expect("close first");
}
