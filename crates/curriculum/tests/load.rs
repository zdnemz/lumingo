#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{example_bytes, example_path, example_value, scratch_dir};
use curriculum::{
    Activity, LoadError, Unit, UnitIndexEntry, load_unit_bytes, load_unit_dir, load_unit_file,
    sha256_hex,
};
use serde_json::{Value, json};

fn issues_of(value: &Value) -> Vec<(String, String)> {
    let bytes = serde_json::to_vec(value).unwrap();
    match load_unit_bytes(&bytes) {
        Err(LoadError::Schema { issues }) => issues
            .into_iter()
            .map(|issue| (issue.pointer, issue.keyword))
            .collect(),
        Err(other) => panic!("expected a schema failure, got {other}"),
        Ok(_) => panic!("expected a schema failure, but the unit loaded"),
    }
}

#[test]
fn the_example_unit_loads() {
    let loaded = load_unit_file(&example_path()).expect("example loads");
    assert_eq!(loaded.unit.id, "a1-u01");
    assert_eq!(loaded.unit.activities.len(), 17);
    assert_eq!(loaded.unit.objectives.len(), 6);
}

#[test]
fn the_model_round_trips_to_the_same_json() {
    // If a field of the schema were missing from the model, or a default were invented,
    // serialising the typed unit would not give the original document back.
    let original = example_value();
    let unit: Unit = serde_json::from_value(original.clone()).unwrap();
    let back = serde_json::to_value(&unit).unwrap();
    assert_eq!(back, original);
}

#[test]
fn the_checksum_is_the_sha256_of_the_file_bytes() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let bytes = example_bytes();
    let loaded = load_unit_bytes(&bytes).unwrap();
    assert_eq!(loaded.checksum, sha256_hex(&bytes));
    assert_eq!(loaded.checksum.len(), 64);

    // A change in a single byte, even whitespace, changes the checksum.
    let mut changed = bytes.clone();
    changed.push(b'\n');
    assert_ne!(load_unit_bytes(&changed).unwrap().checksum, loaded.checksum);
}

#[test]
fn the_index_entry_counts_activities_per_skill() {
    let loaded = load_unit_file(&example_path()).unwrap();
    let entry: UnitIndexEntry = loaded.index_entry();
    assert_eq!(entry.id, "a1-u01");
    assert_eq!(entry.level, curriculum::Level::A1);
    assert_eq!(entry.title.en, "Hello! Nice to meet you");
    assert_eq!(entry.estimated_minutes, 60);
    assert_eq!(entry.checksum, loaded.checksum);
    let counts = entry.skill_counts;
    assert_eq!(
        (
            counts.listening,
            counts.speaking,
            counts.reading,
            counts.writing,
            counts.mediation,
            counts.grammar,
            counts.vocabulary,
            counts.pronunciation
        ),
        (3, 2, 2, 3, 0, 2, 2, 3)
    );
}

#[test]
fn a_wrong_id_is_rejected_with_its_pointer() {
    let mut unit = example_value();
    unit["id"] = json!("A1-u01");
    assert_eq!(issues_of(&unit), [("/id".to_owned(), "pattern".to_owned())]);
}

#[test]
fn an_unknown_root_field_is_rejected_at_the_root() {
    let mut unit = example_value();
    unit["surprise"] = json!(1);
    assert_eq!(
        issues_of(&unit),
        [(String::new(), "additionalProperties".to_owned())]
    );
}

#[test]
fn a_wrong_value_inside_an_activity_points_at_that_value() {
    let mut unit = example_value();
    unit["activities"][2]["answers"] = json!("am");
    assert_eq!(
        issues_of(&unit),
        [("/activities/2/answers".to_owned(), "type".to_owned())]
    );
}

#[test]
fn a_missing_field_is_reported_only_for_the_chosen_activity_type() {
    let mut unit = example_value();
    unit["activities"][0]
        .as_object_mut()
        .unwrap()
        .remove("stem");
    let issues = issues_of(&unit);
    assert_eq!(
        issues,
        [("/activities/0".to_owned(), "required".to_owned())],
        "one error for the mcq, not one per activity type"
    );
}

#[test]
fn an_unknown_activity_type_points_at_the_type_field() {
    let mut unit = example_value();
    unit["activities"][3]["type"] = json!("jigsaw");
    assert_eq!(
        issues_of(&unit),
        [("/activities/3/type".to_owned(), "oneOf".to_owned())]
    );
}

#[test]
fn an_extra_field_on_an_activity_is_rejected() {
    let mut unit = example_value();
    unit["activities"][4]["hint"] = json!("x");
    let issues = issues_of(&unit);
    assert_eq!(
        issues,
        [(
            "/activities/4".to_owned(),
            "unevaluatedProperties".to_owned()
        )]
    );
}

