#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Offline behaviour and the allowlist (ROADMAP S3-12).
//!
//! The voice loop runs against a real `llm_client::ProviderClient` here, so the
//! real HTTP code, the real host allowlist and the real timeouts are in play;
//! only the speech engines are fakes. Nothing leaves the machine: the "provider"
//! is a closed loopback port, a listener that never answers, an address that
//! routes nowhere, or a local capture server.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use app_core::voice::{TurnOutcome, VoiceEvent};
use common::capture::{Behaviour, CaptureServer};
use common::voice::{Audio, Options, Rig};
use llm_client::{
    ClientOptions, Limits, LlmClient, ProfileSource, Protocol, ProviderClient, ProviderProfile,
    RetryPolicy,
};
use tutor_engine::Phase;

/// The time the loop is given to get a reply started, both attempts together.
const BUDGET: Duration = Duration::from_millis(1_500);

fn client(base: &str) -> Arc<dyn LlmClient> {
    let profile = ProviderProfile::new(
        "offline-test",
        Protocol::OpenAiChat,
        base,
        "test-model",
        Some("sk-test-0123456789abcdef"),
        ProfileSource::File,
    )
    .expect("a valid profile");
    // The client's own limits are the same size as the loop's budget, so the
    // loop's deadline is the one that decides.
    let limits = Limits {
        connect: BUDGET,
        first_token: Some(BUDGET),
        total: BUDGET * 2,
    };
    let options = ClientOptions {
        tutor: limits,
        background: Limits {
            first_token: None,
            ..limits
        },
        retry: RetryPolicy {
            transport_retries: 1,
            jitter_base: Duration::from_millis(20),
            rate_limit_retries: 0,
            rate_limit_backoff: Duration::from_millis(50),
        },
    };
    Arc::new(ProviderClient::connect(&profile, options, None).expect("a client"))
}

fn text_only(llm: Arc<dyn LlmClient>) -> Options {
    Options {
        audio: Audio::None,
        listen: false,
        tts: false,
        provider_timeout: BUDGET,
        llm: Some(llm),
        ..Options::default()
    }
}

