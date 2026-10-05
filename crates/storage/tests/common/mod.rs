#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use storage::{Database, L1HelpMode, NewProfile, Profile, Timestamp, UiLanguage};
use tempfile::TempDir;

/// A fresh database in its own temporary directory. Keep the `TempDir` alive for
/// as long as the database is used.
pub async fn temp_db() -> (TempDir, Database) {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("lumingo.sqlite"))
        .await
        .expect("open database");
    (dir, db)
}

/// A timestamp `n` seconds after a fixed start, so tests never read the clock.
pub fn ts(n: i64) -> Timestamp {
    Timestamp::from_unix_seconds(1_790_000_000 + n).expect("valid timestamp")
}

pub async fn make_profile(db: &Database) -> Profile {
    db.profiles()
        .create(&NewProfile {
            display_name: "Test learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at: ts(0),
        })
        .await
        .expect("create profile")
}
