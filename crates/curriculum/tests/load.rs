#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions
use curriculum::{Activity, Level, LoadError, UnitLoader, load_dir};
use serde_json::{Value, json};
use std::path::Path;

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn paths(err: LoadError) -> Vec<String> {
    match err {
        LoadError::Schema(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("expected a schema error, got {other}"),
    }
}

#[test]
fn the_example_unit_loads_with_every_part_typed() {
    let unit = UnitLoader::new().load_str(EXAMPLE).unwrap();
    assert_eq!(
        (unit.id.as_str(), unit.level, unit.schema_version.as_str()),
        ("a1-u01", Level::A1, "1.0")
    );
    assert!(
        !unit.objectives.is_empty()
            && !unit.targets.vocabulary.is_empty()
            && !unit.dialogues.is_empty()
    );
    assert!(
        unit.activities
            .iter()
            .all(|a| !a.common().objective_ids.is_empty())
    );
    // the example is meant to cover several activity types
    let kinds: std::collections::HashSet<_> =
        unit.activities.iter().map(std::mem::discriminant).collect();
    assert!(kinds.len() >= 6, "{} kinds", kinds.len());
    assert!(
        unit.activities
            .iter()
            .any(|a| matches!(a, Activity::Mcq { .. }))
    );
}

#[test]
fn a_unit_survives_a_round_trip_through_the_types() {
    let loader = UnitLoader::new();
    let unit = loader.load_str(EXAMPLE).unwrap();
    let again = loader
        .load_value(serde_json::to_value(&unit).unwrap())
        .unwrap();
    assert_eq!(
        unit, again,
        "serialising and loading again must give the same unit"
    );
}

#[test]
fn a_missing_required_field_is_reported_at_the_unit() {
    let mut v = example();
    v.as_object_mut().unwrap().remove("theme");
    let err = UnitLoader::new().load_value(v).unwrap_err();
    assert!(
        paths(err).contains(&String::new()),
        "a missing root property has the root as its path"
    );
}

#[test]
fn a_bad_value_deep_in_an_activity_is_reported_with_its_path() {
    let mut v = example();
    let i = v["activities"]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a["type"] == "mcq")
        .unwrap();
    v["activities"][i]["answer_index"] = json!(9);
    let err = UnitLoader::new().load_value(v).unwrap_err();
    assert!(
        paths(err)
            .iter()
            .any(|p| p == &format!("/activities/{i}/answer_index"))
    );
}

#[test]
fn bad_ids_levels_and_extra_properties_are_rejected() {
    let loader = UnitLoader::new();
    let mut v = example();
    v["id"] = json!("a1-u99");
    assert!(paths(loader.load_value(v).unwrap_err()).contains(&"/id".to_owned()));
    let mut v = example();
    v["level"] = json!("D9");
    assert!(paths(loader.load_value(v).unwrap_err()).contains(&"/level".to_owned()));
    let mut v = example();
    v["surprise"] = json!(true);
    assert!(
        loader.load_value(v).is_err(),
        "additionalProperties is false at the root"
    );
    let mut v = example();
    v["activities"][0]["unexpected_field"] = json!(1);
    assert!(
        loader.load_value(v).is_err(),
        "unevaluatedProperties is false inside activities"
    );
}

#[test]
fn text_that_is_not_json_reports_a_line_and_column() {
    let err = UnitLoader::new().load_str("{\n  \"id\": \n").unwrap_err();
    assert!(matches!(err, LoadError::Json { line: 3, .. }), "{err}");
}

#[test]
fn the_error_message_names_the_first_paths() {
    let mut v = example();
    v["sequence"] = json!("one");
    let message = UnitLoader::new().load_value(v).unwrap_err().to_string();
    assert!(message.contains("/sequence"), "{message}");
}

#[test]
fn load_dir_reports_each_file_on_its_own() {
    let dir = std::env::temp_dir().join(format!("curriculum-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a1")).unwrap();
    std::fs::write(dir.join("a1/good.json"), EXAMPLE).unwrap();
    std::fs::write(dir.join("a1/bad.json"), "{}").unwrap();
    std::fs::write(dir.join("a1/notes.txt"), "ignored").unwrap();
    let results = load_dir(&UnitLoader::new(), &dir).unwrap();
    let names: Vec<(String, bool)> = results
        .iter()
        .map(|(p, r)| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                r.is_ok(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("bad.json".to_owned(), false),
            ("good.json".to_owned(), true)
        ]
    );
    assert!(load_dir(&UnitLoader::new(), Path::new("/definitely/not/here")).is_err());
}
