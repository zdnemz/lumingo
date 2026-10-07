//! The backup that is taken before a schema upgrade, and its pruning.
//!
//! The build ships a single migration, so the only way to exercise an upgrade
//! is a migration list built for the test, which is what `open_with` exists for.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use common::{TempDir, backups};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use storage::{Database, OpenConfig, OpenMigration};

const INIT: &str = include_str!("../migrations/0001_init.sql");
const M2: &str = "CREATE TABLE extra_two (id INTEGER PRIMARY KEY) STRICT;";
const M3: &str = "CREATE TABLE extra_three (id INTEGER PRIMARY KEY) STRICT;";
const M4: &str = "CREATE TABLE extra_four (id INTEGER PRIMARY KEY) STRICT;";

/// A migration list up to version `n`, starting from the real initial schema.
fn upto(n: i64) -> OpenConfig {
    let mut migrations = vec![OpenMigration {
        version: 1,
        description: "init",
        sql: INIT,
    }];
    for (version, sql) in [(2, M2), (3, M3), (4, M4)] {
        if version <= n {
            migrations.push(OpenMigration {
                version,
                description: "test",
                sql,
            });
        }
    }
    OpenConfig { migrations }
}

async fn table_names(path: &std::path::Path) -> Vec<String> {
    let options = SqliteConnectOptions::new().filename(path);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    pool.close().await;
    names
}

#[tokio::test]
async fn a_fresh_file_makes_no_backup_and_an_upgrade_makes_one() {
    let dir = TempDir::new();

    let db = Database::open_with(dir.db_path(), upto(1)).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), 1);
    db.close().await;
    assert!(backups(dir.path()).is_empty(), "fresh file backed up");

    let db = Database::open_with(dir.db_path(), upto(2)).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), 2);
    db.close().await;

    assert_eq!(backups(dir.path()), vec!["lumingo.sqlite.bak-1"]);

    // The backup holds the schema as it was before the upgrade: version 1 and
    // no extra table.
    let backup = dir.path().join("lumingo.sqlite.bak-1");
    let tables = table_names(&backup).await;
    assert!(tables.iter().any(|t| t == "turns"));
    assert!(!tables.iter().any(|t| t == "extra_two"));

    let options = SqliteConnectOptions::new().filename(&backup);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    let version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(version, Some(1));
}

#[tokio::test]
async fn reopening_at_the_same_version_makes_no_new_backup() {
    let dir = TempDir::new();

    let db = Database::open_with(dir.db_path(), upto(2)).await.unwrap();
    db.close().await;
    let db = Database::open_with(dir.db_path(), upto(2)).await.unwrap();
    db.close().await;

    assert!(backups(dir.path()).is_empty(), "reopen backed up");
}

#[tokio::test]
async fn only_the_two_newest_backups_are_kept() {
    let dir = TempDir::new();

    for n in 1..=4 {
        let db = Database::open_with(dir.db_path(), upto(n)).await.unwrap();
        assert_eq!(db.schema_version().await.unwrap(), n);
        db.close().await;
    }

    assert_eq!(
        backups(dir.path()),
        vec!["lumingo.sqlite.bak-2", "lumingo.sqlite.bak-3"]
    );
}

#[tokio::test]
async fn a_failed_migration_leaves_the_database_unchanged() {
    let dir = TempDir::new();

    let db = Database::open_with(dir.db_path(), upto(1)).await.unwrap();
    db.close().await;

    // Version 2 is deliberately broken.
    let mut migrations = upto(1).migrations;
    migrations.push(OpenMigration {
        version: 2,
        description: "broken",
        sql: "CREATE TABLE half (id INTEGER PRIMARY KEY) STRICT; THIS IS NOT SQL;",
    });

    let error = Database::open_with(dir.db_path(), OpenConfig { migrations })
        .await
        .unwrap_err();
    assert!(
        matches!(error, storage::StorageError::MigrationFailed { .. }),
        "unexpected error: {error:?}"
    );

    // Still at version 1, without the table the broken migration started.
    let db = Database::open_with(dir.db_path(), upto(1)).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), 1);
    db.close().await;
    let tables = table_names(&dir.db_path()).await;
    assert!(!tables.iter().any(|t| t == "half"));
}

#[tokio::test]
async fn a_garbage_file_is_refused_as_not_a_database() {
    let dir = TempDir::new();
    std::fs::write(dir.db_path(), b"this is not sqlite at all, not even close").unwrap();

    let error = Database::open(dir.db_path()).await.unwrap_err();
    assert!(
        matches!(error, storage::StorageError::NotADatabase { .. }),
        "unexpected error: {error:?}"
    );
}
