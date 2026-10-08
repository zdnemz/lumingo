#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Provider profiles: sources, saving, activation, the key rule and the test.

mod common;

use app_core::AppCore;
use app_core::api::{
    ErrorCode, ProbeFailureKind, ProviderProtocol, ProviderSource, SaveProviderRequest, SecretText,
    ServerEvent,
};
use app_core::error::CoreError;
use common::fake_provider::FakeProvider;
use common::{config, test_core, test_core_with_env};

const KEY: &str = "sk-test-SECRETMATERIAL-9f8e7d6c";
const KEY_LAST4: &str = "7d6c";

fn save_request(name: &str, base_url: &str) -> SaveProviderRequest {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "protocol": "openai_chat",
        "base_url": base_url,
        "model": "gpt-test",
        "api_key": KEY,
    }))
    .expect("request")
}

fn env_vars() -> [(&'static str, &'static str); 4] {
    [
        ("TUTOR_LLM_PROTOCOL", "openai_chat"),
        ("TUTOR_LLM_BASE_URL", "https://api.example.test/v1"),
        ("TUTOR_LLM_MODEL", "env-model"),
        ("TUTOR_LLM_API_KEY", "sk-env-ENVSECRET-abcdef123456"),
    ]
}

/// Everything the program says about providers, as one string.
fn everything_said(core: &AppCore) -> String {
    serde_json::to_string(&(core.list_providers(), core.snapshot())).expect("json")
}

