//! The route groups of `GET/POST/PUT/DELETE /api/...`. Each handler parses its
//! input, calls one method of `app-core` and serialises the answer. A route whose
//! engine is not in this build answers a typed `not_available` (HTTP 501) that
//! says what is missing; `app-core` lists the missing parts in the snapshot's
//! `unavailable` list. An unknown path under `/api` is a JSON 404.

mod data;
mod diagnostics;
mod free;
mod game;
mod models;
mod progress;
mod providers;
mod sessions;
mod settings;
mod speech;
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
        // Sessions and turns
        .route("/api/sessions", post(sessions::start))
        .route("/api/sessions/{id}/stop", post(sessions::stop))
        .route("/api/sessions/{id}/pause", post(sessions::pause))
        .route("/api/sessions/{id}/resume", post(sessions::resume))
        .route("/api/sessions/{id}/text", post(sessions::text))
        .route("/api/sessions/{id}/turns/{seq}/edit", post(sessions::edit))
        .route(
            "/api/sessions/{id}/push-to-talk",
            post(sessions::push_to_talk),
        )
        .route("/api/tutor/stop-speaking", post(sessions::stop_speaking))
        // Activities
        .route(
            "/api/sessions/{id}/next-activity",
            get(sessions::next_activity),
        )
        .route("/api/activities/submit", post(sessions::submit_activity))
        // Free modes
        .route("/api/writing/{session}/drafts", post(free::submit_draft))
        .route("/api/reading/generate", post(free::generate))
        .route("/api/reading/{session}/answers", post(free::answer))
        // Speech
        .route("/api/tts/speak", post(speech::speak))
        .route("/api/audio/devices", get(speech::devices))
        .route("/api/audio/test", post(speech::test))
        // Models
        .route("/api/models", get(models::list))
        .route("/api/models/{id}/download", post(models::download))
        .route("/api/models/{id}/cancel", post(models::cancel))
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
        .route("/api/inspector", get(models::inspector))
}
