#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::{HashMap, HashSet};

use common::example_value;
use curriculum::Level;
use curriculum::syllabus::{Syllabus, syllabus_from_value};
use curriculum::validate::{
    FileReport, RuleCode, SetConfig, SetReport, UnitInput, UnitOptions, validate_set,
};
use serde_json::{Value, json};

fn variant(sequence: u64) -> Value {
    let mut unit = example_value();
    unit["sequence"] = json!(sequence);
    unit["id"] = json!(format!("a1-u{sequence:02}"));
    unit
}

fn input(file: &str, document: Value) -> UnitInput {
    UnitInput {
        file: file.to_owned(),
        document: Ok(document),
    }
}

struct Run<'a> {
    whole_set: bool,
    complete: bool,
    syllabi: &'a HashMap<Level, Syllabus>,
    rubric_ids: Option<&'a HashSet<String>>,
}

fn run(inputs: &[UnitInput], options: &Run<'_>) -> SetReport {
    validate_set(
        inputs,
        &SetConfig {
            unit_options: UnitOptions::default(),
            whole_set: options.whole_set,
            complete: options.complete,
            syllabi: options.syllabi,
            rubric_ids: options.rubric_ids,
        },
    )
}

fn folder(inputs: &[UnitInput]) -> SetReport {
    let none = HashMap::new();
    run(
        inputs,
        &Run {
            whole_set: true,
            complete: false,
            syllabi: &none,
            rubric_ids: None,
        },
    )
}

fn codes(report: &FileReport) -> Vec<RuleCode> {
    report.errors().map(|f| f.code).collect()
}

fn syllabus(entries: &[(u64, &str, usize, &[&str])]) -> Syllabus {
    let units: Vec<Value> = entries
        .iter()
        .map(|(sequence, theme, objectives, grammar)| {
            let objectives: Vec<Value> = (0..*objectives)
                .map(|_| json!({"skill": "listening", "can_do": {"en": "x", "id": "y"}}))
                .collect();
            json!({
                "id": format!("a1-u{sequence:02}"),
                "sequence": sequence,
                "title": {"en": "Title", "id": "Judul"},
                "theme": theme,
                "objectives": objectives,
                "grammar_ids": grammar,
                "vocabulary_field": "greetings",
                "functions": ["greeting"],
                "pronunciation_focus": ["th"]
            })
        })
        .collect();
    syllabus_from_value(&json!({"schema_version": "1.0", "level": "A1", "units": units})).unwrap()
}

const EXAMPLE_GRAMMAR: [&str; 3] = ["g-be-i-am", "g-be-from", "g-question-chunks"];

fn a1(syllabus: Syllabus) -> HashMap<Level, Syllabus> {
    HashMap::from([(Level::A1, syllabus)])
}

#[test]
fn the_example_alone_passes_and_lists_every_rule_it_could_not_run() {
    let none = HashMap::new();
    let report = run(
        &[input("a1-u01.json", example_value())],
        &Run {
            whole_set: false,
            complete: false,
            syllabi: &none,
            rubric_ids: None,
        },
    );
    assert!(report.passed(), "{:?}", report.units[0].findings);
    let mut skipped: Vec<RuleCode> = report.set.skipped.iter().map(|s| s.code).collect();
    skipped.sort();
    assert_eq!(
        skipped,
        [
            RuleCode::X01,
            RuleCode::X02,
            RuleCode::X03,
            RuleCode::X04,
            RuleCode::X05,
            RuleCode::X06
        ]
    );
}

#[test]
fn a_folder_with_the_first_unit_passes() {
    let report = folder(&[input("a1-u01.json", example_value())]);
    assert!(report.passed(), "{:?}", report.units[0].findings);
}

#[test]
fn an_unreadable_file_is_an_e01_error_of_its_own() {
    let report = folder(&[
        input("a1-u01.json", example_value()),
        UnitInput {
            file: "broken.json".to_owned(),
            document: Err("not valid JSON at line 3, column 1".to_owned()),
        },
    ]);
    assert!(!report.passed());
    assert!(report.units[0].passed());
    assert_eq!(codes(&report.units[1]), [RuleCode::E01]);
}

// X01

#[test]
fn x01_sequences_have_no_gap() {
    let report = folder(&[
        input("a1-u01.json", variant(1)),
        input("a1-u03.json", variant(3)),
    ]);
    let gap: Vec<&str> = report
        .set
        .errors()
        .filter(|f| f.code == RuleCode::X01)
        .map(|f| f.message.as_str())
        .collect();
    assert_eq!(gap.len(), 1, "{gap:?}");
    assert!(
        gap[0].contains("level A1") && gap[0].contains("sequence 2"),
        "{}",
        gap[0]
    );
}

