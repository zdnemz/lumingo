//! The Lumingo local server: a loopback HTTP API, one WebSocket, and the
//! embedded web UI. Handlers here parse, check, and serialise. Tutoring,
//! scoring, and prompt logic never live in this crate.

pub mod events;
pub mod security;
pub mod types;
