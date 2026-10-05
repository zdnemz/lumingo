//! Every request, response and event type of the HTTP and WebSocket API.
//!
//! They are defined here once. `cargo test -p app-core` writes the TypeScript
//! declarations to `apps/web/src/generated/` (see `.cargo/config.toml`), and CI
//! fails when the checked-in files differ.

mod common;
pub(crate) mod mirror;

pub use common::{ApiErrorBody, ErrorCode, Feature, HardwareProfile, L1HelpMode, UiLanguage};
