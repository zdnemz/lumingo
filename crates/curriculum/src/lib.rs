//! Curriculum units for Lumingo: typed model, schema-checked loader and content checksum.
//!
//! The unit format is defined by `curriculum/schema/unit.schema.json` and explained in
//! `docs/CURRICULUM_SPEC.md`. [`model`] mirrors the schema, [`load`] checks a file against
//! the schema before it deserialises it, and [`index`] holds the plain row that
//! `app-core` writes to the database index. This crate does not depend on `storage`.
//!
//! The optional `ts` feature derives `ts_rs::TS` for every public unit type so the UI can
//! reuse the types. It is off by default.

#![forbid(unsafe_code)]

pub mod index;
pub mod load;
pub mod model;
pub mod schema;
pub mod syllabus;
pub mod validate;

pub use index::{SkillCounts, UnitIndexEntry};
pub use load::{
    LoadError, LoadedUnit, UnitFile, load_unit_bytes, load_unit_dir, load_unit_file, sha256_hex,
    unit_from_value,
};
pub use model::*;
pub use schema::{SchemaChecker, SchemaIssue, SchemaLoadError};
