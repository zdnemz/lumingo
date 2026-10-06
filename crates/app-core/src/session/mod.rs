//! Sessions: the boundary the core asks, and the manager that implements it.
//!
//! [`SessionService`] is the boundary. The core asks it which session is running,
//! tells it to stop at shutdown, and forwards every session command of the API to
//! it. Until a service is attached with [`crate::AppCore::attach_sessions`] each of
//! those answers [`CoreError::NotAvailable`], and the snapshot lists the feature
//! as unavailable.
//!
//! [`SessionManager`] is the implementation the server attaches. It owns at most
//! one active session at a time and wires the engines of `tutor-engine`, the
//! voice loop of this crate and the speech and audio engines to the event bus.
//! See its documentation for the kinds of session and the rules.

mod catalogs;
mod commands;
mod convert;
mod emit;
mod env;
mod free;
mod manager;
mod speak;
mod text;
mod unit;
mod voice;

use async_trait::async_trait;

use crate::api::{
    Ack, ActiveSessionView, AnswerReadingRequest, AudioDevices, AudioTestReport, AudioTestRequest,
    DraftAccepted, EditResult, EngineView, Feature, GenerateReadingRequest, NextActivity,
    ReadingAnswered, ReadingOutcomeView, SessionEnded, SpeakAccepted, SpeakRequest,
    StartSessionRequest, StopRequest, SubmitActivityRequest, SubmitActivityResponse, TurnAccepted,
};
use crate::error::{CoreError, CoreResult};

pub use manager::SessionManager;

fn not_available<T>() -> CoreResult<T> {
    Err(CoreError::NotAvailable(Feature::Sessions))
}

/// What the core needs from session orchestration, and everything the session
/// routes of the API ask of it. Every command has a default that answers
/// `NotAvailable`, so a service that only knows which session runs (as a test of
/// the core's data commands needs) is a small type.
#[async_trait]
pub trait SessionService: Send + Sync {
    /// The id of the session that is running now, if any. The core refuses to
    /// delete that session and refuses "delete all data" while one runs.
    fn active_session(&self) -> Option<i64>;

    /// Ends the running session, if any, and returns when its workers have
    /// stopped. Called during shutdown.
    async fn stop_active(&self) -> CoreResult<()>;

    /// The running session as the snapshot shows it. Built from memory only.
    fn active_view(&self) -> Option<ActiveSessionView> {
        None
    }

    /// The audio and speech engines as the snapshot shows them. Built from memory
    /// only.
    fn engines(&self) -> Vec<EngineView> {
        Vec::new()
    }

    /// Whether this service provides `feature`, computed from what loaded. The
    /// snapshot lists the features it does not provide as unavailable.
    fn provides(&self, feature: Feature) -> bool {
        feature == Feature::Sessions
    }

    /// Starts a session. Refused while another one runs.
    async fn start(&self, _request: StartSessionRequest) -> CoreResult<ActiveSessionView> {
        not_available()
    }

    /// Ends session `id`, finishing it or cancelling it.
    async fn stop(&self, _id: i64, _request: StopRequest) -> CoreResult<SessionEnded> {
        not_available()
    }

    async fn pause(&self, _id: i64) -> CoreResult<ActiveSessionView> {
        not_available()
    }

    async fn resume(&self, _id: i64) -> CoreResult<ActiveSessionView> {
        not_available()
    }

    /// A typed message as the learner's next turn. Returns when the turn is
    /// accepted; the reply arrives as events.
    async fn send_text(&self, _id: i64, _text: String) -> CoreResult<TurnAccepted> {
        not_available()
    }

    /// Corrects the transcript of a spoken turn, by the turn number its events carry.
    async fn edit_turn(&self, _id: i64, _turn: u64, _text: String) -> CoreResult<EditResult> {
        not_available()
    }

    /// Push-to-talk pressed or released.
    async fn push_to_talk(&self, _id: i64, _pressed: bool) -> CoreResult<Ack> {
        not_available()
    }

    /// Stops the tutor, whether it is thinking or speaking.
    async fn stop_speaking(&self) -> CoreResult<Ack> {
        not_available()
    }

    /// The first activity of a unit session that has no answer yet.
    async fn next_activity(&self, _id: i64) -> CoreResult<NextActivity> {
        not_available()
    }

    async fn submit_activity(
        &self,
        _request: SubmitActivityRequest,
    ) -> CoreResult<SubmitActivityResponse> {
        not_available()
    }

    /// A draft of a writing session. Layer one comes back at once.
    async fn submit_draft(&self, _id: i64, _text: String) -> CoreResult<DraftAccepted> {
        not_available()
    }

    async fn generate_reading(
        &self,
        _request: GenerateReadingRequest,
    ) -> CoreResult<ReadingOutcomeView> {
        not_available()
    }

    async fn answer_reading(
        &self,
        _id: i64,
        _request: AnswerReadingRequest,
    ) -> CoreResult<ReadingAnswered> {
        not_available()
    }

    /// Speaks a text the server stored.
    async fn speak(&self, _request: SpeakRequest) -> CoreResult<SpeakAccepted> {
        not_available()
    }

    async fn audio_devices(&self) -> CoreResult<AudioDevices> {
        not_available()
    }

    async fn audio_test(&self, _request: AudioTestRequest) -> CoreResult<AudioTestReport> {
        not_available()
    }
}
