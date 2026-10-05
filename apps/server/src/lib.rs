//! The Lumingo local server: a loopback HTTP API, one WebSocket, and the
//! embedded web UI. Handlers here parse, check, call `app-core` and serialise.
//! Tutoring, scoring and prompt logic never live in this crate, and the request
//! and response types are the ones `app-core` defines.

pub mod assets;
pub mod error;
mod json;
mod routes;
pub mod security;
mod ws;

use std::sync::Arc;
use std::time::Duration;

use app_core::AppCore;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router, middleware};

use crate::error::ApiError;
use crate::security::Guard;

/// Largest request body any route accepts. Every request is a small JSON
/// document (a profile, a setting, an id), so a large body is a mistake or an
/// attack and is refused with HTTP 413 before it is read into memory.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// Shared by every handler.
#[derive(Debug)]
pub struct AppState {
    pub core: Arc<AppCore>,
    pub guard: Arc<Guard>,
}

impl AppState {
    pub fn new(core: Arc<AppCore>, guard: Arc<Guard>) -> Self {
        Self { core, guard }
    }
}

/// How often the spike heartbeat event is published.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(1);

/// Builds the whole application. The guard wraps every route, including the UI.
pub fn build_router(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .merge(routes::api())
        .route("/ws", get(ws::events))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES));
    if state.core.dev_mode() {
        router = router.route("/dev/session", get(dev_session));
    }
    router
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state.guard),
            security::guard,
        ))
        .with_state(state)
}

/// An unknown path under `/api` answers in JSON like every other API error; any
/// other unknown path is a page of the UI.
async fn fallback(req: Request) -> Response {
    let path = req.uri().path();
    if path == "/api" || path.starts_with("/api/") {
        return ApiError::not_found().into_response();
    }
    assets::serve_ui(req).await
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

/// Publishes a heartbeat once a second until the core is asked to shut down.
pub async fn run_heartbeat(state: Arc<AppState>) {
    let shutdown = state.core.shutdown_token();
    let mut ticker = tokio::time::interval(HEARTBEAT_EVERY);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            _ = ticker.tick() => state.core.publish_heartbeat(),
        }
    }
}
