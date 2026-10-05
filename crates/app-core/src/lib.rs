//! The core of Lumingo: everything `apps/server` exposes, without HTTP.
//!
//! `apps/server` parses a request, checks it, calls one method here and
//! serialises the answer. Everything the server can do is therefore also
//! reachable from a test or a command-line tool without a socket.
//!
//! # What lives here
//!
//! - [`api`]: every request, response and event type. They are exported to
//!   TypeScript from here and nowhere else.
//! - [`hardware`]: memory and logical processors, as measured.
//! - [`events`]: the event bus.
//! - [`session`]: the boundary to session orchestration, which is not built in.
//! - [`AppCore`]: lifecycle and the typed command and query API.
//!
//! # Channels
//!
//! There is exactly one channel, the event bus of [`events`]. It is a Tokio
//! broadcast channel of capacity 64. When a client falls 64 events behind, the
//! oldest events are dropped for that client only, the sender never waits, and
//! the client is sent a fresh `Snapshot` instead of what it missed.
#![forbid(unsafe_code)]

pub mod api;
pub mod clock;
pub mod config;
mod core;
mod data;
mod diagnostics;
pub mod error;
pub mod events;
pub mod hardware;
mod progress;
mod providers;
mod rewards;
pub mod session;
mod settings;
mod units;
pub mod voice;

pub use clock::{Clock, SystemClock};
pub use config::{CoreConfig, default_curriculum_dir, default_data_dir};
pub use core::AppCore;
pub use error::{CoreError, CoreResult};
pub use session::SessionService;
