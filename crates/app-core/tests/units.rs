#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The curriculum index: loading a folder, the database index, and the detail read.

mod common;

use std::path::Path;

use app_core::AppCore;
use app_core::error::CoreError;
use common::{config, test_core};
use serde_json::Value;

fn example() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../curriculum/examples/a1-u01.example.json");
    serde_json::from_slice(&std::fs::read(path).expect("example unit")).expect("json")
}

/// The example unit under another id and position.
fn variant(id: &str, level: &str, sequence: u64) -> Value {
    let mut unit = example();
    unit["id"] = id.into();
    unit["level"] = level.into();
    unit["sequence"] = sequence.into();
    unit
}

fn write(dir: &Path, name: &str, unit: &Value) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), serde_json::to_vec_pretty(unit).unwrap()).unwrap();
}

#[tokio::test]
async fn a_missing_folder_is_a_reported_problem_not_a_failure() {
    let t = test_core().await;
    let list = t.core.list_units();
    assert!(list.units.is_empty() && list.content_version.is_none());
    assert_eq!(list.issues.len(), 1);
    assert_eq!(list.issues[0].file, ".");
}

#[tokio::test]
async fn units_are_listed_in_learning_order_and_indexed_in_the_database() {
    let t = test_core().await;
    let dir = t.dir.path().join("units");
    // Written out of order on purpose.
    write(&dir, "b.json", &variant("a1-u02", "A1", 2));
    write(&dir, "c.json", &variant("a2-u01", "A2", 1));
    write(&dir, "a.json", &example());
    let core = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();

    let list = core.list_units();
    assert!(list.issues.is_empty(), "{:?}", list.issues);
    let ids: Vec<&str> = list.units.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(ids, ["a1-u01", "a1-u02", "a2-u01"]);
    let first = &list.units[0];
    assert_eq!(first.sequence, 1);
    assert_eq!(first.estimated_minutes, 60);
    assert!(first.objective_count >= 1);
    assert!(first.skill_counts.speaking + first.skill_counts.listening > 0);
    assert!(
        list.content_version
            .as_deref()
            .unwrap()
            .starts_with("manifest-")
    );

    // The database index follows, with objectives named "<unit>/<objective>".
    let indexed = core.database().curriculum().units().await.unwrap();
    assert_eq!(indexed.len(), 3);
    let objectives = core
        .database()
        .curriculum()
        .objectives("a1-u01")
        .await
        .unwrap();
    assert_eq!(
        objectives.len(),
        usize::try_from(first.objective_count).unwrap()
    );
    assert!(objectives.iter().all(|o| o.id.starts_with("a1-u01/")));
    let version = core
        .database()
        .curriculum()
        .latest_version()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(version.unit_count, 3);
    assert_eq!(Some(version.content_version), list.content_version);
}

#[tokio::test]
async fn reopening_with_the_same_files_does_not_index_again() {
    let t = test_core().await;
    write(&t.dir.path().join("units"), "a.json", &example());
    let first = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();
    let version = first
        .database()
        .curriculum()
        .latest_version()
        .await
        .unwrap()
        .unwrap();
    first.close().await.unwrap();

    let second = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();
    let again = second
        .database()
        .curriculum()
        .latest_version()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.id, version.id, "no new version row");
    assert_eq!(
        second.list_units().content_version,
        Some(version.content_version)
    );
}

#[tokio::test]
async fn bad_files_are_reported_and_never_hide_the_good_ones() {
    let t = test_core().await;
    let dir = t.dir.path().join("units");
    write(&dir, "good.json", &example());
    std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
    let mut wrong = example();
    wrong["level"] = "Z9".into();
    write(&dir, "wrong.json", &wrong);
    write(&dir, "same-id.json", &variant("a1-u01", "A1", 5));
    write(&dir, "same-place.json", &variant("a1-u09", "A1", 1));
    let core = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();

    let list = core.list_units();
    assert_eq!(list.units.len(), 1);
    assert_eq!(list.units[0].id, "a1-u01");
    let mut files: Vec<&str> = list.issues.iter().map(|i| i.file.as_str()).collect();
    files.sort_unstable();
    assert_eq!(
        files,
        [
            "broken.json",
            "same-id.json",
            "same-place.json",
            "wrong.json"
        ]
    );
    assert!(
        list.issues
            .iter()
            .all(|i| !i.message.is_empty() && i.message.chars().count() <= 401)
    );
}

#[tokio::test]
async fn a_unit_is_read_whole_from_its_file() {
    let t = test_core().await;
    write(&t.dir.path().join("units"), "a.json", &example());
    let core = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();

    let detail = core.unit("a1-u01").await.unwrap();
    assert_eq!(detail.unit.id, "a1-u01");
    assert_eq!(detail.unit.title.en, "Hello! Nice to meet you");
    assert!(!detail.unit.activities.is_empty());
    let missing = core.unit("a1-u99").await.unwrap_err();
    assert!(matches!(missing, CoreError::NotFound { .. }), "{missing:?}");
}

#[tokio::test]
async fn a_file_edited_after_indexing_is_refused_until_the_index_is_rebuilt() {
    let t = test_core().await;
    let dir = t.dir.path().join("units");
    write(&dir, "a.json", &example());
    let core = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();
    let before = core.list_units().content_version.unwrap();

    let mut edited = example();
    edited["theme"] = "greetings_edited".into();
    write(&dir, "a.json", &edited);
    let stale = core.unit("a1-u01").await.unwrap_err();
    assert!(matches!(stale, CoreError::Conflict(_)), "{stale:?}");

    let list = core.reindex_curriculum().await.unwrap();
    assert_ne!(
        list.content_version.unwrap(),
        before,
        "an edit is a new version"
    );
    assert_eq!(list.units[0].theme, "greetings_edited");
    assert_eq!(
        core.unit("a1-u01").await.unwrap().unit.theme,
        "greetings_edited"
    );
}
