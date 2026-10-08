use std::sync::Arc;

use app_core::api::StateSnapshot;
use axum::Json;
use axum::extract::State;

use crate::AppState;

/// `GET /api/state`
pub async fn get(State(state): State<Arc<AppState>>) -> Json<StateSnapshot> {
    Json(state.core.snapshot())
}
