//! Sessions, turns and activities.

use std::sync::Arc;

use app_core::api::{
    Ack, ActiveSessionView, EditResult, EditTurnRequest, NextActivity, PushToTalkRequest,
    SendTextRequest, SessionEnded, StartSessionRequest, StopRequest, SubmitActivityRequest,
    SubmitActivityResponse, TurnAccepted,
};
use axum::Json;
use axum::extract::{Path, State};

use crate::AppState;
use crate::error::ApiError;
use crate::json::{ApiJson, OptionalJson, parse_id, parse_turn};

/// `POST /api/sessions`
pub async fn start(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<StartSessionRequest>,
) -> Result<Json<ActiveSessionView>, ApiError> {
    Ok(Json(state.core.start_session(request).await?))
}

/// `POST /api/sessions/{id}/stop`. The body may be empty: that finishes the
/// session.
pub async fn stop(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    OptionalJson(request): OptionalJson<StopRequest>,
) -> Result<Json<SessionEnded>, ApiError> {
    Ok(Json(
        state.core.stop_session(parse_id(&id)?, request).await?,
    ))
}

/// `POST /api/sessions/{id}/pause`
pub async fn pause(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ActiveSessionView>, ApiError> {
    Ok(Json(state.core.pause_session(parse_id(&id)?).await?))
}

/// `POST /api/sessions/{id}/resume`
pub async fn resume(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ActiveSessionView>, ApiError> {
    Ok(Json(state.core.resume_session(parse_id(&id)?).await?))
}

/// `POST /api/sessions/{id}/text`. Answers when the turn is accepted; the reply
/// arrives on the event stream.
pub async fn text(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<SendTextRequest>,
) -> Result<Json<TurnAccepted>, ApiError> {
    Ok(Json(
        state.core.send_text(parse_id(&id)?, request.text).await?,
    ))
}

/// `POST /api/sessions/{id}/turns/{seq}/edit`. `seq` is the turn number the
/// events of the turn carry.
pub async fn edit(
    State(state): State<Arc<AppState>>,
    Path((id, seq)): Path<(String, String)>,
    ApiJson(request): ApiJson<EditTurnRequest>,
) -> Result<Json<EditResult>, ApiError> {
    Ok(Json(
        state
            .core
            .edit_turn(parse_id(&id)?, parse_turn(&seq)?, request.text)
            .await?,
    ))
}

/// `POST /api/sessions/{id}/push-to-talk`
pub async fn push_to_talk(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<PushToTalkRequest>,
) -> Result<Json<Ack>, ApiError> {
    Ok(Json(
        state
            .core
            .push_to_talk(parse_id(&id)?, request.pressed)
            .await?,
    ))
}

/// `POST /api/tutor/stop-speaking`
pub async fn stop_speaking(State(state): State<Arc<AppState>>) -> Result<Json<Ack>, ApiError> {
    Ok(Json(state.core.stop_speaking().await?))
}

/// `GET /api/sessions/{id}/next-activity`
pub async fn next_activity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<NextActivity>, ApiError> {
    Ok(Json(state.core.next_activity(parse_id(&id)?).await?))
}

/// `POST /api/activities/submit`
pub async fn submit_activity(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<SubmitActivityRequest>,
) -> Result<Json<SubmitActivityResponse>, ApiError> {
    Ok(Json(state.core.submit_activity(request).await?))
}
