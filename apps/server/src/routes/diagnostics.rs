use std::sync::Arc;

use app_core::api::DiagnosticsReport;
use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::error::ApiError;

/// `GET /api/diagnostics`
pub async fn get(State(state): State<Arc<AppState>>) -> Result<Json<DiagnosticsReport>, ApiError> {
    Ok(Json(state.core.diagnostics().await?))
}
