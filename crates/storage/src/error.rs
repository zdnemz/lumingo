//! Typed errors for `storage`. No variant ever carries learner text, a key, or
//! SQL parameters; a caller can log any of these.

use sqlx::migrate::MigrateError;
use std::path::PathBuf;

/// Boxed error source for failures that wrap two different error kinds.
type BoxSource = Box<dyn std::error::Error + Send + Sync>;

/// Everything that can go wrong while opening, migrating or querying the database.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The file is not an SQLite database at all.
    #[error("file is not a SQLite database: {path}")]
    NotADatabase { path: PathBuf },
    /// The file is SQLite, but it was created by something else (it has tables
    /// of its own and no migration history of ours).
    #[error("database at {path} was not created by Lumingo")]
    ForeignDatabase { path: PathBuf },
    /// The database file was written by a newer build of the app.
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: i64, supported: i64 },
    /// The applied migration history does not match this build: a migration was
    /// edited after it ran, or one this build does not know is missing.
    #[error("database migration history does not match this build (version {version})")]
    MigrationMismatch { version: i64 },
    /// A previous migration failed part-way and left the schema dirty.
    #[error("database schema is dirty at version {version}")]
    SchemaDirty { version: i64 },
    /// A migration failed and was rolled back. The database is left as it was.
    #[error("a migration failed and was rolled back; the database is unchanged")]
    MigrationFailed {
        version: Option<i64>,
        #[source]
        source: BoxSource,
    },
    /// The copy made before a migration could not be written. The database is
    /// left untouched.
    #[error("could not back up the database to {path} before migrating")]
    BackupFailed {
        path: PathBuf,
        #[source]
        source: BoxSource,
    },
    /// The requested row does not exist.
    #[error("no {table} row with id {id}")]
    NotFound { table: &'static str, id: i64 },
    /// A uniqueness rule of the schema was violated.
    #[error("a {table} row with this {column} already exists")]
    Conflict {
        table: &'static str,
        column: &'static str,
    },
    /// A value the schema rejects (CHECK, type, NOT NULL, foreign key), or a
    /// stored text value this build does not know.
    #[error("value rejected by the {table} schema: {detail}")]
    Invalid { table: &'static str, detail: String },
    /// The database engine reported an error.
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

/// Maps a sqlx error to the closest typed variant. Callers pass the table name
/// so a message can name it without this crate knowing every query.
pub(crate) fn classify(table: &'static str, error: sqlx::Error) -> StorageError {
    if let sqlx::Error::Database(db) = &error {
        let message = db.message().to_ascii_lowercase();
        if message.contains("unique") || message.contains("primary key") {
            return StorageError::Conflict {
                table,
                column: "unique key",
            };
        }
        if message.contains("check")
            || message.contains("constraint")
            || message.contains("datatype")
            || message.contains("not null")
        {
            return StorageError::Invalid {
                table,
                detail: db.message().to_owned(),
            };
        }
    }
    StorageError::Database(error)
}

/// The row was expected and is missing.
pub(crate) fn not_found(table: &'static str, id: i64) -> StorageError {
    StorageError::NotFound { table, id }
}

/// Maps a migration error to a typed one. The file is untouched in every case:
/// sqlx runs each migration in a transaction and rolls it back on failure.
pub(crate) fn map_migrate_error(error: MigrateError) -> StorageError {
    match error {
        MigrateError::VersionMissing(version) | MigrateError::VersionMismatch(version) => {
            StorageError::MigrationMismatch { version }
        }
        MigrateError::Dirty(version) => StorageError::SchemaDirty { version },
        MigrateError::ExecuteMigration(source, version) => StorageError::MigrationFailed {
            version: Some(version),
            source: Box::new(source),
        },
        MigrateError::Execute(source) => StorageError::MigrationFailed {
            version: None,
            source: Box::new(source),
        },
        other => StorageError::MigrationFailed {
            version: None,
            source: Box::new(other),
        },
    }
}
