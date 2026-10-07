//! Opening, migrating and refusing files that are not ours.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use common::{TempDir, backups};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::path::Path;
use storage::{Database, StorageError};

/// Opens a raw pool on a file, for arranging states that the crate itself
/// refuses to create (foreign files, newer files, edited migrations).
async fn raw_pool(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    SqlitePool::connect_with(options).await.unwrap()
}

#[tokio::test]
async fn a_fresh_file_gets_the_full_schema() {
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(db.writer())
    .await
    .unwrap();

    // The 24 tables of migration 0001 plus the migration bookkeeping table.
    assert_eq!(tables.len(), 25, "unexpected table set: {tables:?}");
    for expected in [
        "app_settings",
        "assessment_attempts",
        "assessment_evidence",
        "audio_clips",
        "curriculum_versions",
        "error_events",
        "error_stats",
        "generated_content",
        "llm_calls",
        "models_installed",
        "objective_mastery",
        "objectives",
        "pending_scoring",
        "perf_samples",
        "profiles",
        "pron_results",
        "provider_profiles",
        "review_schedule",
        "sessions",
        "skill_estimates",
        "turn_analysis",
        "turns",
        "unit_progress",
        "units",
    ] {
        assert!(tables.iter().any(|t| t == expected), "missing {expected}");
    }
}

#[tokio::test]
async fn the_connection_pragmas_are_on() {
    let dir = TempDir::new();
    let db = Database::open(dir.db_path()).await.unwrap();

    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(db.writer())
        .await
        .unwrap();
    assert_eq!(journal.to_lowercase(), "wal");

    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(db.writer())
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);

    // Readers must see the same settings: they share the file.
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(db.readers())
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);
}

#[tokio::test]
async fn reopening_applies_nothing_and_makes_no_backup() {
    let dir = TempDir::new();

    let db = Database::open(dir.db_path()).await.unwrap();
    db.close().await;

    let db = Database::open(dir.db_path()).await.unwrap();
    db.close().await;

    assert!(
        backups(dir.path()).is_empty(),
        "unexpected backups: {:?}",
        backups(dir.path())
    );
}

#[tokio::test]
async fn a_foreign_sqlite_file_is_refused() {
    let dir = TempDir::new();
    let path = dir.path().join("someone-elses.sqlite");

    let pool = raw_pool(&path).await;
    sqlx::query("CREATE TABLE not_ours (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let error = Database::open(&path).await.unwrap_err();
    assert!(
        matches!(error, StorageError::ForeignDatabase { .. }),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn a_file_from_a_newer_build_is_refused() {
    let dir = TempDir::new();

    let db = Database::open(dir.db_path()).await.unwrap();
    db.close().await;

    let pool = raw_pool(&dir.db_path()).await;
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (99, 'from the future', TRUE, X'00', 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let error = Database::open(dir.db_path()).await.unwrap_err();
    assert!(
        matches!(error, StorageError::SchemaTooNew { found: 99, .. }),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn an_edited_migration_is_refused() {
    let dir = TempDir::new();

    let db = Database::open(dir.db_path()).await.unwrap();
    db.close().await;

    let pool = raw_pool(&dir.db_path()).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = X'AB' WHERE version = 1")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let error = Database::open(dir.db_path()).await.unwrap_err();
    assert!(
        matches!(error, StorageError::MigrationMismatch { version: 1 }),
        "unexpected error: {error:?}"
    );
}
