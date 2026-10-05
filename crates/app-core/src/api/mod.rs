//! Every request, response and event type of the HTTP and WebSocket API.
//!
//! They are defined here once. `cargo test -p app-core` writes the TypeScript
//! declarations to `apps/web/src/generated/` (see `.cargo/config.toml`), and CI
//! fails when the checked-in files differ.

mod common;
mod events;
pub(crate) mod mirror;
mod settings;
mod state;

pub use common::{ApiErrorBody, ErrorCode, Feature, HardwareProfile, L1HelpMode, UiLanguage};
pub use events::ServerEvent;
pub use settings::{AdaptiveTiming, Settings};
pub use state::StateSnapshot;