#[tokio::test]
async fn with_no_source_there_are_no_providers() {
    let t = test_core().await;
    let list = t.core.list_providers();
    assert!(list.providers.is_empty() && list.active_id.is_none() && list.problems.is_empty());
    assert!(t.core.snapshot().provider.is_none());
    let error = t.core.llm_client().await.err().expect("no client");
    assert!(
        matches!(error, CoreError::ProviderNotConfigured),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_env_profile_is_listed_first_active_and_read_only() {
    let t = test_core_with_env(&env_vars()).await;
    let list = t.core.list_providers();
    assert_eq!(list.providers.len(), 1);
    let env = &list.providers[0];
    assert_eq!(env.name, "env");
    assert_eq!(env.source, ProviderSource::Env);
    assert_eq!(env.protocol, ProviderProtocol::OpenAiChat);
    assert!(env.has_key && env.is_active);
    assert_eq!(env.key_last4.as_deref(), Some("3456"));
    assert_eq!(list.active_id, Some(env.id));
    assert_eq!(t.core.snapshot().provider.as_ref(), Some(env));
    assert!(!everything_said(&t.core).contains("ENVSECRET"));

    let error = t.core.delete_provider(env.id).await.unwrap_err();
    assert!(matches!(error, CoreError::ReadOnly(_)), "{error:?}");
    let error = t
        .core
        .save_provider(save_request("env", "https://x.example.test"))
        .await
        .unwrap_err();
    assert!(matches!(error, CoreError::ReadOnly(_)), "{error:?}");
    // The env profile can be activated like any other, and a client can be built.
    assert!(t.core.llm_client().await.is_ok());
}

#[tokio::test]
async fn an_incomplete_environment_is_reported_as_a_problem_and_the_core_still_starts() {
    let t = test_core_with_env(&[("TUTOR_LLM_MODEL", "only-a-model")]).await;
    let list = t.core.list_providers();
    assert!(list.providers.is_empty());
    assert_eq!(list.problems.len(), 1);
    assert!(
        list.problems[0].contains("TUTOR_LLM_PROTOCOL"),
        "{:?}",
        list.problems
    );
}

#[tokio::test]
async fn a_saved_profile_keeps_its_key_in_the_file_and_nowhere_else() {
    let t = test_core().await;
    let mut events = t.core.events().subscribe();
    let info = t
        .core
        .save_provider(save_request("mine", "https://api.example.test/v1"))
        .await
        .unwrap();
    assert_eq!(info.name, "mine");
    assert!(info.has_key);
    assert_eq!(info.key_last4.as_deref(), Some(KEY_LAST4));
    assert_eq!(info.source, ProviderSource::File);
    assert!(info.is_active, "the first profile becomes active");

    // The key is in the plain-text file, as the settings screen says ...
    let file = std::fs::read_to_string(t.core.config().providers_path()).unwrap();
    assert!(file.contains(KEY));
    // ... and in no answer the core gives.
    assert!(!everything_said(&t.core).contains("SECRETMATERIAL"));
    assert!(!format!("{info:?}").contains("SECRETMATERIAL"));

    // ... and not in the database file.
    t.core.database().compact().await.unwrap();
    let db_bytes = std::fs::read(t.core.database().path()).unwrap();
    assert!(!db_bytes.windows(14).any(|w| w == b"SECRETMATERIAL"));

    match events.recv().await.unwrap() {
        ServerEvent::ProviderStatus { provider, .. } => assert_eq!(provider.unwrap().name, "mine"),
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn providers_file_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let t = test_core().await;
    t.core
        .save_provider(save_request("mine", "https://api.example.test/v1"))
        .await
        .unwrap();
    let mode = std::fs::metadata(t.core.config().providers_path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[tokio::test]
async fn saving_again_without_a_key_keeps_it_and_clear_key_removes_it() {
    let t = test_core().await;
    t.core
        .save_provider(save_request("mine", "https://api.example.test/v1"))
        .await
        .unwrap();

    let mut again = save_request("mine", "https://api.example.test/v1");
    again.api_key = None;
    again.model = "gpt-test-2".to_owned();
    let kept = t.core.save_provider(again.clone()).await.unwrap();
    assert!(kept.has_key);
    assert_eq!(kept.key_last4.as_deref(), Some(KEY_LAST4));
    assert_eq!(kept.model, "gpt-test-2");
    assert!(kept.is_active, "the active mark follows the edited profile");
    assert_eq!(t.core.list_providers().providers.len(), 1);

    again.clear_key = Some(true);
    let cleared = t.core.save_provider(again).await.unwrap();
    assert!(!cleared.has_key && cleared.key_last4.is_none());
    let file = std::fs::read_to_string(t.core.config().providers_path()).unwrap();
    assert!(!file.contains(KEY));
}

#[tokio::test]
async fn activating_and_deleting_keep_exactly_one_profile_active() {
    let t = test_core().await;
    let a = t
        .core
        .save_provider(save_request("a", "https://a.example.test/v1"))
        .await
        .unwrap();
    let b = t
        .core
        .save_provider(save_request("b", "https://b.example.test/v1"))
        .await
        .unwrap();
    assert!(
        a.is_active && !b.is_active,
        "only the first one is made active"
    );

    let activated = t.core.activate_provider(b.id).await.unwrap();
    assert!(activated.is_active);
    let list = t.core.list_providers();
    assert_eq!(list.providers.iter().filter(|p| p.is_active).count(), 1);
    assert_eq!(list.active_id, Some(b.id));
    assert_eq!(t.core.snapshot().provider.unwrap().name, "b");

    // Deleting the active one hands the mark to the one that remains.
    let list = t.core.delete_provider(b.id).await.unwrap();
    assert_eq!(list.providers.len(), 1);
    assert_eq!(list.providers[0].name, "a");
    assert!(list.providers[0].is_active);

    let list = t.core.delete_provider(list.providers[0].id).await.unwrap();
    assert!(list.providers.is_empty() && list.active_id.is_none());
    assert!(t.core.snapshot().provider.is_none());

    let missing = t.core.delete_provider(12345).await.unwrap_err();
    assert!(matches!(missing, CoreError::NotFound { .. }), "{missing:?}");
    let missing = t.core.activate_provider(12345).await.unwrap_err();
    assert!(matches!(missing, CoreError::NotFound { .. }), "{missing:?}");
}

#[tokio::test]
async fn invalid_profiles_are_refused_without_echoing_the_key() {
    let t = test_core().await;
    let cases = [
        (
            "plain http to a remote host",
            "http://remote.example.test/v1",
            "mine",
        ),
        (
            "a URL with a password",
            "https://user:pw@api.example.test/v1",
            "mine",
        ),
        ("not a URL", "banana", "mine"),
        ("an empty name", "https://api.example.test/v1", "  "),
    ];
    for (label, url, name) in cases {
        let error = t
            .core
            .save_provider(save_request(name, url))
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidInput, "{label}: {error:?}");
        let body = serde_json::to_string(&error.body()).unwrap();
        assert!(!body.contains("SECRETMATERIAL"), "{label}: {body}");
    }
    assert!(t.core.list_providers().providers.is_empty());
    assert!(!t.core.config().providers_path().exists());
}

#[tokio::test]
async fn profiles_and_the_active_mark_survive_a_restart() {
    let t = test_core().await;
    t.core
        .save_provider(save_request("a", "https://a.example.test/v1"))
        .await
        .unwrap();
    let mut b = save_request("b", "https://b.example.test/v1");
    b.make_active = Some(true);
    t.core.save_provider(b).await.unwrap();
    t.core.close().await.unwrap();

    let again = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();
    let list = again.list_providers();
    assert_eq!(list.providers.len(), 2);
    assert_eq!(again.snapshot().provider.unwrap().name, "b");
    assert!(list.providers.iter().all(|p| p.has_key));
}

#[tokio::test]
async fn an_unreadable_providers_file_is_reported_and_never_overwritten() {
    let t = test_core().await;
    t.core.close().await.unwrap();
    let path = t.core.config().providers_path();
    let broken = "[[profile]]\nname = \"x\"\nprotocol = \"nonsense\"\n";
    std::fs::write(&path, broken).unwrap();

    let again = AppCore::open(config(&t.dir, &t.clock, &env_vars()))
        .await
        .unwrap();
    let list = again.list_providers();
    assert_eq!(list.problems.len(), 1, "{:?}", list.problems);
    assert!(list.problems[0].starts_with("providers file"));
    // The environment profile still works.
    assert_eq!(list.providers.len(), 1);
    assert_eq!(list.providers[0].name, "env");

    let error = again
        .save_provider(save_request("mine", "https://api.example.test/v1"))
        .await
        .unwrap_err();
    assert!(matches!(error, CoreError::Conflict(_)), "{error:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
}

#[tokio::test]
async fn the_test_runs_the_probe_and_stores_what_it_found() {
    let provider = FakeProvider::start(KEY).await;
    let t = test_core().await;
    let mut request = save_request("local", &provider.base_url);
    request.make_active = Some(true);
    let info = t.core.save_provider(request).await.unwrap();
    assert!(info.capabilities.is_none() && info.probed_at.is_none());
    let mut events = t.core.events().subscribe();

    let report = t.core.test_provider(info.id).await.unwrap();
    assert!(report.ok, "{report:?}");
    assert_eq!(report.provider_id, info.id);
    let caps = report.capabilities.expect("capabilities");
    assert!(caps.auth_ok && caps.stream_ok);
    assert_eq!(caps.structured_level, Some(1));
    assert!(caps.ttft_ms.is_some());
    assert!(
        caps.contracts_ok.is_empty(),
        "the fake provider answers the contract steps with plain text"
    );
    assert!(provider.request_count() >= 3);

    // The result is stored, shown in the list and announced.
    let stored = t.core.list_providers().providers.remove(0);
    assert_eq!(stored.capabilities.as_ref(), Some(&caps));
    assert!(stored.probed_at.is_some());
    match events.recv().await.unwrap() {
        ServerEvent::ProviderStatus { provider, .. } => {
            assert_eq!(provider.unwrap().capabilities, Some(caps));
        }
        other => panic!("{other:?}"),
    }
    assert!(!everything_said(&t.core).contains("SECRETMATERIAL"));

    // After a restart the stored capabilities are still there.
    t.core.close().await.unwrap();
    let again = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();
    assert!(again.list_providers().providers[0].capabilities.is_some());
}

#[tokio::test]
async fn a_wrong_key_is_a_normal_answer_with_ok_false() {
    let provider = FakeProvider::start("a-different-key-entirely").await;
    let t = test_core().await;
    let info = t
        .core
        .save_provider(save_request("local", &provider.base_url))
        .await
        .unwrap();
    let report = t.core.test_provider(info.id).await.unwrap();
    assert!(!report.ok && report.capabilities.is_none());
    let failure = report.failure.expect("failure");
    assert_eq!(failure.kind, ProbeFailureKind::Auth);
    assert!(
        !failure.message.contains("SECRETMATERIAL"),
        "{}",
        failure.message
    );
    assert!(t.core.list_providers().providers[0].capabilities.is_none());
}

#[tokio::test]
async fn an_unreachable_provider_is_reported_as_a_network_failure() {
    let t = test_core().await;
    // Nothing listens on this loopback port.
    let info = t
        .core
        .save_provider(save_request("down", "http://127.0.0.1:9"))
        .await
        .unwrap();
    let report = t.core.test_provider(info.id).await.unwrap();
    assert!(!report.ok);
    assert!(
        matches!(
            report.failure.expect("failure").kind,
            ProbeFailureKind::Network | ProbeFailureKind::Timeout
        ),
        "unreachable"
    );
}

#[tokio::test]
async fn testing_an_unknown_profile_is_not_found() {
    let t = test_core().await;
    let error = t.core.test_provider(99).await.unwrap_err();
    assert!(matches!(error, CoreError::NotFound { .. }), "{error:?}");
}

#[tokio::test]
async fn a_second_test_while_one_runs_is_refused_as_busy() {
    let provider = FakeProvider::start(KEY).await;
    let t = test_core().await;
    let info = t
        .core
        .save_provider(save_request("local", &provider.base_url))
        .await
        .unwrap();
    // Hold the gate by starting a test and polling it once before the second.
    let first = t.core.test_provider(info.id);
    tokio::pin!(first);
    let second = async {
        // Let the first future take the gate.
        tokio::task::yield_now().await;
        t.core.test_provider(info.id).await
    };
    let (first_result, second_result) = tokio::join!(first, second);
    assert!(first_result.unwrap().ok);
    assert!(
        matches!(second_result, Err(CoreError::Busy)),
        "{second_result:?}"
    );
}

#[tokio::test]
async fn the_active_client_is_built_from_the_active_profile() {
    let provider = FakeProvider::start(KEY).await;
    let t = test_core().await;
    let info = t
        .core
        .save_provider(save_request("local", &provider.base_url))
        .await
        .unwrap();
    t.core.test_provider(info.id).await.unwrap();
    let client = t.core.llm_client().await.unwrap();
    // The capabilities of the stored test are applied, so the tutor does not re-probe.
    assert_eq!(
        client.capabilities().structured_level.map(|l| l.as_u8()),
        Some(1)
    );
}

#[tokio::test]
async fn shutdown_cancels_a_running_test() {
    let provider = FakeProvider::start(KEY).await;
    let t = test_core().await;
    let info = t
        .core
        .save_provider(save_request("local", &provider.base_url))
        .await
        .unwrap();
    t.core.request_shutdown();
    let error = t.core.test_provider(info.id).await.unwrap_err();
    assert!(matches!(error, CoreError::ShuttingDown), "{error:?}");
}

#[test]
fn a_secret_prints_nothing() {
    let secret: SecretText = serde_json::from_str("\"sk-abc-123456\"").unwrap();
    assert_eq!(secret.expose(), "sk-abc-123456");
    assert!(!format!("{secret:?}").contains("123456"));
}
