#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Lifecycle, snapshot, events and settings.

mod common;

use std::sync::Arc;

use app_core::api::{AdaptiveTiming, Feature, L1HelpMode, ServerEvent, Settings, UiLanguage};
use app_core::error::CoreError;
use app_core::{AppCore, SessionService};
use common::{config, test_core};

#[tokio::test]
async fn a_fresh_core_has_one_default_profile_and_default_settings() {
    let t = test_core().await;
    let settings = t.core.settings();
    assert_eq!(settings.display_name, "Learner");
    assert_eq!(settings.ui_language, UiLanguage::Id);
    assert_eq!(settings.l1, "id");
    assert_eq!(settings.l1_help_mode, L1HelpMode::Auto);
    assert_eq!(settings.adaptive_timing, AdaptiveTiming::Auto);
    assert!(!settings.keep_recordings, "recordings are opt-in");
    assert!(
        t.core
            .database()
            .profiles()
            .first()
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn the_snapshot_reports_hardware_and_every_missing_feature() {
    let t = test_core().await;
    let snapshot = t.core.snapshot();
    assert_eq!(snapshot.hardware, common::fixed_hardware());
    assert_eq!(snapshot.unavailable, Feature::ALL.to_vec());
    assert!(!snapshot.dev_mode);
}

#[tokio::test]
async fn settings_survive_a_restart() {
    let t = test_core().await;
    let new = Settings {
        display_name: "Sari".to_owned(),
        ui_language: UiLanguage::En,
        l1: "jv".to_owned(),
        l1_help_mode: L1HelpMode::Off,
        adaptive_timing: AdaptiveTiming::NeverWait,
        keep_recordings: true,
    };
    let stored = t.core.update_settings(new.clone()).await.unwrap();
    assert_eq!(stored, new);
    assert_eq!(t.core.snapshot().settings, new);
    t.core.close().await.unwrap();

    let again = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();
    assert_eq!(again.settings(), new);
    // Still one profile.
    assert_eq!(
        again
            .database()
            .profiles()
            .first()
            .await
            .unwrap()
            .unwrap()
            .id,
        again.profile_id()
    );
}

#[tokio::test]
async fn invalid_settings_are_refused_and_change_nothing() {
    let t = test_core().await;
    let before = t.core.settings();
    let mut bad = before.clone();
    bad.l1 = "indonesian".to_owned();
    let error = t.core.update_settings(bad).await.unwrap_err();
    assert!(matches!(error, CoreError::InvalidInput(_)), "{error:?}");
    assert_eq!(t.core.settings(), before);
}

#[tokio::test]
async fn heartbeats_and_snapshots_share_one_sequence() {
    let t = test_core().await;
    let mut rx = t.core.events().subscribe();
    let first = t.core.snapshot_event();
    assert_eq!(first.seq(), 0);
    t.core.publish_heartbeat();
    t.core.publish_heartbeat();
    assert!(matches!(
        rx.recv().await.unwrap(),
        ServerEvent::Heartbeat { seq: 1, .. }
    ));
    assert_eq!(rx.recv().await.unwrap().seq(), 2);
    assert_eq!(t.core.snapshot_event().seq(), 2);
}

#[tokio::test]
async fn changes_are_refused_after_shutdown_has_begun_but_reads_still_work() {
    let t = test_core().await;
    t.core.request_shutdown();
    let error = t.core.update_settings(t.core.settings()).await.unwrap_err();
    assert!(matches!(error, CoreError::ShuttingDown), "{error:?}");
    assert_eq!(t.core.settings().display_name, "Learner");
}

struct FakeSessions;

#[async_trait::async_trait]
impl SessionService for FakeSessions {
    fn active_session(&self) -> Option<i64> {
        None
    }
    async fn stop_active(&self) -> app_core::CoreResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn sessions_are_not_available_until_a_service_is_attached() {
    let t = test_core().await;
    let error = t.core.session_service().err().expect("not available");
    assert!(
        matches!(error, CoreError::NotAvailable(Feature::Sessions)),
        "{error:?}"
    );
    assert_eq!(error.code(), app_core::api::ErrorCode::NotAvailable);

    t.core.attach_sessions(Arc::new(FakeSessions)).unwrap();
    assert!(t.core.session_service().is_ok());
    assert!(!t.core.snapshot().unavailable.contains(&Feature::Sessions));
    assert!(t.core.snapshot().unavailable.contains(&Feature::Speech));
    // Only one service can be attached.
    assert!(matches!(
        t.core.attach_sessions(Arc::new(FakeSessions)),
        Err(CoreError::Conflict(_))
    ));
}
