//! The boundary to session orchestration.
//!
//! Starting a lesson, running turns, speaking and scoring are built by other
//! parts of the program (the tutor engine and the audio and speech crates). The
//! core does not contain them and does not pretend to: until a
//! [`SessionService`] is attached with [`crate::AppCore::attach_sessions`],
//! everything that needs one answers [`crate::CoreError::NotAvailable`], and the
//! snapshot lists the feature as unavailable.
//!
//! The trait holds only what the core itself must ask a running session
//! (which one is active, and to stop it). Start, pause, text and the other
//! session commands are added to this trait when the tutor engine is wired in,
//! so the server implements them against one named boundary.

use async_trait::async_trait;

use crate::error::CoreResult;

#[async_trait]
pub trait SessionService: Send + Sync {
    /// The id of the session that is running now, if any. The core refuses to
    /// delete that session and refuses "delete all data" while one runs.
    fn active_session(&self) -> Option<i64>;

    /// Ends the running session, if any, and returns when its workers have
    /// stopped. Called during shutdown.
    async fn stop_active(&self) -> CoreResult<()>;
}