#[test]
fn x01_a_repeated_sequence_is_reported_on_the_second_unit() {
    let report = folder(&[
        input("first.json", variant(1)),
        input("second.json", variant(1)),
    ]);
    let repeat: Vec<&str> = report.units[1]
        .errors()
        .filter(|f| f.code == RuleCode::X01)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(repeat, ["/sequence"]);
    assert!(report.units[0].errors().all(|f| f.code != RuleCode::X01));
}

#[test]
fn x01_a_complete_curriculum_needs_all_thirty_sequences() {
    let none = HashMap::new();
    let inputs = [
        input("a1-u01.json", variant(1)),
        input("a1-u02.json", variant(2)),
    ];
    let report = run(
        &inputs,
        &Run {
            whole_set: true,
            complete: true,
            syllabi: &none,
            rubric_ids: None,
        },
    );
    let message = report
        .set
        .errors()
        .find(|f| f.code == RuleCode::X01)
        .map(|f| f.message.clone())
        .expect("a gap from 3 to 30");
    assert!(
        message.contains("1 to 30") && message.contains("30"),
        "{message}"
    );
    // The same two units are fine while the curriculum is still being written.
    assert!(
        folder(&inputs)
            .set
            .errors()
            .all(|f| f.code != RuleCode::X01)
    );
}

// X02

#[test]
fn x02_prerequisites_exist_and_come_earlier() {
    let mut second = variant(2);
    second["prerequisites"] = json!(["a1-u01"]);
    let report = folder(&[
        input("a1-u01.json", variant(1)),
        input("a1-u02.json", second),
    ]);
    assert!(report.units[1].errors().all(|f| f.code != RuleCode::X02));

    let mut missing = variant(2);
    missing["prerequisites"] = json!(["a1-u05"]);
    let report = folder(&[
        input("a1-u01.json", variant(1)),
        input("a1-u02.json", missing),
    ]);
    let x02: Vec<(&str, &str)> = report.units[1]
        .errors()
        .filter(|f| f.code == RuleCode::X02)
        .map(|f| (f.path.as_str(), f.message.as_str()))
        .collect();
    assert_eq!(x02.len(), 2, "{x02:?}");
    assert!(x02.iter().all(|(path, _)| *path == "/prerequisites/0"));
    assert!(x02.iter().any(|(_, m)| m.contains("does not come before")));
    assert!(
        x02.iter()
            .any(|(_, m)| m.contains("not among the validated units"))
    );
}

#[test]
fn x02_a_unit_is_not_its_own_prerequisite_and_levels_order_before_sequences() {
    let mut first = variant(1);
    first["prerequisites"] = json!(["a1-u01"]);
    let report = folder(&[input("a1-u01.json", first)]);
    assert!(report.units[0].errors().any(|f| f.code == RuleCode::X02));

    // A prerequisite from a lower level is earlier whatever its sequence.
    let mut b1 = example_value();
    b1["level"] = json!("A2");
    b1["id"] = json!("a2-u01");
    b1["prerequisites"] = json!(["a1-u30"]);
    let mut last_a1 = variant(1);
    last_a1["sequence"] = json!(30);
    last_a1["id"] = json!("a1-u30");
    let report = folder(&[input("a1-u30.json", last_a1), input("a2-u01.json", b1)]);
    assert!(report.units[1].errors().all(|f| f.code != RuleCode::X02));
}

// X03

#[test]
fn x03_the_unit_matches_its_syllabus_entry() {
    let good = a1(syllabus(&[(
        1,
        "greetings_and_introductions",
        6,
        &EXAMPLE_GRAMMAR,
    )]));
    let report = run(
        &[input("a1-u01.json", variant(1))],
        &Run {
            whole_set: true,
            complete: false,
            syllabi: &good,
            rubric_ids: None,
        },
    );
    assert!(report.units[0].errors().all(|f| f.code != RuleCode::X03));
    assert!(report.set.skipped.iter().all(|s| s.code != RuleCode::X03));
}

#[test]
fn x03_theme_objective_count_and_grammar_ids_are_compared() {
    let planned = a1(syllabus(&[(1, "family", 5, &["g-be-i-am", "g-other"])]));
    let report = run(
        &[input("a1-u01.json", variant(1))],
        &Run {
            whole_set: true,
            complete: false,
            syllabi: &planned,
            rubric_ids: None,
        },
    );
    let paths: Vec<&str> = report.units[0]
        .errors()
        .filter(|f| f.code == RuleCode::X03)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(paths, ["/theme", "/objectives", "/targets/grammar"]);
}

