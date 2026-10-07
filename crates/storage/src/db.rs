//! Opening the database: connection settings, the embedded migration list, and
//! the backup that is taken before a schema upgrade.
//!
//! One writer pool (a single connection, so writes serialise) and a small
//! reader pool share the file in WAL mode. A file that is not ours, a file from
//! a newer build, or a file whose applied migration was edited is refused
//! untouched.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::migrate::{Migration, MigrationType, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, SqlitePool};

use crate::error::{StorageError, map_migrate_error};

/// Newest schema version this build knows: the version of the last embedded
/// migration. Asserted against the list by a unit test below.
pub const SCHEMA_VERSION: i64 = 1;

/// How many `<db>.bak-<version>` files are kept next to the database.
const KEPT_BACKUPS: usize = 2;

/// Reader pool size. One writer connection is enough; readers run short queries.
const READER_CONNECTIONS: u32 = 4;

/// One migration as this crate embeds it. Plain data, so the configuration does
/// not leak sqlx types into the rest of the workspace.
#[derive(Debug, Clone, Copy)]
pub struct OpenMigration {
    pub version: i64,
    pub description: &'static str,
    pub sql: &'static str,
}

/// Every migration, in order. Listed explicitly rather than read from a folder,
/// so a stray file can never change the schema and the SQL is part of the
/// binary. Once a migration has shipped, its file must never be edited; a
/// change is a new numbered file.
const EMBEDDED_MIGRATIONS: &[OpenMigration] = &[OpenMigration {
    version: 1,
    description: "init",
    sql: include_str!("../migrations/0001_init.sql"),
}];

/// How to open a database: which migrations to apply. The default is the
/// embedded list that the application uses. Tests and tools may pass their own
/// list to exercise an upgrade, which is the only way the backup path runs
/// while the build ships a single migration.
pub struct OpenConfig {
    pub migrations: Vec<OpenMigration>,
}

impl Default for OpenConfig {
    fn default() -> Self {
        Self {
            migrations: EMBEDDED_MIGRATIONS.to_vec(),
        }
    }
}

/// Builds sqlx migrations from the plain list. `Migration::new` computes the
/// checksum, so an edited SQL file is refused on the next open.
fn to_sqlx_migrations(config: &[OpenMigration]) -> Vec<Migration> {
    config
        .iter()
        .map(|file| {
            Migration::new(
                file.version,
                Cow::Borrowed(file.description),
                MigrationType::Simple,
                Cow::Borrowed(file.sql),
                false,
            )
        })
        .collect()
}

/// What an existing file looks like before we touch it.
enum FileState {
    /// No file, or a file with no tables of its own.
    Fresh,
    /// Our schema; `version` is the newest applied migration (0 when the
    /// bookkeeping table exists but is empty).
    Ours { version: i64 },
    /// Tables exist but none of them is our migration table.
    Foreign,
}

/// The open database: one write pool, one read pool, and the file path.
#[derive(Debug)]
pub struct Database {
    path: PathBuf,
    writer: SqlitePool,
    readers: SqlitePool,
}

impl Database {
    /// Opens the database at `path`, creating and migrating it when needed.
    /// Uses the embedded migrations.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open_with(path, OpenConfig::default()).await
    }

    /// Opens with an explicit migration list. The application uses [`open`];
    /// this exists for tests and tools that exercise an upgrade.
    ///
    /// [`open`]: Database::open
    pub async fn open_with(
        path: impl AsRef<Path>,
        config: OpenConfig,
    ) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let target = config.migrations.last().map(|m| m.version).unwrap_or(0);

        match inspect(&path).await? {
            FileState::Foreign => return Err(StorageError::ForeignDatabase { path }),
            FileState::Fresh => {}
            FileState::Ours { version } => {
                if version > target {
                    return Err(StorageError::SchemaTooNew {
                        found: version,
                        supported: target,
                    });
                }
                // An upgrade of a schema that already holds data is the only
                // case that gets a backup. A fresh file has nothing to lose.
                if version > 0 && version < target {
                    backup(&path, version).await?;
                }
            }
        }

        let writer = open_pool(&path, 1).await?;
        let readers = open_pool(&path, READER_CONNECTIONS).await?;

        let migrator = Migrator {
            migrations: Cow::Owned(to_sqlx_migrations(&config.migrations)),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        };
        migrator.run(&writer).await.map_err(map_migrate_error)?;

        Ok(Database {
            path,
            writer,
            readers,
        })
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The write pool: one connection, so writes serialise. Repositories use
    /// this. Exposed for tests and diagnostics; application code should go
    /// through the repositories.
    pub fn writer(&self) -> &SqlitePool {
        &self.writer
    }

    /// The read pool. See [`writer`](Database::writer) about direct use.
    pub fn readers(&self) -> &SqlitePool {
        &self.readers
    }

    /// The newest applied migration version in the file.
    pub async fn schema_version(&self) -> Result<i64, StorageError> {
        let version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&self.writer)
            .await?;
        Ok(version.unwrap_or(0))
    }

    /// Rebuilds the file to reclaim space, for example after deleting a lot of
    /// learner data. Cannot run inside a transaction, so it is a bare VACUUM.
    pub async fn compact(&self) -> Result<(), StorageError> {
        sqlx::query("VACUUM").execute(&self.writer).await?;
        Ok(())
    }

    /// Closes both pools. Callers should close before deleting or moving the
    /// file; dropping the value closes them too.
    pub async fn close(&self) {
        self.writer.close().await;
        self.readers.close().await;
    }
}

