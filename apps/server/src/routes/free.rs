//! The free modes: writing drafts and graded reading.

use std::sync::Arc;

use app_core::api::{
    AnswerReadingRequest, DraftAccepted, GenerateReadingRequest, ReadingAnswered,
    ReadingOutcomeView, SubmitDraftRequest,
};
use axum::Json;
use axum::extract::{Path, State};

use crate::AppState;
use crate::error::ApiError;
use crate::json::{ApiJson, parse_id};

/// `POST /api/writing/{session}/drafts`. Layer one comes back at once; the rest
/// arrives as a `FeedbackReady` event.
pub async fn submit_draft(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    ApiJson(request): ApiJson<SubmitDraftRequest>,
) -> Result<Json<DraftAccepted>, ApiError> {
    Ok(Json(
        state
            .core
            .submit_draft(parse_id(&session)?, request.text)
            .await?,
    ))
}

/// `POST /api/reading/generate`
pub async fn generate(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<GenerateReadingRequest>,
) -> Result<Json<ReadingOutcomeView>, ApiError> {
    Ok(Json(state.core.generate_reading(request).await?))
}

/// `POST /api/reading/{session}/answers`
pub async fn answer(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    ApiJson(request): ApiJson<AnswerReadingRequest>,
) -> Result<Json<ReadingAnswered>, ApiError> {
    Ok(Json(
        state
            .core
            .answer_reading(parse_id(&session)?, request)
            .await?,
    ))
}
