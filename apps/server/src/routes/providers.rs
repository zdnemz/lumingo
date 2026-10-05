use std::sync::Arc;

use app_core::api::{ProbeReport, ProviderInfo, ProviderList, SaveProviderRequest};
use axum::Json;
use axum::extract::{Path, State};

use crate::AppState;
use crate::error::ApiError;
use crate::json::{ApiJson, parse_id};

/// `GET /api/providers`
pub async fn list(State(state): State<Arc<AppState>>) -> Json<ProviderList> {
    Json(state.core.list_providers())
}

/// `POST /api/providers`
pub async fn save(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<SaveProviderRequest>,
) -> Result<Json<ProviderInfo>, ApiError> {
    Ok(Json(state.core.save_provider(request).await?))
}

/// `DELETE /api/providers/{id}`
pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProviderList>, ApiError> {
    Ok(Json(state.core.delete_provider(parse_id(&id)?).await?))
}

/// `POST /api/providers/{id}/test` runs the llm-client probe. If the browser
/// gives up on the request, this future is dropped and the probe is cancelled.
pub async fn test(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProbeReport>, ApiError> {
    Ok(Json(state.core.test_provider(parse_id(&id)?).await?))
}

/// `POST /api/providers/{id}/activate`
pub async fn activate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProviderInfo>, ApiError> {
    Ok(Json(state.core.activate_provider(parse_id(&id)?).await?))
}
