//! Local database for Lumingo: the schema migrations and the repositories that
//! wrap them. Other crates never see SQL.
//!
//! The schema is migration 0001, copied unchanged from the blueprint's
//! `docs/sql/`. Once a migration has shipped, the copy in `migrations/` is the
//! source of truth and must never be edited; a change is a new numbered file.
//!
//! # Opening
//!
//! [`Database::open`] creates or opens the file in WAL mode with foreign keys
//! on and a busy timeout, copies the old file with `VACUUM INTO` to
//! `<db>.bak-<version>` before an upgrade of an existing schema (the newest two
//! backups are kept), and runs the migrations. One write connection serialises
//! writes; a small pool reads. A file written by a newer build, a file whose
//! applied migration was edited, or a file that is not ours is refused and left
//! untouched.
//!
//! # Repositories
//!
//! Each is a method on [`Database`]: `profiles`, `sessions` (including the
//! audio-clip listing used before a delete), `turns`, `analysis`, and
//! `attempts` with its evidence and pending-scoring queue. A repository returns
//! the plain types of [`models`]; a text value the schema constrains with CHECK
//! is a typed enum, so a row written by a newer build is a typed error, not a
//! silent string.
//!
//! Every timestamp is supplied by the caller as UTC ISO-8601 text; the crate
//! has no clock of its own, so its behaviour is deterministic in tests.
#![forbid(unsafe_code)]

mod analysis;
mod attempts;
mod db;
mod error;
mod models;
mod profiles;
mod rows;
mod sessions;
mod turns;

pub use db::{Database, OpenConfig, OpenMigration, SCHEMA_VERSION};
pub use error::StorageError;
pub use models::*;
