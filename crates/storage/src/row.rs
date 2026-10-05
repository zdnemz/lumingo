//! Small helpers for reading rows by column name.
//!
//! Repositories select `*` and read by name, so adding a column in a later
//! migration does not break an older query.

use sqlx::Row;
use sqlx::sqlite::{SqliteQueryResult, SqliteRow};

use crate::enums::{DbEnum, parse_enum};
use crate::error::{Result, StorageError};
use crate::time::Timestamp;

pub(crate) fn enum_col<E: DbEnum>(row: &SqliteRow, column: &'static str) -> Result<E> {
    let text: String = row.try_get(column)?;
    parse_enum(column, &text)
}

pub(crate) fn timestamp_col(row: &SqliteRow, column: &'static str) -> Result<Timestamp> {
    let text: String = row.try_get(column)?;
    Ok(Timestamp::parse(&text)?)
}

/// Turns "no row changed" into `NotFound` for updates and deletes by id.
pub(crate) fn expect_changed(result: &SqliteQueryResult, what: &'static str) -> Result<()> {
    if result.rows_affected() == 0 {
        Err(StorageError::NotFound { what })
    } else {
        Ok(())
    }
}