/// Starts a turn against an unreachable provider and returns how long it took
/// to see `ProviderUnavailable`, with the learner's message.
async fn unavailable_after(base: &str) -> (Duration, String, Phase) {
    let mut rig = Rig::start(Vec::new(), text_only(client(base))).await;
    let started = Instant::now();
    rig.handle.open().await.unwrap();
    let events = rig
        .log
        .wait("ProviderUnavailable", |e| {
            e.iter()
                .any(|e| matches!(e, VoiceEvent::ProviderUnavailable { .. }))
        })
        .await;
    let waited = started.elapsed();
    let message = events
        .iter()
        .find_map(|e| match e {
            VoiceEvent::ProviderUnavailable { message } => Some(message.clone()),
            _ => None,
        })
        .unwrap();
    let phase = rig.handle.phase();
    rig.log.turn_ended(1).await;
    rig.finish().await;
    (waited, message, phase)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_loopback_port_is_unavailable_within_the_timeout() {
    // Bind to learn a free port, then close it again.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (waited, message, phase) = unavailable_after(&format!("http://127.0.0.1:{port}/v1")).await;
    assert!(waited < BUDGET, "{waited:?}");
    assert_eq!(phase, Phase::ProviderUnavailable);
    assert!(message.contains("could not be reached"), "{message}");
    assert!(message.contains("resume"), "{message}");
    // The message names the failure's category and nothing else: no key, no address.
    assert!(!message.contains("sk-test"), "{message}");
    assert!(!message.contains(&port.to_string()), "{message}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_black_hole_address_is_unavailable_within_the_timeout() {
    // 10.255.255.1 is a private address nothing answers on. Depending on the
    // machine the connection hangs or is refused at once; either way the loop
    // must report within its budget.
    let (waited, _message, phase) = unavailable_after("https://10.255.255.1:443/v1").await;
    assert!(waited < BUDGET + Duration::from_millis(500), "{waited:?}");
    assert_eq!(phase, Phase::ProviderUnavailable);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_accepts_and_never_answers_is_given_up_after_both_attempts() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let (waited, _message, phase) = unavailable_after(&format!("http://127.0.0.1:{port}/v1")).await;
    holder.abort();
    // Two attempts of half the budget each.
    assert!(waited >= BUDGET - Duration::from_millis(100), "{waited:?}");
    assert!(waited < BUDGET + Duration::from_millis(500), "{waited:?}");
    assert_eq!(phase, Phase::ProviderUnavailable);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_answers_503_twice_ends_the_turn_as_unavailable() {
    let provider = CaptureServer::start(Behaviour::Unavailable).await;
    let (_waited, _message, phase) = unavailable_after(&provider.url("/v1")).await;
    assert_eq!(phase, Phase::ProviderUnavailable);
    // The client retries a 5xx once and the loop retries the turn once.
    assert!(provider.requests().len() >= 2);
}

// ---- the allowlist ---------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_voice_loop_makes_no_request_to_any_host_but_the_configured_provider() {
    let provider = CaptureServer::start(Behaviour::Reply("Hello there. How are you?")).await;
    // A second server on the same machine that nothing is configured to use.
    let other = CaptureServer::start(Behaviour::Reply("never asked")).await;
    let dir = tempfile::tempdir().unwrap();
    let db = storage::Database::open(dir.path().join("lumingo.sqlite"))
        .await
        .unwrap();
    let profile = db
        .profiles()
        .create(&storage::NewProfile {
            display_name: "Test learner".to_owned(),
            ui_language: storage::UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: storage::L1HelpMode::Auto,
            created_at: storage::Timestamp::from_unix_seconds(1_790_000_000).unwrap(),
        })
        .await
        .unwrap();
    let counter = Arc::new(std::sync::atomic::AtomicI64::new(0));
    let recording = app_core::voice::Recording {
        db,
        profile_id: profile.id,
        provider_profile_id: None,
        model: "test-model".to_owned(),
        app_version: "0.0.0".to_owned(),
        link_unit: false,
        clock: Arc::new(move || {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            storage::Timestamp::from_unix_seconds(1_790_000_000 + n).unwrap()
        }),
    };
    let mut rig = Rig::start(
        Vec::new(),
        Options {
            audio: Audio::Playback,
            transcripts: vec!["My name is Dewi."],
            provider_timeout: BUDGET,
            llm: Some(client(&provider.url("/v1"))),
            recording: Some(recording),
            ..Options::default()
        },
    )
    .await;

    // The whole voice path: the tutor opens, the learner speaks, the learner
    // types, and the background analysis runs for each learner turn.
    rig.handle.open().await.unwrap();
    rig.log.turn_ended(1).await;
    rig.say().await;
    let events = rig.log.turn_ended(2).await;
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::TurnEnded {
            turn: 2,
            outcome: TurnOutcome::Replied
        }
    )));
    rig.handle.send_text("I like tea").await.unwrap();
    rig.log.turn_ended(3).await;
    rig.finish().await;

    let seen = provider.requests();
    assert!(
        seen.len() >= 3,
        "the provider heard from the loop: {seen:?}"
    );
    for request in &seen {
        assert_eq!(request.host, provider.authority(), "{request:?}");
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/chat/completions");
    }
    assert!(
        other.requests().is_empty(),
        "a host that is not the provider received {:?}",
        other.requests()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_to_another_host_is_not_followed_by_the_voice_loop() {
    let elsewhere = CaptureServer::start(Behaviour::Reply("stolen")).await;
    let provider =
        CaptureServer::start(Behaviour::Redirect(elsewhere.url("/v1/chat/completions"))).await;
    let (_waited, _message, phase) = unavailable_after(&provider.url("/v1")).await;
    assert_eq!(phase, Phase::ProviderUnavailable);
    assert!(!provider.requests().is_empty());
    assert!(
        elsewhere.requests().is_empty(),
        "the redirect target was contacted: {:?}",
        elsewhere.requests()
    );
}
