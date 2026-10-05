use std::sync::Arc;

use app_core::api::{UnitDetail, UnitList};
use axum::Json;
use axum::extract::{Path, State};

use crate::AppState;
use crate::error::ApiError;

/// `GET /api/units`
pub async fn list(State(state): State<Arc<AppState>>) -> Json<UnitList> {
    Json(state.core.list_units())
}

/// `GET /api/units/{id}`
pub async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<UnitDetail>, ApiError> {
    Ok(Json(state.core.unit(&id).await?))
}