/// Opens a pool with the settings every connection in this app must have:
/// foreign keys on, WAL, a busy timeout, and `synchronous = NORMAL` (the WAL
/// recommendation: durable against app crashes, not against power loss).
async fn open_pool(path: &Path, max_connections: u32) -> Result<SqlitePool, StorageError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .map_err(StorageError::Database)
}

/// Looks at an existing file without changing it.
async fn inspect(path: &Path) -> Result<FileState, StorageError> {
    if !path.exists() {
        return Ok(FileState::Fresh);
    }

    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .connect()
        .await
        .map_err(|error| map_open_error(path, error))?;

    let tables: Result<Vec<String>, sqlx::Error> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&mut conn)
    .await;

    let version: Result<Option<i64>, sqlx::Error> = match &tables {
        Ok(names) if names.iter().any(|name| name == "_sqlx_migrations") => {
            sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
                .fetch_one(&mut conn)
                .await
        }
        _ => Ok(None),
    };

    let _ = conn.close().await;

    let tables = tables.map_err(|error| map_open_error(path, error))?;
    if tables.is_empty() {
        return Ok(FileState::Fresh);
    }
    if !tables.iter().any(|name| name == "_sqlx_migrations") {
        return Ok(FileState::Foreign);
    }
    let version = version.map_err(|error| map_open_error(path, error))?;
    Ok(FileState::Ours {
        version: version.unwrap_or(0),
    })
}

/// Copies the file with `VACUUM INTO` before an upgrade. `VACUUM INTO` reads
/// through the WAL, so a commit that is not yet checkpointed is included.
async fn backup(path: &Path, from_version: i64) -> Result<(), StorageError> {
    let target = backup_path(path, from_version);
    let fail = |source: Box<dyn std::error::Error + Send + Sync>| StorageError::BackupFailed {
        path: target.clone(),
        source,
    };

    if target.exists() {
        std::fs::remove_file(&target).map_err(|source| fail(Box::new(source)))?;
    }

    let mut conn = SqliteConnectOptions::new()
        .filename(path)
        .connect()
        .await
        .map_err(StorageError::Database)?;
    sqlx::query("VACUUM INTO ?")
        .bind(target.to_string_lossy().into_owned())
        .execute(&mut conn)
        .await
        .map_err(|source| fail(Box::new(source)))?;
    let _ = conn.close().await;

    prune_backups(path, KEPT_BACKUPS);
    Ok(())
}

/// `<db>.bak-<version>`, next to the database file.
fn backup_path(path: &Path, version: i64) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(format!(".bak-{version}"));
    path.with_file_name(name)
}

/// Keeps the newest `keep` backups and deletes the rest. Cleanup only: a file
/// that cannot be removed is logged, never fatal, because the backup itself
/// already succeeded.
fn prune_backups(path: &Path, keep: usize) {
    let Some(parent) = path.parent() else {
        return;
    };
    let Some(file_name) = path.file_name() else {
        return;
    };
    let prefix = format!("{}.bak-", file_name.to_string_lossy());

    let mut backups: Vec<(i64, PathBuf)> = match std::fs::read_dir(parent) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let version = name.strip_prefix(&prefix)?.parse::<i64>().ok()?;
                Some((version, entry.path()))
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "could not list backups for pruning");
            return;
        }
    };

    backups.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    for (version, old) in backups.into_iter().skip(keep) {
        if let Err(error) = std::fs::remove_file(&old) {
            tracing::warn!(%error, version, "could not prune an old backup");
        }
    }
}

/// "File is not a database" from SQLite becomes a typed error; everything else
/// passes through.
fn map_open_error(path: &Path, error: sqlx::Error) -> StorageError {
    let message = match &error {
        sqlx::Error::Database(db) => db.message().to_ascii_lowercase(),
        other => other.to_string().to_ascii_lowercase(),
    };
    if message.contains("not a database") {
        return StorageError::NotADatabase {
            path: path.to_path_buf(),
        };
    }
    StorageError::Database(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_matches_the_last_embedded_migration() {
        let last = EMBEDDED_MIGRATIONS
            .last()
            .map(|file| file.version)
            .unwrap_or(0);
        assert_eq!(SCHEMA_VERSION, last);
    }

    #[test]
    fn embedded_migrations_are_ordered_and_nonempty() {
        let versions: Vec<i64> = EMBEDDED_MIGRATIONS.iter().map(|f| f.version).collect();
        assert!(!versions.is_empty());
        assert!(versions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(EMBEDDED_MIGRATIONS.iter().all(|f| !f.sql.trim().is_empty()));
    }

    #[test]
    fn backup_names_sit_next_to_the_database() {
        let path = Path::new("/data/lumingo.sqlite");
        assert_eq!(
            backup_path(path, 3),
            PathBuf::from("/data/lumingo.sqlite.bak-3")
        );
    }
}
