//! The curriculum model and its loader. `load_unit` checks a unit against
//! `curriculum/schema/unit.schema.json` first and only then builds the typed
//! `Unit`, so a broken file is reported with the path of the first problems, not
//! as a vague deserialisation error. The content rules that a schema cannot
//! express (CURRICULUM_SPEC section 6) are validators in a later task.
#![forbid(unsafe_code)]

mod load;
mod model;

pub use load::{LoadError, LoadedFile, SchemaIssue, UnitLoader, load_dir};
pub use model::*;
