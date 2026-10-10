#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{
    content_cli, example_value, json_report, repo_root, scratch_dir, stderr, stdout, write_json,
};
use serde_json::{Value, json};

fn path_str(path: &std::path::Path) -> String {
    path.to_str().expect("utf-8 path").to_owned()
}

#[test]
fn the_example_unit_passes_with_exit_code_zero() {
    let output = content_cli(&["validate", "curriculum/examples"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("a1-u01.example.json"), "{text}");
    assert!(text.contains("0 error(s)"), "{text}");
    assert!(text.contains("PASS"), "{text}");
}

#[test]
fn warnings_are_listed_and_do_not_fail_the_run() {
    let output = content_cli(&["validate", "curriculum/examples"]);
    let text = stdout(&output);
    assert!(text.contains("warning W07  /activities/14"), "{text}");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn rules_that_could_not_run_are_listed_and_not_passed_silently() {
    let text = stdout(&content_cli(&["validate", "curriculum/examples"]));
    for code in ["W01", "W02", "X03", "X05"] {
        assert!(
            text.contains(&format!("skipped {code}")),
            "{code} missing in\n{text}"
        );
    }
    // W03 runs through the real checker, and the example unit's model answers pass it.
    assert!(!text.contains("skipped W03"), "W03 should run in\n{text}");
    // The rubric catalog exists in the repository, so X06 runs instead of being skipped.
    assert!(!text.contains("skipped X06"), "X06 should run in\n{text}");
}

#[test]
fn the_json_report_has_one_object_per_file_with_code_severity_path_and_message() {
    let output = content_cli(&["validate", "curriculum/examples", "--format", "json"]);
    assert_eq!(output.status.code(), Some(0));
    let report = json_report(&output);
    assert_eq!(report["passed"], true);
    assert_eq!(report["errors"], 0);
    assert_eq!(report["files_examined"], 1);
    let file = &report["files"][0];
    assert_eq!(file["kind"], "unit");
    assert_eq!(file["id"], "a1-u01");
    assert_eq!(file["checksum"].as_str().unwrap().len(), 64);
    let warning = &file["findings"][0];
    assert_eq!(warning["code"], "W07");
    assert_eq!(warning["severity"], "warning");
    assert_eq!(warning["path"], "/activities/14");
    assert!(warning["message"].as_str().unwrap().contains("18 words"));
}

#[test]
fn the_checksum_is_the_sha256_of_the_file() {
    let report = json_report(&content_cli(&[
        "validate",
        "curriculum/examples",
        "--format",
        "json",
    ]));
    let bytes = std::fs::read(repo_root().join("curriculum/examples/a1-u01.example.json")).unwrap();
    assert_eq!(
        report["files"][0]["checksum"].as_str().unwrap(),
        curriculum::sha256_hex(&bytes)
    );
}

#[test]
fn a_broken_unit_fails_with_exit_code_one_and_names_the_rule_and_path() {
    let dir = scratch_dir("cli_broken_unit");
    let mut unit = example_value();
    unit["activities"][3]["answer"] = json!("My name is Dewi Putri");
    let file = dir.join("a1-u01.json");
    write_json(&file, &unit);

    let output = content_cli(&["validate", &path_str(&file)]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(text.contains("error   E07  /activities/3/answer"), "{text}");
    assert!(text.contains("FAIL"), "{text}");

    let output = content_cli(&["validate", &path_str(&file), "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output);
    assert_eq!(report["passed"], false);
    assert_eq!(report["errors"], 1);
    let finding = &report["files"][0]["findings"][0];
    assert_eq!(finding["code"], "E07");
    assert_eq!(finding["severity"], "error");
    assert_eq!(finding["path"], "/activities/3/answer");
}

#[test]
fn schema_errors_are_reported_with_their_json_pointer() {
    let dir = scratch_dir("cli_schema_error");
    let mut unit = example_value();
    unit["activities"][2]["answers"] = json!("am");
    let file = dir.join("a1-u01.json");
    write_json(&file, &unit);
    let report = json_report(&content_cli(&[
        "validate",
        &path_str(&file),
        "--format",
        "json",
    ]));
    let finding = &report["files"][0]["findings"][0];
    assert_eq!(finding["code"], "E01");
    assert_eq!(finding["path"], "/activities/2/answers");
}

#[test]
fn a_file_that_is_not_json_is_an_e01_error() {
    let dir = scratch_dir("cli_not_json");
    let file = dir.join("a1-u01.json");
    std::fs::write(&file, "{ not json").unwrap();
    let output = content_cli(&["validate", &path_str(&file), "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output);
    assert_eq!(report["files"][0]["findings"][0]["code"], "E01");
}

#[test]
fn a_missing_path_exits_with_two_and_says_why() {
    let output = content_cli(&["validate", "no/such/folder"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("no/such/folder"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_unknown_kind_exits_with_two() {
    let output = content_cli(&["validate", "curriculum/examples", "--kind", "poem"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unknown kind"));
}

#[test]
fn an_empty_unit_folder_passes_and_says_nothing_was_validated() {
    let output = content_cli(&["validate", "curriculum/units"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout(&output).contains("nothing was validated"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_folder_runs_the_rules_across_units() {
    let dir = scratch_dir("cli_cross_units");
    let first = example_value();
    let mut third = example_value();
    third["sequence"] = json!(3);
    third["id"] = json!("a1-u03");
    write_json(&dir.join("a1/a1-u01.json"), &first);
    write_json(&dir.join("a1/a1-u03.json"), &third);

    let output = content_cli(&["validate", &path_str(&dir), "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output);
    let set_findings = report["set"]["findings"].as_array().unwrap();
    assert!(
        set_findings
            .iter()
            .any(|f| f["code"] == "X01" && f["message"].as_str().unwrap().contains("sequence 2")),
        "{set_findings:?}"
    );
}

#[test]
fn a_single_file_skips_the_rules_that_need_a_folder() {
    let dir = scratch_dir("cli_single_file");
    let file = dir.join("a1-u01.json");
    write_json(&file, &example_value());
    let output = content_cli(&["validate", &path_str(&file)]);
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    for code in ["X01", "X02", "X04"] {
        assert!(
            text.contains(&format!("skipped {code}")),
            "{code} missing in\n{text}"
        );
    }
}

fn curriculum_tree(name: &str) -> std::path::PathBuf {
    let root = scratch_dir(name).join("curriculum");
    write_json(&root.join("units/a1/a1-u01.json"), &example_value());
    root
}

fn rubric(id: &str, family: &str) -> Value {
    let bands: Vec<Value> = (0..5)
        .map(|b| json!({"band": b, "description": {"en": "x"}}))
        .collect();
    json!({
        "schema_version": "1.0", "id": id, "version": 1, "level": "A1", "task_family": family,
        "dimensions": {
            "task_achievement": bands, "range": bands, "accuracy": bands, "coherence": bands
        }
    })
}

#[test]
fn rubric_ids_are_looked_up_in_the_catalogs_folder() {
    let root = curriculum_tree("cli_rubrics");
    write_json(
        &root.join("catalogs/rubrics/spoken.json"),
        &rubric("rubric-a1-spoken-production", "spoken_production"),
    );
    let units = path_str(&root.join("units"));

    let output = content_cli(&["validate", &units, "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output);
    let finding = &report["files"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"] == "X06")
        .expect("the written rubric is missing")
        .clone();
    assert_eq!(finding["path"], "/activities/11/rubric_id");

    write_json(
        &root.join("catalogs/rubrics/written.json"),
        &rubric("rubric-a1-written-production", "written_production"),
    );
    let output = content_cli(&["validate", &units]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
}

#[test]
fn catalog_files_are_recognised_by_their_location_and_validated() {
    let root = curriculum_tree("cli_catalog_files");
    let mut broken = rubric("rubric-a1-spoken-production", "spoken_production");
    broken["dimensions"]["range"][2]["band"] = json!(3);
    write_json(&root.join("catalogs/rubrics/spoken.json"), &broken);

    let output = content_cli(&["validate", &path_str(&root), "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output);
    let rubric_report = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "rubric")
        .expect("the rubric was validated as a rubric");
    assert_eq!(rubric_report["findings"][0]["code"], "K01");
    assert_eq!(
        rubric_report["findings"][0]["path"],
        "/dimensions/range/2/band"
    );
}

#[test]
fn syllabus_files_are_compared_with_the_units() {
    let root = curriculum_tree("cli_syllabus");
    let syllabus = json!({
        "schema_version": "1.0", "level": "A1",
        "units": [{
            "id": "a1-u01", "sequence": 1,
            "title": {"en": "Hello", "id": "Halo"}, "theme": "family",
            "objectives": [
                {"skill": "listening", "can_do": {"en": "x", "id": "y"}},
                {"skill": "writing", "can_do": {"en": "x", "id": "y"}}
            ],
            "grammar_ids": ["g-be-i-am", "g-be-from", "g-question-chunks"],
            "vocabulary_field": "greetings", "functions": ["greeting"], "pronunciation_focus": []
        }]
    });
    write_json(&root.join("syllabus/a1.json"), &syllabus);
    let output = content_cli(&[
        "validate",
        &path_str(&root.join("units")),
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let findings = json_report(&output)["files"][0]["findings"]
        .as_array()
        .unwrap()
        .clone();
    let paths: Vec<&str> = findings
        .iter()
        .filter(|f| f["code"] == "X03")
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["/theme", "/objectives"]);
}

#[test]
fn a_word_list_switches_on_w01_and_w02() {
    let dir = scratch_dir("cli_word_list");
    let list = dir.join("words.csv");
    std::fs::write(
        &list,
        "hello,A1\nhi,A1\ngoodbye,A1\nname,A1\nfrom,A2\nfine,A1\n",
    )
    .unwrap();
    let output = content_cli(&[
        "validate",
        "curriculum/examples",
        "--word-list",
        &path_str(&list),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(
        text.contains("warning W02  /targets/vocabulary/9/level_tag"),
        "{text}"
    );
    assert!(!text.contains("skipped W01"), "{text}");
}

#[test]
fn complete_mode_demands_all_thirty_units() {
    let output = content_cli(&[
        "validate",
        "curriculum/examples",
        "--complete",
        "--format",
        "json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_report(&output);
    let messages: Vec<&str> = report["set"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["message"].as_str().unwrap())
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("1 to 30")),
        "{messages:?}"
    );
}

#[test]
fn the_schema_folder_is_not_mistaken_for_units() {
    // Validating the whole curriculum folder must skip schema/ (and data/ and checkpoints/).
    let output = content_cli(&["validate", "curriculum", "--format", "json"]);
    let report = json_report(&output);
    let files: Vec<&str> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["file"].as_str().unwrap())
        .collect();
    assert!(files.iter().all(|f| !f.contains("/schema/")), "{files:?}");
    assert!(files.iter().any(|f| f.ends_with("a1-u01.example.json")));
}

#[test]
fn the_kind_flag_overrides_the_guess() {
    let dir = scratch_dir("cli_kind_flag");
    let file = dir.join("whatever.json");
    write_json(
        &file,
        &rubric("rubric-a1-spoken-production", "spoken_production"),
    );
    let output = content_cli(&["validate", &path_str(&file), "--kind", "rubric"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let output = content_cli(&["validate", &path_str(&file)]);
    assert_eq!(output.status.code(), Some(1), "as a unit it must fail");
}