#[test]
fn a_listening_set_needs_exactly_one_audio_source() {
    let mut unit = example_value();
    let set = &mut unit["activities"][14];
    assert_eq!(set["type"], "listening_set");
    set["dialogue_id"] = json!("d-new-classmates");
    assert_eq!(
        issues_of(&unit),
        [("/activities/14".to_owned(), "oneOf".to_owned())],
        "both present"
    );

    let mut unit = example_value();
    unit["activities"][14]
        .as_object_mut()
        .unwrap()
        .remove("audio_text");
    assert_eq!(
        issues_of(&unit),
        [("/activities/14".to_owned(), "oneOf".to_owned())],
        "neither present"
    );
}

#[test]
fn every_error_is_reported_not_only_the_first() {
    let mut unit = example_value();
    unit["id"] = json!("nope");
    unit["sequence"] = json!(31);
    unit["activities"][1]["options"] = json!(["only", "two"]);
    let pointers: Vec<String> = issues_of(&unit).into_iter().map(|(p, _)| p).collect();
    assert!(pointers.contains(&"/id".to_owned()));
    assert!(pointers.contains(&"/sequence".to_owned()));
    assert!(pointers.contains(&"/activities/1/options".to_owned()));
    assert_eq!(pointers.len(), 3);
}

#[test]
fn the_error_text_names_the_pointer() {
    let mut unit = example_value();
    unit["estimated_minutes"] = json!(5);
    let err = load_unit_bytes(&serde_json::to_vec(&unit).unwrap()).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("/estimated_minutes"), "{text}");
}

#[test]
fn invalid_json_reports_the_position() {
    let err = load_unit_bytes(b"{\n  \"id\": \n").unwrap_err();
    match err {
        LoadError::Json { line, .. } => assert_eq!(line, 3),
        other => panic!("unexpected {other}"),
    }
}

#[test]
fn a_missing_file_reports_its_path() {
    let err = load_unit_file(std::path::Path::new("/definitely/not/here.json")).unwrap_err();
    assert!(matches!(err, LoadError::Io { .. }));
    assert!(err.to_string().contains("/definitely/not/here.json"));
}

#[test]
fn the_model_denies_unknown_fields_by_itself() {
    // The loader checks the schema first, but the types must not accept strays either.
    let mut unit = example_value();
    unit["objectives"][0]["colour"] = json!("red");
    assert!(serde_json::from_value::<Unit>(unit).is_err());

    let mut unit = example_value();
    unit["activities"][0]["colour"] = json!("red");
    assert!(serde_json::from_value::<Unit>(unit).is_err());
}

#[test]
fn activities_deserialise_to_the_right_variant() {
    let loaded = load_unit_file(&example_path()).unwrap();
    let kinds: Vec<&'static str> = loaded
        .unit
        .activities
        .iter()
        .map(|a| a.activity_type().as_str())
        .collect();
    assert_eq!(kinds[0], "mcq");
    assert_eq!(kinds[9], "guided_speaking");
    assert_eq!(kinds[11], "guided_writing");
    assert_eq!(kinds[13], "reading_set");
    assert_eq!(kinds[15], "error_correction");
    assert!(matches!(loaded.unit.activities[10], Activity::Roleplay(_)));
}

#[test]
fn the_directory_loader_returns_one_result_per_file_in_path_order() {
    let dir = scratch_dir("load_dir_mixed");
    std::fs::create_dir_all(dir.join("a1")).unwrap();
    std::fs::write(dir.join("a1/a1-u01.json"), example_bytes()).unwrap();
    std::fs::write(dir.join("a1/broken.json"), b"{ not json").unwrap();
    std::fs::write(dir.join(".gitkeep"), b"").unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();

    let files = load_unit_dir(&dir).unwrap();
    assert_eq!(files.len(), 2, "only .json files are loaded");
    assert!(files[0].path.ends_with("a1/a1-u01.json"));
    assert!(files[0].result.is_ok());
    assert!(files[1].path.ends_with("a1/broken.json"));
    assert!(matches!(files[1].result, Err(LoadError::Json { .. })));
}

#[test]
fn the_directory_loader_fails_for_a_missing_directory() {
    let err = load_unit_dir(std::path::Path::new("/definitely/not/a/dir")).unwrap_err();
    assert!(matches!(err, LoadError::Io { .. }));
}

#[test]
fn the_examples_folder_loads() {
    let files = load_unit_dir(&common::repo_root().join("curriculum/examples")).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].result.is_ok());
}