#[test]
fn x03_a_unit_without_a_syllabus_entry_is_an_error_and_a_level_without_a_syllabus_is_skipped() {
    let planned = a1(syllabus(&[(2, "family", 6, &EXAMPLE_GRAMMAR)]));
    let mut a2 = example_value();
    a2["level"] = json!("A2");
    a2["id"] = json!("a2-u01");
    let report = run(
        &[input("a1-u01.json", variant(1)), input("a2-u01.json", a2)],
        &Run {
            whole_set: true,
            complete: false,
            syllabi: &planned,
            rubric_ids: None,
        },
    );
    assert!(
        report.units[0]
            .errors()
            .any(|f| f.code == RuleCode::X03 && f.path == "/id")
    );
    let skipped = report
        .set
        .skipped
        .iter()
        .find(|s| s.code == RuleCode::X03)
        .expect("A2 has no syllabus");
    assert!(skipped.reason.contains("A2"), "{}", skipped.reason);
}

// X04

#[test]
fn x04_a_long_sentence_in_two_units_is_reported_on_the_later_unit() {
    let report = folder(&[
        input("a1-u01.json", variant(1)),
        input("a1-u02.json", variant(2)),
    ]);
    assert!(report.units[0].errors().all(|f| f.code != RuleCode::X04));
    let x04: Vec<&str> = report.units[1]
        .errors()
        .filter(|f| f.code == RuleCode::X04)
        .map(|f| f.message.as_str())
        .collect();
    assert!(!x04.is_empty());
    assert!(x04.iter().all(|m| m.contains("a1-u01")), "{x04:?}");
    // One finding per sentence, even when the sentence occurs twice in the later unit.
    let unique: HashSet<&&str> = x04.iter().collect();
    assert_eq!(unique.len(), x04.len());
}

// X05

#[test]
fn x05_grammar_must_be_taught_and_practised_twice() {
    let planned = a1(syllabus(&[
        (1, "greetings_and_introductions", 6, &EXAMPLE_GRAMMAR),
        (2, "family", 6, &["g-have-got"]),
    ]));
    let complete = |inputs: &[UnitInput]| {
        run(
            inputs,
            &Run {
                whole_set: true,
                complete: true,
                syllabi: &planned,
                rubric_ids: None,
            },
        )
    };
    let report = complete(&[input("a1-u01.json", variant(1))]);
    let messages: Vec<String> = report
        .set
        .errors()
        .filter(|f| f.code == RuleCode::X05)
        .map(|f| f.message.clone())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("\"g-have-got\"") && m.contains("taught in no unit"))
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("\"g-be-from\"") && m.contains("only one unit"))
    );

    // Two units that list the example grammar practise it twice.
    let report = complete(&[
        input("a1-u01.json", variant(1)),
        input("a1-u02.json", variant(2)),
    ]);
    let messages: Vec<String> = report
        .set
        .errors()
        .filter(|f| f.code == RuleCode::X05)
        .map(|f| f.message.clone())
        .collect();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("g-have-got"));
}

#[test]
fn x05_is_skipped_unless_the_curriculum_is_declared_complete() {
    let planned = a1(syllabus(&[(
        1,
        "greetings_and_introductions",
        6,
        &EXAMPLE_GRAMMAR,
    )]));
    let report = run(
        &[input("a1-u01.json", variant(1))],
        &Run {
            whole_set: true,
            complete: false,
            syllabi: &planned,
            rubric_ids: None,
        },
    );
    assert!(report.set.errors().all(|f| f.code != RuleCode::X05));
    let skipped = report
        .set
        .skipped
        .iter()
        .find(|s| s.code == RuleCode::X05)
        .expect("skipped");
    assert!(skipped.reason.contains("--complete"));
}

// X06

#[test]
fn x06_rubric_ids_must_exist() {
    let none = HashMap::new();
    let ids: HashSet<String> = HashSet::from(["rubric-a1-spoken-production".to_owned()]);
    let report = run(
        &[input("a1-u01.json", variant(1))],
        &Run {
            whole_set: true,
            complete: false,
            syllabi: &none,
            rubric_ids: Some(&ids),
        },
    );
    let paths: Vec<&str> = report.units[0]
        .errors()
        .filter(|f| f.code == RuleCode::X06)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["/activities/11/rubric_id"],
        "the written rubric is missing"
    );

    let both: HashSet<String> = HashSet::from([
        "rubric-a1-spoken-production".to_owned(),
        "rubric-a1-written-production".to_owned(),
    ]);
    let report = run(
        &[input("a1-u01.json", variant(1))],
        &Run {
            whole_set: true,
            complete: false,
            syllabi: &none,
            rubric_ids: Some(&both),
        },
    );
    assert!(report.passed());
}

#[test]
fn the_report_counts_errors_and_warnings_across_units_and_the_set() {
    let report = folder(&[
        input("a1-u01.json", variant(1)),
        input("a1-u03.json", variant(3)),
    ]);
    assert!(report.error_count() >= 2, "X01 gap and X04 repeats");
    assert!(report.warning_count() >= 2, "one W07 per unit");
    assert!(!report.passed());
}
