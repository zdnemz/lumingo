#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Deleting a session, deleting everything, and the export.

mod common;

use std::sync::Arc;

use app_core::api::{
    AdaptiveTiming, L1HelpMode, SaveProviderRequest, ServerEvent, UiLanguage, XpSourceKind,
};
use app_core::error::CoreError;
use app_core::{AppCore, SessionService};
use common::seed::{
    LEARNER_TEXT, seed_session_with_attempt, seed_turn_with_recording, test_core_with_unit,
};

/// True when the raw database file still holds the marker text.
fn file_holds_learner_text(core: &AppCore) -> bool {
    let bytes = std::fs::read(core.database().path()).unwrap();
    let marker = LEARNER_TEXT.as_bytes();
    bytes.windows(marker.len()).any(|w| w == marker)
}

struct Running(i64);

#[async_trait::async_trait]
impl SessionService for Running {
    fn active_session(&self) -> Option<i64> {
        Some(self.0)
    }
    async fn stop_active(&self) -> app_core::CoreResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn deleting_a_session_removes_its_text_and_recording_and_keeps_the_numbers() {
    let t = test_core_with_unit().await;
    let (session, attempt) =
        seed_session_with_attempt(&t.core, storage::SessionKind::Writing).await;
    let recording = seed_turn_with_recording(&t.core, session.id).await;
    t.core
        .record_practice(XpSourceKind::Writing, &format!("session:{}", session.id))
        .await
        .unwrap();
    assert!(recording.exists());
    t.core.database().compact().await.unwrap();
    assert!(
        file_holds_learner_text(&t.core),
        "the seed is in the file before the delete"
    );

    let result = t.core.delete_session(session.id).await.unwrap();
    assert_eq!(result.audio_files_removed, 1);
    assert!(result.compacted);
    assert!(!recording.exists(), "the recording is gone");
    assert!(
        t.core
            .database()
            .turns()
            .list(session.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        t.core
            .database()
            .sessions()
            .get(session.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !file_holds_learner_text(&t.core),
        "the text is gone from the file too"
    );

    // The scores stay and the text that backed them goes.
    let evidence = t.core.attempt_evidence(attempt.id).await.unwrap();
    assert!(
        evidence.evidence.iter().all(|e| e.content.is_none()),
        "{evidence:?}"
    );
    let overview = t.core.progress().await.unwrap();
    assert_eq!(overview.units.len(), 1);
    assert_eq!(overview.objectives.len(), 1);
    assert!(overview.recent_sessions.is_empty());
    // XP never pointed at the session.
    assert!(t.core.game_state().await.unwrap().xp_total > 0);

    let again = t.core.delete_session(session.id).await.unwrap_err();
    assert!(matches!(again, CoreError::NotFound { .. }), "{again:?}");
}

#[tokio::test]
async fn a_recording_path_that_leaves_the_data_directory_is_never_deleted() {
    let t = test_core_with_unit().await;
    let (session, _) = seed_session_with_attempt(&t.core, storage::SessionKind::Lesson).await;
    let turn = t
        .core
        .database()
        .turns()
        .append(&storage::NewTurn {
            session_id: session.id,
            role: storage::TurnRole::Learner,
            input_mode: storage::InputMode::Voice,
            text: "hello".to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: None,
            created_at: t.clock_now(),
        })
        .await
        .unwrap();
    let outside = t.dir.path().join("outside.wav");
    std::fs::write(&outside, b"not ours").unwrap();
    t.core
        .database()
        .audio_clips()
        .add(turn.id, "../outside.wav", 10, &t.clock_now())
        .await
        .unwrap();

    let result = t.core.delete_session(session.id).await.unwrap();
    assert_eq!(result.audio_files_removed, 0);
    assert!(outside.exists());
}

#[tokio::test]
async fn the_running_session_cannot_be_deleted_and_blocks_delete_all() {
    let t = test_core_with_unit().await;
    let (session, _) = seed_session_with_attempt(&t.core, storage::SessionKind::Lesson).await;
    let (other, _) = seed_session_with_attempt(&t.core, storage::SessionKind::Lesson).await;
    t.core
        .attach_sessions(Arc::new(Running(session.id)))
        .unwrap();

    let error = t.core.delete_session(session.id).await.unwrap_err();
    assert!(matches!(error, CoreError::Conflict(_)), "{error:?}");
    let error = t.core.delete_all_data().await.unwrap_err();
    assert!(matches!(error, CoreError::Conflict(_)), "{error:?}");
    // Another session is fine.
    t.core.delete_session(other.id).await.unwrap();
    assert!(
        t.core
            .database()
            .sessions()
            .get(session.id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn delete_all_removes_the_learner_and_keeps_providers_settings_and_the_unit_index() {
    let t = test_core_with_unit().await;
    let (session, _) = seed_session_with_attempt(&t.core, storage::SessionKind::Writing).await;
    let recording = seed_turn_with_recording(&t.core, session.id).await;
    t.core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    let mut settings = t.core.settings();
    settings.display_name = "Sari".to_owned();
    settings.ui_language = UiLanguage::En;
    settings.adaptive_timing = AdaptiveTiming::NeverWait;
    t.core.update_settings(settings).await.unwrap();
    let provider: SaveProviderRequest = serde_json::from_value(serde_json::json!({
        "name": "mine", "protocol": "openai_chat",
        "base_url": "https://api.example.test/v1", "model": "m", "api_key": "sk-abcdefgh-12345678"
    }))
    .unwrap();
    t.core.save_provider(provider).await.unwrap();
    let mut events = t.core.events().subscribe();

    let result = t.core.delete_all_data().await.unwrap();
    assert_eq!(result.audio_files_removed, 1);
    assert!(result.compacted);
    assert!(!recording.exists());
    assert!(!file_holds_learner_text(&t.core));

    // A fresh learner: a new profile row (SQLite may reuse the number), no
    // progress, no XP, the starter hat only.
    assert!(
        t.core
            .database()
            .profiles()
            .get(t.core.profile_id())
            .await
            .unwrap()
            .is_some()
    );
    let overview = t.core.progress().await.unwrap();
    assert!(overview.units.is_empty() && overview.objectives.is_empty());
    assert!(overview.recent_sessions.is_empty());
    let game = t.core.game_state().await.unwrap();
    assert_eq!(game.xp_total, 0);
    assert!(game.streak_days.is_empty());
    assert_eq!(game.cosmetics.iter().filter(|c| c.unlocked).count(), 1);
    let fresh = t.core.settings();
    assert_eq!(fresh.display_name, "Learner", "the name is personal data");
    assert_eq!(
        fresh.ui_language,
        UiLanguage::En,
        "the interface language is kept"
    );
    assert_eq!(fresh.l1_help_mode, L1HelpMode::Auto);
    assert_eq!(
        fresh.adaptive_timing,
        AdaptiveTiming::NeverWait,
        "not learning data"
    );
    // Not learning data: providers and the unit index.
    assert_eq!(t.core.list_providers().providers.len(), 1);
    assert_eq!(t.core.list_units().units.len(), 1);
    // Other tabs are told to start over.
    match events.recv().await.unwrap() {
        ServerEvent::Snapshot { state, .. } => assert_eq!(state.settings.display_name, "Learner"),
        other => panic!("{other:?}"),
    }
    // The core keeps working for the new profile.
    t.core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
}

#[tokio::test]
async fn the_export_has_the_learners_data_and_no_key() {
    let t = test_core_with_unit().await;
    let (session, attempt) =
        seed_session_with_attempt(&t.core, storage::SessionKind::Writing).await;
    seed_turn_with_recording(&t.core, session.id).await;
    t.core
        .record_practice(XpSourceKind::Writing, "session:1")
        .await
        .unwrap();
    let provider: SaveProviderRequest = serde_json::from_value(serde_json::json!({
        "name": "mine", "protocol": "openai_chat",
        "base_url": "https://api.example.test/v1", "model": "m",
        "api_key": "sk-export-SECRETKEYMATERIAL-Hq7Lp"
    }))
    .unwrap();
    t.core.save_provider(provider).await.unwrap();

    let bundle = t.core.export().await.unwrap();
    assert_eq!(bundle.format_version, 1);
    assert_eq!(bundle.exported_at, "2026-10-05T08:00:00.000Z");
    let text = serde_json::to_string(&bundle).unwrap();
    assert!(!text.contains("SECRETKEYMATERIAL"), "no key in the export");
    assert!(!text.contains("q7Lp"), "not even the last four characters");
    assert!(!text.contains("has_key"));

    let data = &bundle.data;
    assert_eq!(data["profile"]["display_name"], "Learner");
    assert_eq!(data["providers"][0]["name"], "mine");
    assert_eq!(
        data["providers"][0]["base_url"],
        "https://api.example.test/v1"
    );
    assert_eq!(data["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(
        data["sessions"][0]["turns"][0]["turn"]["text"],
        LEARNER_TEXT
    );
    assert_eq!(data["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(data["attempts"][0]["attempt"]["id"], attempt.id);
    assert!(
        data["attempts"][0]["evidence"][0]["content"]
            .as_str()
            .unwrap()
            .contains(LEARNER_TEXT)
    );
    assert_eq!(data["progress"]["units"][0]["unit_id"], "a1-u01");
    assert_eq!(data["game"]["xp"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn an_empty_learner_exports_cleanly() {
    let t = common::test_core().await;
    let bundle = t.core.export().await.unwrap();
    assert_eq!(bundle.data["sessions"], serde_json::json!([]));
    assert_eq!(bundle.data["providers"], serde_json::json!([]));
}
