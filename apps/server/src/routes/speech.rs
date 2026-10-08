//! Speech: speaking a stored text, the audio devices and their test.

use std::sync::Arc;

use app_core::api::{AudioDevices, AudioTestReport, AudioTestRequest, SpeakAccepted, SpeakRequest};
use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::error::ApiError;
use crate::json::ApiJson;

/// `POST /api/tts/speak`. The request names a stored text; it never carries one.
pub async fn speak(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<SpeakRequest>,
) -> Result<Json<SpeakAccepted>, ApiError> {
    Ok(Json(state.core.speak(request).await?))
}

/// `GET /api/audio/devices`
pub async fn devices(State(state): State<Arc<AppState>>) -> Result<Json<AudioDevices>, ApiError> {
    Ok(Json(state.core.audio_devices().await?))
}

/// `POST /api/audio/test`. Runs for the length of the test; if the browser gives
/// up, this future is dropped and the devices are released.
pub async fn test(
    State(state): State<Arc<AppState>>,
    ApiJson(request): ApiJson<AudioTestRequest>,
) -> Result<Json<AudioTestReport>, ApiError> {
    Ok(Json(state.core.audio_test(request).await?))
}
