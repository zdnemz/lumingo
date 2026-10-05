//! Opening the database: connection settings, migrations and the backup that
//! comes before a schema upgrade.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::migrate::{MigrateError, Migration, MigrationType, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, SqlSafeStr, SqliteConnection, SqlitePool};

use crate::error::{Result, StorageError};

/// Newest schema version this build knows. The last entry of `MIGRATIONS`.
pub const SCHEMA_VERSION: i64 = 1;

/// How many `<db>.bak-<version>` files are kept next to the database.
const KEPT_BACKUPS: usize = 2;

struct MigrationFile {
    version: i64,
    description: &'static str,
    sql: &'static str,
}

/// Every migration, in order. Listed explicitly rather than read from a folder so
/// a stray file can never change the schema, and so the SQL is part of the binary.
const MIGRATIONS: &[MigrationFile] = &[MigrationFile {
    version: 1,
    description: "init",
    sql: include_str!("../migrations/0001_init.sql"),
}];

fn migrator(up_to: i64) -> Migrator {
    let migrations = MIGRATIONS
        .iter()
        .filter(|file| file.version <= up_to)
        .map(|file| {
            Migration::new(
                file.version,
                file.description.into(),
                MigrationType::Simple,
                file.sql.into_sql_str(),
                false,
            )
        })
        .collect();
    Migrator::with_migrations(migrations)
}

/// Settings for [`Database::open_with`].
#[derive(Debug, Clone)]
pub struct OpenConfig {
    /// How long a connection waits for a lock held by another connection before
    /// the call fails with a busy error.
    pub busy_timeout: Duration,
    /// Size of the read pool. The write side is always exactly one connection.
    pub read_connections: u32,
}

impl Default for OpenConfig {
    fn default() -> Self {
        Self {
            busy_timeout: Duration::from_secs(5),
            read_connections: 4,
        }
    }
}

/// The local database: one write connection and a small pool of readers.
///
/// Cloning is cheap and every clone shares the same connections. Writes queue on
/// the single write connection, so two writers inside this process cannot
/// deadlock on SQLite's write lock. Readers never block the writer under WAL.
///
/// Every function is async and cancel-safe: dropping the future rolls back an
/// unfinished transaction.
#[derive(Debug, Clone)]
pub struct Database {
    write: SqlitePool,
    read: SqlitePool,
    path: PathBuf,
}

impl Database {
    /// Opens or creates the database at `path` with the default settings and
    /// brings the schema up to date.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenConfig::default()).await
    }

    /// Like [`Database::open`] with explicit settings.
    pub async fn open_with(path: impl AsRef<Path>, config: OpenConfig) -> Result<Self> {
        Self::open_up_to(path.as_ref(), &config, SCHEMA_VERSION).await
    }

    /// Opens the database as a build that only knows migrations up to `up_to`.
    /// Production code always passes [`SCHEMA_VERSION`]; the lower values let
    /// tests build an old database and then upgrade it.
    pub(crate) async fn open_up_to(path: &Path, config: &OpenConfig, up_to: i64) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| StorageError::Open {
                    path: path.to_owned(),
                    source: sqlx::Error::Io(source),
                })?;
        }

        let write_options = write_options(path, config);
        prepare_schema(path, &write_options, up_to).await?;

        let open_error = |source| StorageError::Open {
            path: path.to_owned(),
            source,
        };
        let write = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(write_options)
            .await
            .map_err(open_error)?;
        let read = SqlitePoolOptions::new()
            .max_connections(config.read_connections.max(1))
            .connect_with(read_options(path, config))
            .await
            .map_err(open_error)?;

        Ok(Self {
            write,
            read,
            path: path.to_owned(),
        })
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The schema version recorded in the file.
    pub async fn schema_version(&self) -> Result<i64> {
        let mut conn = self.reader().acquire().await?;
        applied_version(&mut conn).await
    }

    /// Folds the write-ahead log into the main file and rewrites the file without
    /// free pages. Call it after deleting learning data so deleted text does not
    /// linger in unused pages.
    pub async fn compact(&self) -> Result<()> {
        let mut conn = self.writer().acquire().await?;
        sqlx::query("VACUUM").execute(&mut *conn).await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    /// Flushes the log and closes every connection of every clone. Call it on
    /// shutdown.
    pub async fn close(&self) -> Result<()> {
        if !self.writer().is_closed() {
            let mut conn = self.writer().acquire().await?;
            sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                .execute(&mut *conn)
                .await?;
        }
        self.read.close().await;
        self.write.close().await;
        Ok(())
    }

    /// The pool for queries that only read.
    pub(crate) fn reader(&self) -> &SqlitePool {
        &self.read
    }

    /// The single write connection, for statements that change one row.
    pub(crate) fn writer(&self) -> &SqlitePool {
        &self.write
    }
}

