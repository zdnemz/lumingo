//! The session commands of the typed API. Each one checks that the core is
//! running and hands the work to the attached [`SessionService`].

use crate::api::{
    Ack, ActiveSessionView, AnswerReadingRequest, AudioDevices, AudioTestReport, AudioTestRequest,
    DraftAccepted, EditResult, GenerateReadingRequest, NextActivity, ReadingAnswered,
    ReadingOutcomeView, SessionEnded, SpeakAccepted, SpeakRequest, StartSessionRequest,
    StopRequest, SubmitActivityRequest, SubmitActivityResponse, TurnAccepted,
};
use crate::core::AppCore;
use crate::error::CoreResult;

impl AppCore {
    /// Starts a session. At most one runs at a time: a second start is refused
    /// with `Conflict`.
    pub async fn start_session(
        &self,
        request: StartSessionRequest,
    ) -> CoreResult<ActiveSessionView> {
        self.ensure_running()?;
        self.session_service()?.start(request).await
    }

    /// Ends session `id`. Finishing makes the summary; cancelling does not.
    pub async fn stop_session(&self, id: i64, request: StopRequest) -> CoreResult<SessionEnded> {
        self.session_service()?.stop(id, request).await
    }

    pub async fn pause_session(&self, id: i64) -> CoreResult<ActiveSessionView> {
        self.ensure_running()?;
        self.session_service()?.pause(id).await
    }

    pub async fn resume_session(&self, id: i64) -> CoreResult<ActiveSessionView> {
        self.ensure_running()?;
        self.session_service()?.resume(id).await
    }

    /// A typed message as the learner's next turn. The reply arrives as events.
    pub async fn send_text(&self, id: i64, text: String) -> CoreResult<TurnAccepted> {
        self.ensure_running()?;
        self.session_service()?.send_text(id, text).await
    }

    /// Corrects the transcript of a spoken turn.
    pub async fn edit_turn(&self, id: i64, turn: u64, text: String) -> CoreResult<EditResult> {
        self.ensure_running()?;
        self.session_service()?.edit_turn(id, turn, text).await
    }

    pub async fn push_to_talk(&self, id: i64, pressed: bool) -> CoreResult<Ack> {
        self.ensure_running()?;
        self.session_service()?.push_to_talk(id, pressed).await
    }

    /// Stops the tutor, wherever it is speaking.
    pub async fn stop_speaking(&self) -> CoreResult<Ack> {
        self.session_service()?.stop_speaking().await
    }

    pub async fn next_activity(&self, id: i64) -> CoreResult<NextActivity> {
        self.session_service()?.next_activity(id).await
    }

    pub async fn submit_activity(
        &self,
        request: SubmitActivityRequest,
    ) -> CoreResult<SubmitActivityResponse> {
        self.ensure_running()?;
        self.session_service()?.submit_activity(request).await
    }

    pub async fn submit_draft(&self, id: i64, text: String) -> CoreResult<DraftAccepted> {
        self.ensure_running()?;
        self.session_service()?.submit_draft(id, text).await
    }

    pub async fn generate_reading(
        &self,
        request: GenerateReadingRequest,
    ) -> CoreResult<ReadingOutcomeView> {
        self.ensure_running()?;
        self.session_service()?.generate_reading(request).await
    }

    pub async fn answer_reading(
        &self,
        id: i64,
        request: AnswerReadingRequest,
    ) -> CoreResult<ReadingAnswered> {
        self.ensure_running()?;
        self.session_service()?.answer_reading(id, request).await
    }

    /// Speaks a text the server stored. A request never carries text to speak.
    pub async fn speak(&self, request: SpeakRequest) -> CoreResult<SpeakAccepted> {
        self.ensure_running()?;
        self.session_service()?.speak(request).await
    }

    pub async fn audio_devices(&self) -> CoreResult<AudioDevices> {
        self.session_service()?.audio_devices().await
    }

    pub async fn audio_test(&self, request: AudioTestRequest) -> CoreResult<AudioTestReport> {
        self.ensure_running()?;
        self.session_service()?.audio_test(request).await
    }
}
