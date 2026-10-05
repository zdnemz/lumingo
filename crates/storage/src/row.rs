//! Small helpers for reading rows by column name.
//!
//! Repositories select `*` and read by name, so adding a column in a later
//! migration does not break an older query.

use serde_json::Value;
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

pub(crate) fn opt_enum_col<E: DbEnum>(row: &SqliteRow, column: &'static str) -> Result<Option<E>> {
    let text: Option<String> = row.try_get(column)?;
    text.map(|t| parse_enum(column, &t)).transpose()
}

pub(crate) fn opt_timestamp_col(
    row: &SqliteRow,
    column: &'static str,
) -> Result<Option<Timestamp>> {
    let text: Option<String> = row.try_get(column)?;
    Ok(text.map(|t| Timestamp::parse(&t)).transpose()?)
}

pub(crate) fn json_col(row: &SqliteRow, column: &'static str) -> Result<Value> {
    let text: String = row.try_get(column)?;
    serde_json::from_str(&text).map_err(|_| StorageError::InvalidStored { column })
}

pub(crate) fn opt_json_col(row: &SqliteRow, column: &'static str) -> Result<Option<Value>> {
    let text: Option<String> = row.try_get(column)?;
    text.map(|t| serde_json::from_str(&t).map_err(|_| StorageError::InvalidStored { column }))
        .transpose()
}

/// JSON columns are stored as text and guarded by `json_valid` in the schema.
pub(crate) fn json_text(value: &Value) -> String {
    value.to_string()
}

pub(crate) fn opt_json_text(value: Option<&Value>) -> Option<String> {
    value.map(json_text)
}
