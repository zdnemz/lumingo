#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The payload inspector and the key rule, with the real provider client against a
//! provider on the loopback address: the key is sent to the provider and appears
//! nowhere else.

mod common;

use app_core::api::{PayloadOutcomeView, ServerEvent, SessionKind, StopRequest};
use common::fake_provider::FakeProvider;
use common::sessions::{Setup, chat_request, rig};
use serde_json::json;

const KEY: &str = "sk-test-SECRETMATERIAL-9f8e7d6c";
const MIDDLE: &str = "SECRETMATERIAL";

#[tokio::test(flavor = "multi_thread")]
async fn the_inspector_shows_what_was_sent_and_what_came_back_and_the_key_is_nowhere() {
    let provider = FakeProvider::start(KEY).await;
    let r = rig(Setup {
        llm: false,
        ..Setup::default()
    })
    .await;
    let saved = r
        .core()
        .save_provider(
            serde_json::from_value(json!({
                "name": "fake",
                "protocol": "openai_chat",
                "base_url": provider.base_url,
                "model": "gpt-test",
                "api_key": KEY,
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    r.core().activate_provider(saved.id).await.unwrap();
    assert!(
        r.core().inspector().entries.is_empty(),
        "nothing was sent yet"
    );

    let view = r.start(chat_request("introductions")).await;
    assert_eq!(view.kind, SessionKind::TextChat);
    r.events.reply_stored(0).await;
    r.core()
        .send_text(view.id, "I am Dewi and I like tea.".to_owned())
        .await
        .unwrap();
    let mark = 0;
    r.events
        .wait("the second reply", |events| {
            events
                .iter()
                .skip(mark)
                .filter(|e| {
                    matches!(
                        e,
                        ServerEvent::TurnState {
                            tutor_turn_seq: Some(_),
                            ..
                        }
                    )
                })
                .count()
                >= 2
        })
        .await;
    let ended = r
        .core()
        .stop_session(view.id, StopRequest::default())
        .await
        .unwrap();

    let report = r.core().inspector();
    assert_eq!(report.capacity, 50);
    assert_eq!(report.max_body_bytes, 16 * 1024);
    assert!(
        report.entries.len() >= 3,
        "two replies and the analysis: {:#?}",
        report
            .entries
            .iter()
            .map(|e| (&e.endpoint, e.outcome))
            .collect::<Vec<_>>()
    );
    let ids: Vec<u64> = report.entries.iter().map(|e| e.id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "oldest first: {ids:?}");
    let streamed: Vec<_> = report.entries.iter().filter(|e| e.streaming).collect();
    assert_eq!(streamed.len(), 2);
    for entry in &streamed {
        assert!(
            entry.endpoint.ends_with("/chat/completions"),
            "{}",
            entry.endpoint
        );
        assert_eq!(entry.status, Some(200));
        assert_eq!(entry.outcome, PayloadOutcomeView::Ok);
        assert!(
            entry
                .response_body
                .as_deref()
                .unwrap()
                .contains("Hello there.")
        );
        assert!(!entry.request_truncated);
    }
    assert!(
        streamed[1]
            .request_body
            .contains("I am Dewi and I like tea."),
        "the inspector shows what left the machine, learner text included"
    );

    // The key reaches the provider and nothing else. Everything the program says,
    // as text: the inspector, the snapshot, the providers, every event, the export,
    // the diagnostics and the answer to the stop.
    let events = r.events.settled().await;
    let said = serde_json::to_string(&json!({
        "inspector": r.core().inspector(),
        "snapshot": r.core().snapshot(),
        "providers": r.core().list_providers(),
        "events": events,
        "export": r.core().export().await.unwrap(),
        "diagnostics": r.core().diagnostics().await.unwrap(),
        "ended": ended,
    }))
    .unwrap();
    assert!(!said.contains(KEY), "the key is in the output");
    assert!(!said.contains(MIDDLE), "part of the key is in the output");
    assert!(
        !said.to_ascii_lowercase().contains("bearer"),
        "no header is kept"
    );
    assert!(!said.to_ascii_lowercase().contains("authorization"));
    assert!(
        said.contains("9f8e7d6c") || said.contains("7d6c"),
        "the last four are shown"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_key_shows_in_the_inspector_as_an_http_error_without_the_key() {
    let provider = FakeProvider::start("a-different-key").await;
    let r = rig(Setup {
        llm: false,
        ..Setup::default()
    })
    .await;
    let saved = r
        .core()
        .save_provider(
            serde_json::from_value(json!({
                "name": "fake",
                "protocol": "openai_chat",
                "base_url": provider.base_url,
                "model": "gpt-test",
                "api_key": KEY,
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    r.core().activate_provider(saved.id).await.unwrap();
    // The refusal arrives when the opening turn asks for its reply.
    let view = r.start(chat_request("introductions")).await;
    r.events
        .wait("the provider to refuse", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::SessionState {
                        message: Some(_),
                        ..
                    }
                )
            })
        })
        .await;
    let report = r.core().inspector();
    let refused = report
        .entries
        .iter()
        .find(|e| e.status == Some(401))
        .expect("the refused request is shown");
    assert_eq!(refused.outcome, PayloadOutcomeView::HttpError);
    assert!(
        refused
            .response_body
            .as_deref()
            .unwrap()
            .contains("Incorrect API key")
    );
    let said = serde_json::to_string(&(report, r.events.settled().await)).unwrap();
    assert!(!said.contains(KEY) && !said.contains(MIDDLE));
    r.core()
        .stop_session(view.id, StopRequest { cancel: true })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_inspector_keeps_only_the_last_entries_and_counts_what_it_dropped() {
    let provider = FakeProvider::start(KEY).await;
    let r = rig(Setup {
        llm: false,
        ..Setup::default()
    })
    .await;
    let saved = r
        .core()
        .save_provider(
            serde_json::from_value(json!({
                "name": "fake",
                "protocol": "openai_chat",
                "base_url": provider.base_url,
                "model": "gpt-test",
                "api_key": KEY,
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    // Every test of the provider sends requests; sixty of them fill a ring of fifty.
    for _ in 0..16 {
        let _ = r.core().test_provider(saved.id).await;
    }
    let report = r.core().inspector();
    assert_eq!(report.capacity, 50);
    assert!(report.entries.len() <= 50, "{}", report.entries.len());
    assert_eq!(report.entries.len(), 50, "the ring is full");
    assert!(report.dropped > 0, "and it counted what it let go");
    assert_eq!(
        report.entries.last().unwrap().id,
        report.dropped + 50,
        "ids count every request; the dropped ones are the oldest"
    );
    let said = serde_json::to_string(&report).unwrap();
    assert!(!said.contains(KEY) && !said.contains(MIDDLE));
}
