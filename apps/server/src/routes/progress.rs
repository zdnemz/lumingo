use std::sync::Arc;

use app_core::api::{AttemptEvidence, ProgressOverview};
use axum::Json;
use axum::extract::{Path, State};

use crate::AppState;
use crate::error::ApiError;
use crate::json::parse_id;

/// `GET /api/progress`
pub async fn overview(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProgressOverview>, ApiError> {
    Ok(Json(state.core.progress().await?))
}

/// `GET /api/attempts/{id}/evidence`
pub async fn evidence(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AttemptEvidence>, ApiError> {
    Ok(Json(state.core.attempt_evidence(parse_id(&id)?).await?))
}
