//! Speech models: the manifest, downloads and the payload inspector.

use std::sync::Arc;

use app_core::api::{Ack, DownloadRequest, DownloadStarted, InspectorReport, ModelList};
use axum::Json;
use axum::extract::{Path, State};

use crate::AppState;
use crate::error::ApiError;
use crate::json::ApiJson;

/// `GET /api/models`
pub async fn list(State(state): State<Arc<AppState>>) -> Result<Json<ModelList>, ApiError> {
    Ok(Json(state.core.models().await?))
}

/// `POST /api/models/{id}/download`. Answers when the download has started;
/// progress and the end arrive as `DownloadProgress` events.
pub async fn download(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<DownloadRequest>,
) -> Result<Json<DownloadStarted>, ApiError> {
    Ok(Json(state.core.download_model(&id, request).await?))
}

/// `POST /api/models/{id}/cancel`
pub async fn cancel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Ack>, ApiError> {
    Ok(Json(state.core.cancel_download(&id)?))
}

/// `GET /api/inspector`
pub async fn inspector(State(state): State<Arc<AppState>>) -> Json<InspectorReport> {
    Json(state.core.inspector())
}
