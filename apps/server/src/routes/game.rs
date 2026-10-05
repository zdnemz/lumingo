use std::sync::Arc;

use app_core::api::{EquipRequest, GameState};
use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::error::ApiError;
use crate::json::ApiJson;

/// `GET /api/game`
pub async fn get(State(state): State<Arc<AppState>>) -> Result<Json<GameState>, ApiError> {
    Ok(Json(state.core.game_state().await?))
}

/// `POST /api/game/equip`. There is deliberately no route that awards XP: XP
/// comes from finished practice inside the core, never from a request.
pub async fn equip(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<EquipRequest>,
) -> Result<Json<GameState>, ApiError> {
    Ok(Json(state.core.equip(request).await?))
}
