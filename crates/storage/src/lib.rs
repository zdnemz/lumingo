//! Local database for Lumingo: migrations and the repositories that wrap them.
//!
//! The schema lives in `migrations/`. Once a migration has shipped, the file in
//! this crate is the source of truth and must never be edited; a change is a new
//! numbered file.
//!
//! Other crates never see SQL. They open a [`Database`] and call repository
//! functions with plain serde types. Every timestamp and every calendar day is
//! supplied by the caller, so the crate has no hidden dependence on the clock or
//! the time zone.
#![forbid(unsafe_code)]

mod db;
mod error;
mod time;

pub use db::{Database, OpenConfig, SCHEMA_VERSION};
pub use error::{Result, StorageError};
pub use time::{LocalDate, TimeError, Timestamp};
