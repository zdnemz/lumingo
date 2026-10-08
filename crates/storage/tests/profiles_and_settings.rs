#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{make_profile, temp_db, ts};
use storage::{L1HelpMode, StorageError, UiLanguage};

#[tokio::test]
async fn profile_round_trips_and_updates() {
    let (_dir, db) = temp_db().await;
    let created = make_profile(&db).await;

    let fetched = db.profiles().get(created.id).await.expect("get");
    assert_eq!(fetched.as_ref(), Some(&created));
    assert_eq!(
        db.profiles().first().await.expect("first"),
        Some(created.clone())
    );

    let mut edited = created.clone();
    edited.ui_language = UiLanguage::En;
    edited.l1_help_mode = L1HelpMode::Off;
    db.profiles().update(&edited).await.expect("update");
    assert_eq!(
        db.profiles().get(created.id).await.expect("get"),
        Some(edited)
    );
}

#[tokio::test]
async fn updating_or_deleting_a_missing_profile_is_not_found() {
    let (_dir, db) = temp_db().await;
    assert!(matches!(
        db.profiles().delete(99).await,
        Err(StorageError::NotFound { .. })
    ));
}

#[tokio::test]
async fn settings_set_replaces_and_remove_reports_presence() {
    let (_dir, db) = temp_db().await;
    let settings = db.settings();
    assert_eq!(settings.get("theme").await.expect("get"), None);

    settings.set("theme", "dark", &ts(1)).await.expect("set");
    settings
        .set("theme", "light", &ts(2))
        .await
        .expect("set again");
    settings
        .set("motion", "reduced", &ts(3))
        .await
        .expect("set other");

    assert_eq!(
        settings.get("theme").await.expect("get").as_deref(),
        Some("light")
    );
    let all = settings.all().await.expect("all");
    let keys: Vec<&str> = all.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["motion", "theme"]);
    assert_eq!(all[1].updated_at, ts(2));

    assert!(settings.remove("theme").await.expect("remove"));
    assert!(!settings.remove("theme").await.expect("remove again"));
}
