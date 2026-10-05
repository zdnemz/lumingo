//! Local database for Lumingo: migrations and the repositories that wrap them.
//!
//! The schema lives in `migrations/`. Once a migration has shipped, the file in
//! this crate is the source of truth and must never be edited; a change is a new
//! numbered file.
#![forbid(unsafe_code)]
