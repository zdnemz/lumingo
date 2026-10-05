use std::sync::Arc;

use app_core::api::{DeleteDataResult, DeleteSessionResult, ExportBundle};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::error::ApiError;
use crate::json::parse_id;

/// `DELETE /api/sessions/{id}`
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<DeleteSessionResult>, ApiError> {
    Ok(Json(state.core.delete_session(parse_id(&id)?).await?))
}

/// `DELETE /api/data`
pub async fn delete_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DeleteDataResult>, ApiError> {
    Ok(Json(state.core.delete_all_data().await?))
}

/// `GET /api/export`. Sent as a download, named by the day of the export.
pub async fn export(State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    let bundle: ExportBundle = state.core.export().await?;
    let day = bundle.exported_at.get(..10).unwrap_or("export").to_owned();
    let mut response = Json(bundle).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!(
        "attachment; filename=\"lumingo-export-{day}.json\""
    )) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}
