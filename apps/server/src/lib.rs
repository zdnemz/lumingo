//! The Lumingo local server: a loopback HTTP API, one WebSocket, and the
//! embedded web UI. Handlers here parse, check, and serialise. Tutoring,
//! scoring, and prompt logic never live in this crate.

pub mod assets;
pub mod events;
pub mod security;
pub mod types;
mod ws;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router, middleware};
use tokio_util::sync::CancellationToken;

use crate::events::EventHub;
use crate::security::Guard;
use crate::types::StateSnapshot;

/// Shared by every handler.
#[derive(Debug)]
pub struct AppState {
    pub hub: EventHub,
    pub guard: Arc<Guard>,
    pub shutdown: CancellationToken,
    pub dev_mode: bool,
}

/// How often the spike heartbeat event is published.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(1);

/// Builds the whole application. The guard wraps every route, including the UI.
pub fn build_router(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .route("/api/state", get(get_state))
        .route("/ws", get(ws::events));
    if state.dev_mode {
        router = router.route("/dev/session", get(dev_session));
    }
    router
        .fallback(assets::serve_ui)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state.guard),
            security::guard,
        ))
        .with_state(state)
}

async fn get_state(State(state): State<Arc<AppState>>) -> Json<StateSnapshot> {
    Json(state.hub.state())
}

/// Development mode only. `next dev` serves its own HTML, so the UI asks here
/// for the session cookie that the server normally sets with the HTML page.
async fn dev_session(State(state): State<Arc<AppState>>) -> Response {
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    if let Ok(cookie) = HeaderValue::from_str(&state.guard.set_cookie_value()) {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

/// Publishes a heartbeat once a second until `shutdown` is cancelled.
pub async fn run_heartbeat(state: Arc<AppState>) {
    let mut ticker = tokio::time::interval(HEARTBEAT_EVERY);
    loop {
        tokio::select! {
            () = state.shutdown.cancelled() => return,
            _ = ticker.tick() => state.hub.publish_heartbeat(),
        }
    }
}