fn write_options(path: &Path, config: &OpenConfig) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // NORMAL cannot corrupt the file under WAL. It can lose the last commits
        // on power loss, which is acceptable for learning progress.
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(config.busy_timeout)
}

fn read_options(path: &Path, config: &OpenConfig) -> SqliteConnectOptions {
    // No journal_mode pragma here: WAL is stored in the file and a read-only
    // connection must not try to change it.
    SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .foreign_keys(true)
        .busy_timeout(config.busy_timeout)
}

/// Brings the file to the newest schema this build knows, after saving a copy of
/// the old file when the upgrade changes an existing schema.
async fn prepare_schema(path: &Path, options: &SqliteConnectOptions, up_to: i64) -> Result<()> {
    let mut conn = options
        .connect()
        .await
        .map_err(|source| StorageError::Open {
            path: path.to_owned(),
            source,
        })?;

    let outcome = upgrade(&mut conn, path, up_to).await;
    // Closing the last connection also folds the log back into the main file.
    let closed = conn.close().await;
    outcome?;
    closed?;
    Ok(())
}

async fn upgrade(conn: &mut SqliteConnection, path: &Path, up_to: i64) -> Result<()> {
    let current = applied_version(conn).await?;
    if current > up_to {
        return Err(StorageError::SchemaTooNew {
            found: current,
            supported: up_to,
        });
    }
    // A fresh file (version 0) has nothing to lose, so it gets no backup.
    if current > 0 && current < up_to {
        tracing::info!(from = current, to = up_to, "upgrading the database schema");
        backup_before_upgrade(conn, path, current).await?;
    }
    migrator(up_to)
        .run(&mut *conn)
        .await
        .map_err(|error| match error {
            MigrateError::VersionMismatch(version) => StorageError::MigrationChanged(version),
            other => StorageError::Migration(other),
        })
}

/// Highest migration recorded as applied, or 0 for a file with none.
async fn applied_version(conn: &mut SqliteConnection) -> Result<i64> {
    let table: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if table.is_none() {
        return Ok(0);
    }
    let version: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&mut *conn)
            .await?;
    Ok(version.unwrap_or(0))
}

/// `<db>.bak-<version>`, next to the database file.
fn backup_path(db_path: &Path, version: i64) -> PathBuf {
    let mut name = db_path.as_os_str().to_owned();
    name.push(format!(".bak-{version}"));
    PathBuf::from(name)
}

/// Writes a consistent copy of the database to `<db>.bak-<old version>`.
///
/// `VACUUM INTO` takes the snapshot inside SQLite, so the copy is complete even
/// though recent commits may still sit in the write-ahead log. The copy is written
/// under a temporary name and renamed, so an interrupted backup never leaves a
/// truncated file under the final name.
async fn backup_before_upgrade(
    conn: &mut SqliteConnection,
    db_path: &Path,
    old_version: i64,
) -> Result<()> {
    let target = backup_path(db_path, old_version);
    let backup_error = |reason: String| StorageError::Backup {
        path: target.clone(),
        reason,
    };

    let mut partial = target.clone().into_os_string();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    remove_if_exists(&partial)
        .await
        .map_err(|e| backup_error(e.to_string()))?;
    // A backup of the same version left by an earlier, failed upgrade is replaced
    // by the copy of what the file holds now.
    remove_if_exists(&target)
        .await
        .map_err(|e| backup_error(e.to_string()))?;

    let partial_text = partial
        .to_str()
        .ok_or_else(|| backup_error("the path is not valid UTF-8".to_owned()))?;
    sqlx::query("VACUUM INTO ?1")
        .bind(partial_text)
        .execute(&mut *conn)
        .await
        .map_err(|e| backup_error(e.to_string()))?;
    tokio::fs::rename(&partial, &target)
        .await
        .map_err(|e| backup_error(e.to_string()))?;

    prune_backups(db_path)
        .await
        .map_err(|e| backup_error(e.to_string()))
}

async fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Keeps the newest [`KEPT_BACKUPS`] backups of this database and deletes the rest.
async fn prune_backups(db_path: &Path) -> std::io::Result<()> {
    let Some(file_name) = db_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return Ok(());
    };
    let dir = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let prefix = format!("{file_name}.bak-");

    let mut found: Vec<(i64, PathBuf)> = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        // `.partial` files do not parse as a number and are skipped here.
        if let Some(version) = name
            .strip_prefix(&prefix)
            .and_then(|v| v.parse::<i64>().ok())
        {
            found.push((version, entry.path()));
        }
    }
    found.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    for (_, stale) in found.into_iter().skip(KEPT_BACKUPS) {
        remove_if_exists(&stale).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_matches_the_last_migration() {
        let last = MIGRATIONS.last().map(|m| m.version);
        assert_eq!(last, Some(SCHEMA_VERSION));
        let versions: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
        let expected: Vec<i64> = (1..=SCHEMA_VERSION).collect();
        assert_eq!(versions, expected, "versions must be consecutive from 1");
    }

    #[tokio::test]
    async fn open_creates_a_wal_database_with_foreign_keys_on() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = Database::open(dir.path().join("lumingo.sqlite"))
            .await
            .expect("open");

        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(db.writer())
            .await
            .expect("journal_mode");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(db.writer())
            .await
            .expect("foreign_keys");
        let busy: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(db.reader())
            .await
            .expect("busy_timeout");
        assert_eq!(mode, "wal");
        assert_eq!(foreign_keys, 1);
        assert_eq!(busy, 5_000);
        assert_eq!(db.schema_version().await.expect("version"), SCHEMA_VERSION);
        db.close().await.expect("close");
    }

    async fn open_plain(path: &Path) -> SqliteConnection {
        write_options(path, &OpenConfig::default())
            .connect()
            .await
            .expect("connect")
    }

    #[tokio::test]
    async fn backup_is_a_complete_copy_even_when_commits_sit_in_the_log() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("lumingo.sqlite");
        let mut conn = open_plain(&path).await;
        sqlx::query("CREATE TABLE note (body TEXT NOT NULL) STRICT")
            .execute(&mut conn)
            .await
            .expect("create");
        sqlx::query("INSERT INTO note (body) VALUES ('kept')")
            .execute(&mut conn)
            .await
            .expect("insert");

        backup_before_upgrade(&mut conn, &path, 7)
            .await
            .expect("backup");

        let backup = backup_path(&path, 7);
        assert!(backup.exists());
        let mut copy = open_plain(&backup).await;
        let body: String = sqlx::query_scalar("SELECT body FROM note")
            .fetch_one(&mut copy)
            .await
            .expect("read copy");
        assert_eq!(body, "kept");
    }

    #[tokio::test]
    async fn only_the_two_newest_backups_are_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("lumingo.sqlite");
        let mut conn = open_plain(&path).await;
        for version in [1, 2, 3, 10] {
            backup_before_upgrade(&mut conn, &path, version)
                .await
                .expect("backup");
        }
        // A file with a similar name that is not a backup must survive.
        let unrelated = dir.path().join("lumingo.sqlite.bak-notes");
        std::fs::write(&unrelated, b"x").expect("write");
        prune_backups(&path).await.expect("prune");

        assert!(!backup_path(&path, 1).exists());
        assert!(!backup_path(&path, 2).exists());
        assert!(backup_path(&path, 3).exists());
        assert!(backup_path(&path, 10).exists());
        assert!(unrelated.exists());
    }
}
