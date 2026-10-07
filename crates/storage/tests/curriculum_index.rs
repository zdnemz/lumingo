//! The curriculum index round trip: the example unit file is loaded by the
//! `curriculum` crate, written to `curriculum_versions`, `units` and
//! `objectives`, and read back unchanged. The mapping between the two crates is
//! the caller's job by design: `curriculum` builds the entry and the manifest,
//! `storage` writes plain data.
#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

mod common;

use common::TempDir;
use curriculum::{UnitIndexEntry, UnitLoader, content_version_for, manifest_checksum};
use storage::{
    Database, IndexStatus, Level, NewCurriculumVersion, NewIndexedObjective, NewIndexedUnit,
    NewProfile, NewSession, SessionKind, StorageError, UiLanguage,
};

const NOW: &str = "2026-10-07T08:00:00.000Z";
const L1_HELP: storage::L1HelpMode = storage::L1HelpMode::Auto;

fn example_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../curriculum/examples/a1-u01.example.json")
}

fn example_entry() -> UnitIndexEntry {
    UnitLoader::new().load_index_entry(&example_path()).unwrap()
}

/// The caller-side mapping: one loaded unit entry becomes one index row.
fn to_index_unit(entry: &UnitIndexEntry) -> NewIndexedUnit {
    NewIndexedUnit {
        id: entry.id.clone(),
        level: to_storage_level(entry.level),
        sequence: i64::from(entry.sequence),
        title_en: entry.title_en.clone(),
        file_sha256: entry.file_sha256.clone(),
        objectives: entry
            .objectives
            .iter()
            .map(|objective| NewIndexedObjective {
                id: objective.id.clone(),
                skill: objective.skill.as_str().to_owned(),
                can_do_en: objective.can_do_en.clone(),
            })
            .collect(),
    }
}

/// The two crates keep separate level enums on purpose: `curriculum` follows
/// the unit schema, `storage` follows the database CHECK. The caller maps.
fn to_storage_level(level: curriculum::Level) -> Level {
    match level {
        curriculum::Level::A1 => Level::A1,
        curriculum::Level::A2 => Level::A2,
        curriculum::Level::B1 => Level::B1,
        curriculum::Level::B2 => Level::B2,
        curriculum::Level::C1 => Level::C1,
        curriculum::Level::C2 => Level::C2,
    }
}

/// The version row for a set of entries, exactly as the app will build it.
fn to_version(entries: &[UnitIndexEntry]) -> NewCurriculumVersion {
    let manifest = manifest_checksum(entries);
    NewCurriculumVersion {
        content_version: content_version_for(&manifest),
        schema_version: entries
            .first()
            .map(|e| e.schema_version.clone())
            .unwrap_or_default(),
        manifest_sha256: manifest,
        installed_at: NOW.to_owned(),
        units: entries.iter().map(to_index_unit).collect(),
    }
}

async fn open(dir: &TempDir) -> Database {
    Database::open(dir.db_path()).await.unwrap()
}

#[tokio::test]
async fn the_example_unit_round_trips_through_the_index() {
    let dir = TempDir::new();
    let db = open(&dir).await;
    let entry = example_entry();
    let version = to_version(std::slice::from_ref(&entry));

    let stored = db.install_curriculum(&version).await.unwrap();
    assert_eq!(stored.content_version, version.content_version);
    assert_eq!(stored.unit_count, 1);
    assert_eq!(stored.schema_version, "1.0");
    assert_eq!(stored.installed_at, NOW);

    // The version compares as current with the same manifest.
    assert_eq!(
        db.curriculum_index_status(&version.content_version, &version.manifest_sha256)
            .await
            .unwrap(),
        IndexStatus::Current
    );

    // The unit row reads back with every field the loader produced.
    let unit = db.indexed_unit("a1-u01").await.unwrap().unwrap();
    assert_eq!(unit.id, entry.id);
    assert_eq!(unit.level, Level::A1);
    assert_eq!(unit.sequence, 1);
    assert_eq!(unit.title_en, entry.title_en);
    assert_eq!(unit.file_sha256, entry.file_sha256);
    assert_eq!(unit.curriculum_version_id, stored.id);

    // The objectives read back in the `<unit id>/<objective id>` form.
    let objectives = db.unit_objectives("a1-u01").await.unwrap();
    assert_eq!(objectives.len(), entry.objectives.len());
    assert_eq!(objectives[0].id, "a1-u01/o1-greet");
    assert_eq!(objectives[0].unit_id, "a1-u01");
    assert_eq!(objectives[0].skill, "speaking_interaction");
    assert_eq!(objectives[0].can_do_en, entry.objectives[0].can_do_en);
    let skills: Vec<&str> = objectives.iter().map(|o| o.skill.as_str()).collect();
    assert!(skills.contains(&"pronunciation") && skills.contains(&"writing"));

    // The checksum listing matches, and the latest version is the one just in.
    assert_eq!(
        db.unit_checksums().await.unwrap(),
        [("a1-u01".to_owned(), entry.file_sha256.clone())]
    );
    assert_eq!(
        db.latest_curriculum_version().await.unwrap().unwrap().id,
        stored.id
    );
}

