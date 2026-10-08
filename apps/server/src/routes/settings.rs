use std::sync::Arc;

use app_core::api::Settings;
use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::error::ApiError;
use crate::json::ApiJson;

/// `GET /api/settings`
pub async fn get(State(state): State<Arc<AppState>>) -> Json<Settings> {
    Json(state.core.settings())
}

/// `PUT /api/settings` replaces every setting.
pub async fn put(
    State(state): State<Arc<AppState>>,
    ApiJson(settings): ApiJson<Settings>,
) -> Result<Json<Settings>, ApiError> {
    Ok(Json(state.core.update_settings(settings).await?))
}
