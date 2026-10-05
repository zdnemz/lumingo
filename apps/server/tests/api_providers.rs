#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The Providers route group, including the key rule: no answer of any route
//! ever holds a stored key.

mod common;
#[path = "../../../crates/app-core/tests/common/fake_provider.rs"]
mod fake_provider;

use common::{Harness, Options, Req, harness, harness_with};
use fake_provider::FakeProvider;
use serde_json::{Value, json};

const KEY: &str = "sk-test-SECRETMATERIAL-9f8e7d6c";

fn provider_body(name: &str, base_url: &str) -> Value {
    json!({
        "name": name,
        "protocol": "openai_chat",
        "base_url": base_url,
        "model": "gpt-test",
        "api_key": KEY
    })
}

/// Every GET route, read with the cookie, as one string.
async fn everything_readable(h: &Harness) -> String {
    let mut all = String::new();
    for uri in [
        "/api/state",
        "/api/providers",
        "/api/settings",
        "/api/progress",
        "/api/game",
        "/api/diagnostics",
        "/api/export",
        "/api/units",
    ] {
        let (_, body) = h.get_json(uri).await;
        all.push_str(&body.to_string());
    }
    all
}

#[tokio::test]
async fn a_provider_is_saved_listed_activated_and_deleted() {
    let h = harness(false).await;
    let (status, list) = h.get_json("/api/providers").await;
    assert_eq!(status, 200);
    assert_eq!(list["providers"], json!([]));
    assert_eq!(list["active_id"], Value::Null);

    let (status, a) = h
        .call(Req::post(
            "/api/providers",
            provider_body("a", "https://a.example.test/v1"),
        ))
        .await;
    assert_eq!(status, 200);
    assert_eq!(a["name"], "a");
    assert_eq!(a["has_key"], true);
    assert_eq!(a["key_last4"], "7d6c");
    assert_eq!(a["source"], "file");
    assert_eq!(a["is_active"], true);
    assert!(a.get("api_key").is_none());

    let (_, b) = h
        .call(Req::post(
            "/api/providers",
            provider_body("b", "https://b.example.test/v1"),
        ))
        .await;
    assert_eq!(b["is_active"], false);

    let (status, activated) = h
        .call(Req::post(
            &format!("/api/providers/{}/activate", b["id"]),
            json!({}),
        ))
        .await;
    assert_eq!(status, 200);
    assert_eq!(activated["is_active"], true);
    let (_, state) = h.get_json("/api/state").await;
    assert_eq!(state["provider"]["name"], "b");

    let (status, list) = h
        .call(Req::delete(&format!("/api/providers/{}", b["id"])))
        .await;
    assert_eq!(status, 200);
    assert_eq!(list["providers"].as_array().unwrap().len(), 1);
    assert_eq!(list["providers"][0]["name"], "a");
    assert_eq!(list["providers"][0]["is_active"], true);
}

#[tokio::test]
async fn no_route_ever_returns_a_stored_key() {
    let h = harness_with(Options {
        env: vec![
            ("TUTOR_LLM_PROTOCOL", "openai_chat"),
            ("TUTOR_LLM_BASE_URL", "https://env.example.test/v1"),
            ("TUTOR_LLM_MODEL", "env-model"),
            ("TUTOR_LLM_API_KEY", "sk-env-ENVSECRET-zz99yy88"),
        ],
        ..Options::default()
    })
    .await;
    let (status, saved) = h
        .call(Req::post(
            "/api/providers",
            provider_body("mine", "https://api.example.test/v1"),
        ))
        .await;
    assert_eq!(status, 200);
    let direct = saved.to_string();
    assert!(!direct.contains("SECRETMATERIAL"));

    let all = everything_readable(&h).await;
    // Only the last four characters may appear, and only as `key_last4`.
    for secret in [
        KEY,
        "SECRETMATERIAL",
        "9f8e7d6c",
        "sk-env-ENVSECRET-zz99yy88",
        "ENVSECRET",
        "zz99yy88",
    ] {
        assert!(!all.contains(secret), "{secret} appears in an answer");
    }
    assert!(all.contains("\"key_last4\":\"7d6c\""));
    assert!(all.contains("\"key_last4\":\"yy88\""));

    // A failed request does not echo the body either.
    let (status, error) = h
        .call(Req::post(
            "/api/providers",
            provider_body("bad", "http://remote.example.test/v1"),
        ))
        .await;
    assert_eq!(status, 400);
    assert!(!error.to_string().contains("SECRETMATERIAL"));
}

