//! The route groups of `GET/POST/PUT/DELETE /api/...`. Each handler parses its
//! input, calls one method of `app-core` and serialises the answer. The groups
//! of the context pack that `app-core` does not implement yet (sessions, turns,
//! activities, free modes, speech, models) have no route here: an unknown path
//! under `/api` is a JSON 404, and `app-core` reports the missing parts in the
//! snapshot's `unavailable` list.

mod data;
mod diagnostics;
mod game;
mod progress;
mod providers;
mod settings;
mod state;
mod units;

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::AppState;

pub fn api() -> Router<Arc<AppState>> {
    Router::new()
        // State
        .route("/api/state", get(state::get))
        // Curriculum
        .route("/api/units", get(units::list))
        .route("/api/units/{id}", get(units::detail))
        // Providers
        .route("/api/providers", get(providers::list).post(providers::save))
        .route("/api/providers/{id}", delete(providers::remove))
        .route("/api/providers/{id}/test", post(providers::test))
        .route("/api/providers/{id}/activate", post(providers::activate))
        // Progress
        .route("/api/progress", get(progress::overview))
        .route("/api/attempts/{id}/evidence", get(progress::evidence))
        // Game
        .route("/api/game", get(game::get))
        .route("/api/game/equip", post(game::equip))
        // Settings
        .route("/api/settings", get(settings::get).put(settings::put))
        // Data
        .route("/api/sessions/{id}", delete(data::delete_session))
        .route("/api/data", delete(data::delete_all))
        .route("/api/export", get(data::export))
        // Diagnostics
        .route("/api/diagnostics", get(diagnostics::get))
}
