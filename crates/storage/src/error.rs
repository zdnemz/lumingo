use std::path::PathBuf;

use sqlx::error::{DatabaseError, ErrorKind};

use crate::time::TimeError;

/// Everything the storage crate can fail with.
///
/// Messages never contain row values, so an error that reaches a log line cannot
/// leak learner text.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The database file could not be opened or created.
    #[error("cannot open the database at {}: {source}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: sqlx::Error,
    },
    /// The database file was written by a newer build than this one.
    #[error(
        "the database is at schema version {found}, but this build only knows version {supported}; update the program"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    /// A migration that was already applied no longer matches the file in this build.
    #[error("migration {0} was changed after it was applied to this database")]
    MigrationChanged(i64),
    /// A migration could not be applied. The database keeps its previous schema.
    #[error("migration failed and the database was left at its previous schema: {0}")]
    Migration(#[source] sqlx::migrate::MigrateError),
    /// The copy made before a schema upgrade failed, so the upgrade did not start.
    #[error("cannot back up the database to {} before the upgrade: {reason}", path.display())]
    Backup { path: PathBuf, reason: String },
    /// A CHECK, UNIQUE, NOT NULL or FOREIGN KEY constraint refused the row.
    #[error("the database refused the row: {0}")]
    Constraint(String),
    /// The row asked for does not exist.
    #[error("{what} not found")]
    NotFound { what: &'static str },
    /// A repository rule refused the operation before it reached the database.
    #[error("rule violated: {0}")]
    Rule(&'static str),
    /// A stored value is not one this build understands.
    #[error("column {column} holds a value this build does not understand")]
    InvalidStored { column: &'static str },
    /// A time value from the caller or from the database could not be read.
    #[error("invalid time value: {0}")]
    Time(#[from] TimeError),
    /// Any other database failure.
    #[error("database error: {0}")]
    Database(#[source] sqlx::Error),
}

impl From<sqlx::Error> for StorageError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::RowNotFound => StorageError::NotFound { what: "row" },
            sqlx::Error::Database(db) if is_constraint(db.as_ref()) => {
                StorageError::Constraint(db.message().to_owned())
            }
            other => StorageError::Database(other),
        }
    }
}

fn is_constraint(error: &dyn DatabaseError) -> bool {
    matches!(
        error.kind(),
        ErrorKind::UniqueViolation
            | ErrorKind::ForeignKeyViolation
            | ErrorKind::CheckViolation
            | ErrorKind::NotNullViolation
    )
}

/// The result type used by every function in this crate.
pub type Result<T> = std::result::Result<T, StorageError>;
