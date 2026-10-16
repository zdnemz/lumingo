use std::sync::Arc;

use app_core::api::{AttemptEvidence, AttemptList, ProgressOverview};
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::AppState;
use crate::error::ApiError;
use crate::json::parse_id;

/// Query parameters of `GET /api/attempts`.
#[derive(Debug, Deserialize)]
pub struct AttemptQuery {
    pub skill: Option<String>,
}

/// `GET /api/progress`
pub async fn overview(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProgressOverview>, ApiError> {
    Ok(Json(state.core.progress().await?))
}

/// `GET /api/attempts?skill=...`
pub async fn attempts(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AttemptQuery>,
) -> Result<Json<AttemptList>, ApiError> {
    let Some(skill) = query.skill else {
        return Err(ApiError::invalid("the skill query parameter is missing"));
    };
    if skill.is_empty() {
        return Err(ApiError::invalid("the skill query parameter is empty"));
    }
    Ok(Json(state.core.skill_attempts(&skill).await?))
}

/// `GET /api/attempts/{id}/evidence`
pub async fn evidence(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AttemptEvidence>, ApiError> {
    Ok(Json(state.core.attempt_evidence(parse_id(&id)?).await?))
}