#[tokio::test]
async fn the_env_profile_is_read_only() {
    let h = harness_with(Options {
        env: vec![
            ("TUTOR_LLM_PROTOCOL", "anthropic_messages"),
            ("TUTOR_LLM_BASE_URL", "https://env.example.test"),
            ("TUTOR_LLM_MODEL", "env-model"),
        ],
        ..Options::default()
    })
    .await;
    let (_, list) = h.get_json("/api/providers").await;
    let env = &list["providers"][0];
    assert_eq!(env["name"], "env");
    assert_eq!(env["source"], "env");
    assert_eq!(env["protocol"], "anthropic_messages");
    assert_eq!(env["has_key"], false);
    assert_eq!(env["is_active"], true);

    let (status, error) = h
        .call(Req::delete(&format!("/api/providers/{}", env["id"])))
        .await;
    assert_eq!(status, 409);
    assert_eq!(error["error"], "read_only");
    let (status, error) = h
        .call(Req::post(
            "/api/providers",
            provider_body("env", "https://x.example.test"),
        ))
        .await;
    assert_eq!(status, 409);
    assert_eq!(error["error"], "read_only");
}

#[tokio::test]
async fn bad_provider_requests_are_400s_and_unknown_ids_are_404s() {
    let h = harness(false).await;
    let (status, error) = h
        .call(Req::post(
            "/api/providers",
            provider_body("x", "http://remote.example.test/v1"),
        ))
        .await;
    assert_eq!(
        (status, error["error"].as_str()),
        (400, Some("invalid_input"))
    );
    let (status, error) = h
        .call(Req::post("/api/providers", json!({"name": "x"})))
        .await;
    assert_eq!(
        (status, error["error"].as_str()),
        (400, Some("invalid_input"))
    );
    let (status, _) = h.call(Req::delete("/api/providers/not-a-number")).await;
    assert_eq!(status, 400);
    for req in [
        Req::delete("/api/providers/999"),
        Req::post("/api/providers/999/test", json!({})),
        Req::post("/api/providers/999/activate", json!({})),
    ] {
        let uri = req.uri.clone();
        let (status, error) = h.call(req).await;
        assert_eq!(
            (status, error["error"].as_str()),
            (404, Some("not_found")),
            "{uri}"
        );
    }
    assert_eq!(h.core.list_providers().providers.len(), 0);
}

#[tokio::test]
async fn the_test_route_runs_the_probe_and_the_result_shows_in_the_list() {
    let provider = FakeProvider::start(KEY).await;
    let h = harness(false).await;
    let (_, saved) = h
        .call(Req::post(
            "/api/providers",
            provider_body("local", &provider.base_url),
        ))
        .await;
    let id = saved["id"].as_i64().unwrap();

    let (status, report) = h
        .call(Req::post(&format!("/api/providers/{id}/test"), json!({})))
        .await;
    assert_eq!(status, 200);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["provider_id"], id);
    assert_eq!(report["failure"], Value::Null);
    assert_eq!(report["capabilities"]["auth_ok"], true);
    assert_eq!(report["capabilities"]["stream_ok"], true);
    assert_eq!(report["capabilities"]["structured_level"], 1);

    let (_, list) = h.get_json("/api/providers").await;
    assert_eq!(list["providers"][0]["capabilities"]["structured_level"], 1);
    assert!(list["providers"][0]["probed_at"].is_string());
    let (_, diagnostics) = h.get_json("/api/diagnostics").await;
    assert_eq!(
        diagnostics["provider"]["capabilities"]["structured_level"],
        1
    );
}

#[tokio::test]
async fn a_failed_test_is_a_200_with_ok_false_and_a_reason() {
    let provider = FakeProvider::start("some-other-key").await;
    let h = harness(false).await;
    let (_, saved) = h
        .call(Req::post(
            "/api/providers",
            provider_body("local", &provider.base_url),
        ))
        .await;
    let id = saved["id"].as_i64().unwrap();
    let (status, report) = h
        .call(Req::post(&format!("/api/providers/{id}/test"), json!({})))
        .await;
    assert_eq!(status, 200);
    assert_eq!(report["ok"], false);
    assert_eq!(report["failure"]["kind"], "auth");
    assert_eq!(report["capabilities"], Value::Null);
    assert!(!report.to_string().contains("SECRETMATERIAL"));
}