#[tokio::test]
async fn a_session_can_reference_an_indexed_unit() {
    let dir = TempDir::new();
    let db = open(&dir).await;
    let entry = example_entry();
    db.install_curriculum(&to_version(std::slice::from_ref(&entry)))
        .await
        .unwrap();

    let profile = db
        .create_profile(NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1_HELP,
            created_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    let session = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Lesson,
            unit_id: Some("a1-u01".to_owned()),
            activity_id: Some("a11-roleplay-classmate".to_owned()),
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(session.unit_id.as_deref(), Some("a1-u01"));

    // A unit that is not indexed is still refused by the foreign key.
    let refused = db
        .create_session(NewSession {
            profile_id: profile.id,
            kind: SessionKind::Lesson,
            unit_id: Some("a1-u99".to_owned()),
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: NOW.to_owned(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        refused,
        StorageError::Invalid {
            table: "sessions",
            ..
        }
    ));
}

#[tokio::test]
async fn reinstalling_replaces_the_objectives_and_updates_the_unit() {
    let dir = TempDir::new();
    let db = open(&dir).await;
    let entry = example_entry();
    db.install_curriculum(&to_version(std::slice::from_ref(&entry)))
        .await
        .unwrap();

    // The same unit with an edited file: new checksum, one fewer objective.
    let mut edited = entry.clone();
    edited.file_sha256 = curriculum::sha256_hex(b"edited unit bytes");
    edited.title_en = "Hello again".to_owned();
    edited.objectives.pop();
    let second = to_version(std::slice::from_ref(&edited));
    let stored = db.install_curriculum(&second).await.unwrap();

    let unit = db.indexed_unit("a1-u01").await.unwrap().unwrap();
    assert_eq!(unit.file_sha256, edited.file_sha256);
    assert_eq!(unit.title_en, "Hello again");
    assert_eq!(unit.curriculum_version_id, stored.id);
    assert_eq!(
        db.unit_objectives("a1-u01").await.unwrap().len(),
        edited.objectives.len()
    );

    // Two versions are on record; the newest is the edited one.
    assert_eq!(
        db.latest_curriculum_version().await.unwrap().unwrap().id,
        stored.id
    );
}

#[tokio::test]
async fn a_changed_manifest_under_the_same_version_name_is_reported() {
    let dir = TempDir::new();
    let db = open(&dir).await;
    let entry = example_entry();
    let version = to_version(std::slice::from_ref(&entry));
    db.install_curriculum(&version).await.unwrap();

    assert_eq!(
        db.curriculum_index_status("manifest-never-seen", "x")
            .await
            .unwrap(),
        IndexStatus::Missing
    );
    assert_eq!(
        db.curriculum_index_status(&version.content_version, "some other manifest")
            .await
            .unwrap(),
        IndexStatus::Changed
    );
}

#[tokio::test]
async fn a_bad_objective_id_and_a_duplicate_version_are_refused_without_partial_writes() {
    let dir = TempDir::new();
    let db = open(&dir).await;
    let entry = example_entry();
    let version = to_version(std::slice::from_ref(&entry));

    // An objective whose id does not start with its unit id.
    let mut bad = version.clone();
    bad.units[0].objectives[0].id = "other-unit/o1".to_owned();
    let error = db.install_curriculum(&bad).await.unwrap_err();
    assert!(matches!(
        error,
        StorageError::Invalid {
            table: "objectives",
            ..
        }
    ));
    assert!(db.indexed_units().await.unwrap().is_empty());
    assert!(db.latest_curriculum_version().await.unwrap().is_none());

    // The good version goes in; the same content version again is a conflict
    // and the first rows are untouched.
    let stored = db.install_curriculum(&version).await.unwrap();
    let error = db.install_curriculum(&version).await.unwrap_err();
    assert!(matches!(
        error,
        StorageError::Conflict {
            table: "curriculum_versions",
            ..
        }
    ));
    assert_eq!(
        db.latest_curriculum_version().await.unwrap().unwrap().id,
        stored.id
    );
    assert_eq!(db.indexed_units().await.unwrap().len(), 1);
}

#[tokio::test]
async fn units_of_two_levels_are_listed_in_curriculum_order() {
    let dir = TempDir::new();
    let db = open(&dir).await;
    let mut first = example_entry();
    first.id = "b1-u01".to_owned();
    first.level = curriculum::Level::B1;
    first.sequence = 1;
    for objective in &mut first.objectives {
        objective.id = objective.id.replace("a1-u01/", "b1-u01/");
    }
    let mut second = example_entry();
    second.id = "a1-u02".to_owned();
    second.sequence = 2;
    second.file_sha256 = curriculum::sha256_hex(b"second unit");
    second.objectives.clear();

    // Inserted out of order on purpose.
    db.install_curriculum(&to_version(&[first, second]))
        .await
        .unwrap();
    let listed: Vec<(String, Level, i64)> = db
        .indexed_units()
        .await
        .unwrap()
        .into_iter()
        .map(|u| (u.id, u.level, u.sequence))
        .collect();
    assert_eq!(
        listed,
        [
            ("a1-u02".to_owned(), Level::A1, 2),
            ("b1-u01".to_owned(), Level::B1, 1)
        ],
        "ordered by level then sequence: A1 comes before B1"
    );
}
