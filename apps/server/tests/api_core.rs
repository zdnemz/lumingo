#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The State, Settings and Curriculum route groups, and the error format.

mod common;

use common::{Options, Req, body_text, harness, harness_with, send};
use serde_json::json;

fn settings_body() -> serde_json::Value {
    json!({
        "display_name": "Sari",
        "ui_language": "en",
        "l1": "jv",
        "l1_help_mode": "off",
        "adaptive_timing": "never_wait",
        "keep_recordings": true
    })
}

#[tokio::test]
async fn state_is_a_full_snapshot() {
    let h = harness(false).await;
    let (status, state) = h.get_json("/api/state").await;
    assert_eq!(status, 200);
    assert_eq!(state["server_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(state["dev_mode"], false);
    assert_eq!(state["settings"]["display_name"], "Learner");
    assert_eq!(state["provider"], serde_json::Value::Null);
    assert_eq!(state["hardware"]["logical_cores"], 8);
    assert_eq!(state["hardware"]["meets_minimum"], true);
    assert_eq!(
        state["unavailable"],
        json!(["sessions", "activities", "free_modes", "speech", "models"])
    );
}

#[tokio::test]
async fn settings_can_be_read_replaced_and_read_back() {
    let h = harness(false).await;
    let (status, before) = h.get_json("/api/settings").await;
    assert_eq!(status, 200);
    assert_eq!(before["keep_recordings"], false);
    assert_eq!(before["adaptive_timing"], "auto");

    let (status, echoed) = h.call(Req::put("/api/settings", settings_body())).await;
    assert_eq!(status, 200);
    assert_eq!(echoed, settings_body());
    let (_, after) = h.get_json("/api/settings").await;
    assert_eq!(after, settings_body());
    let (_, state) = h.get_json("/api/state").await;
    assert_eq!(state["settings"], settings_body());
}

#[tokio::test]
async fn bad_settings_are_a_400_with_a_code_and_change_nothing() {
    let h = harness(false).await;
    let mut wrong_language = settings_body();
    wrong_language["l1"] = json!("indonesian");
    let mut unknown_choice = settings_body();
    unknown_choice["l1_help_mode"] = json!("sometimes");
    let mut missing_field = settings_body();
    missing_field
        .as_object_mut()
        .unwrap()
        .remove("display_name");

    for (label, body) in [
        ("a language that is not a code", wrong_language),
        ("an unknown choice", unknown_choice),
        ("a missing field", missing_field),
    ] {
        let (status, error) = h.call(Req::put("/api/settings", body)).await;
        assert_eq!(status, 400, "{label}");
        assert_eq!(error["error"], "invalid_input", "{label}");
        assert!(error["message"].as_str().is_some_and(|m| !m.is_empty()));
    }

    let mut broken = Req::put("/api/settings", json!({}));
    broken.body = Some("{ not json".to_owned());
    let (status, error) = h.call(broken).await;
    assert_eq!(status, 400);
    assert_eq!(error["error"], "invalid_input");
    assert_eq!(h.core.settings().display_name, "Learner");
}

#[tokio::test]
async fn a_body_above_the_limit_is_refused_with_413_before_it_is_read() {
    let h = harness(false).await;
    let mut huge = settings_body();
    huge["display_name"] = json!("x".repeat(tutor_server::MAX_BODY_BYTES + 1));
    let (status, error) = h.call(Req::put("/api/settings", huge)).await;
    assert_eq!(status, 413);
    assert_eq!(error["error"], "invalid_input");
    assert_eq!(h.core.settings().display_name, "Learner");
}

#[tokio::test]
async fn errors_after_shutdown_has_begun_say_so() {
    let h = harness(false).await;
    h.core.request_shutdown();
    let (status, error) = h.call(Req::put("/api/settings", settings_body())).await;
    assert_eq!(status, 503);
    assert_eq!(error["error"], "shutting_down");
    // Reads still answer.
    let (status, _) = h.get_json("/api/settings").await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn units_are_listed_and_read_whole() {
    let h = harness_with(Options {
        with_unit: true,
        ..Options::default()
    })
    .await;
    let (status, list) = h.get_json("/api/units").await;
    assert_eq!(status, 200);
    assert_eq!(list["units"].as_array().unwrap().len(), 1);
    assert_eq!(list["units"][0]["id"], "a1-u01");
    assert_eq!(list["units"][0]["level"], "A1");
    assert_eq!(list["units"][0]["title"]["en"], "Hello! Nice to meet you");
    assert_eq!(list["issues"], json!([]));
    assert!(
        list["content_version"]
            .as_str()
            .unwrap()
            .starts_with("manifest-")
    );

    let (status, detail) = h.get_json("/api/units/a1-u01").await;
    assert_eq!(status, 200);
    assert_eq!(detail["unit"]["id"], "a1-u01");
    assert!(
        detail["unit"]["activities"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    );

    let (status, error) = h.get_json("/api/units/a9-u99").await;
    assert_eq!(status, 404);
    assert_eq!(error["error"], "not_found");
}

#[tokio::test]
async fn a_missing_unit_folder_is_reported_not_fatal() {
    let h = harness(false).await;
    let (status, list) = h.get_json("/api/units").await;
    assert_eq!(status, 200);
    assert_eq!(list["units"], json!([]));
    assert_eq!(list["issues"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn groups_that_are_not_built_in_are_a_json_404_not_a_page() {
    let h = harness(false).await;
    for req in [
        Req::post("/api/sessions", json!({"kind": "lesson"})),
        Req::post("/api/activities/submit", json!({})),
        Req::post("/api/tts/speak", json!({})),
    ] {
        let uri = req.uri.clone();
        let response = send(&h.router, req.authed(&h.cookie)).await;
        assert_eq!(response.status(), 404, "{uri}");
        let text = body_text(response).await;
        let body: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(body["error"], "not_found", "{uri}");
    }
}

#[tokio::test]
async fn every_api_answer_is_marked_not_to_be_cached() {
    let h = harness(false).await;
    for uri in ["/api/state", "/api/settings", "/api/units", "/api/nope"] {
        let response = send(&h.router, Req::get(uri).authed(&h.cookie)).await;
        assert_eq!(
            response.headers()[axum::http::header::CACHE_CONTROL],
            "no-store",
            "{uri}"
        );
    }
}
