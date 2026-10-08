#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! One deliberately broken copy of the example unit per error rule, run through the real
//! command. Each must fail with exit code 1, with exactly its own rule code, and at the
//! JSON pointer of the broken value. The copies are made here by changing the loaded example.

mod common;

use std::collections::BTreeSet;

use common::{content_cli, example_value, json_report, scratch_dir, write_json};
use serde_json::{Value, json};

type Mutation = fn(&mut Value);

struct Case {
    code: &'static str,
    path: &'static str,
    mutate: Mutation,
}

fn remove(unit: &mut Value, index: usize) {
    unit["activities"].as_array_mut().unwrap().remove(index);
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            code: "E01",
            path: "/activities/2/answers",
            mutate: |u| u["activities"][2]["answers"] = json!("am"),
        },
        Case {
            code: "E02",
            path: "/id",
            mutate: |u| u["sequence"] = json!(2),
        },
        Case {
            code: "E03",
            path: "/activities/4/id",
            mutate: |u| u["activities"][4]["id"] = json!("a01-listen-question"),
        },
        Case {
            code: "E04",
            path: "/activities/0/objective_ids/0",
            mutate: |u| u["activities"][0]["objective_ids"] = json!(["o9-missing"]),
        },
        Case {
            code: "E05",
            path: "/activities/1/answer_index",
            mutate: |u| u["activities"][1]["answer_index"] = json!(3),
        },
        Case {
            code: "E06",
            path: "/activities/2/answers",
            mutate: |u| {
                u["activities"][2]["answers"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(["x"]));
            },
        },
        Case {
            code: "E07",
            path: "/activities/3/answer",
            mutate: |u| u["activities"][3]["answer"] = json!("My name is Dewi Putri"),
        },
        Case {
            code: "E08",
            path: "/activities/8/dialogue_id",
            mutate: |u| u["activities"][8]["dialogue_id"] = json!("d-missing"),
        },
        Case {
            code: "E09",
            path: "/activities/9/model_answers",
            mutate: |u| u["activities"][9]["model_answers"][0]["band"] = json!("at"),
        },
        Case {
            code: "E10",
            path: "/activities/10/target_grammar_ids/0",
            mutate: |u| u["activities"][10]["target_grammar_ids"][0] = json!("g-nope"),
        },
        Case {
            code: "E11",
            path: "/checkpoint/activity_ids/0",
            mutate: |u| u["checkpoint"]["activity_ids"][0] = json!("a99-missing"),
        },
        Case {
            code: "E12",
            path: "/generation_policy/allowed_grammar_ids/0",
            mutate: |u| u["generation_policy"]["allowed_grammar_ids"][0] = json!("g-nope"),
        },
        Case {
            code: "E13",
            path: "/review_items/0/ref",
            mutate: |u| u["review_items"][0]["ref"] = json!("v-nope"),
        },
        Case {
            code: "E14",
            path: "/title/id",
            mutate: |u| {
                u["title"].as_object_mut().unwrap().remove("id");
            },
        },
        Case {
            code: "E15",
            path: "/dialogues/0/turns/0/text",
            mutate: |u| u["dialogues"][0]["turns"][0]["text"] = json!("Good evening (hello)!"),
        },
        Case {
            code: "E16",
            path: "/activities",
            mutate: |u| {
                // No listening_set, with the checkpoint pointed at another listening item.
                remove(u, 14);
                u["checkpoint"]["activity_ids"][2] = json!("a01-listen-question");
            },
        },
        Case {
            code: "E17",
            path: "/activities/13/questions/0/answer_index",
            mutate: |u| u["activities"][13]["questions"][0]["answer_index"] = json!(4),
        },
        Case {
            code: "E18",
            path: "/activities/13/passage",
            mutate: |u| u["activities"][13]["passage"] = json!("Hello. I am Sari."),
        },
        Case {
            code: "E19",
            path: "/activities/15/error_category",
            mutate: |u| u["activities"][15]["error_category"] = json!("not_a_category"),
        },
        Case {
            code: "E20",
            path: "/activities/15/sentence",
            mutate: |u| u["activities"][15]["sentence"] = json!("I am Dewi."),
        },
    ]
}

#[test]
fn every_error_rule_has_a_broken_unit_that_fails_with_its_code_and_path() {
    let dir = scratch_dir("rules_one_per_code");
    let covered: Vec<&str> = cases().iter().map(|c| c.code).collect();
    let expected: Vec<String> = (1..=20).map(|n| format!("E{n:02}")).collect();
    assert_eq!(covered, expected, "one case per rule E01 to E20");

    for case in cases() {
        let mut unit = example_value();
        (case.mutate)(&mut unit);
        let file = dir.join(format!("{}.json", case.code.to_lowercase()));
        write_json(&file, &unit);

        let output = content_cli(&["validate", file.to_str().unwrap(), "--format", "json"]);
        assert_eq!(output.status.code(), Some(1), "{} must fail", case.code);
        let report = json_report(&output);
        let findings = report["files"][0]["findings"].as_array().unwrap();
        let error_codes: BTreeSet<&str> = findings
            .iter()
            .filter(|f| f["severity"] == "error")
            .map(|f| f["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            error_codes,
            BTreeSet::from([case.code]),
            "{}: wrong codes in {findings:?}",
            case.code
        );
        assert!(
            findings
                .iter()
                .any(|f| f["code"] == case.code && f["path"] == case.path),
            "{}: no finding at {} in {findings:?}",
            case.code,
            case.path
        );
    }
}

#[test]
fn the_four_skill_minimums_are_enforced_through_the_command() {
    let dir = scratch_dir("rules_four_skills");
    // One broken unit per skill, each missing what the table of spec section 4 asks for.
    let skills: Vec<(&str, Mutation, &str)> = vec![
        (
            "listening",
            |u| {
                u["activities"][0]["skill"] = json!("vocabulary");
                u["activities"][5]["skill"] = json!("vocabulary");
            },
            "listening needs 2",
        ),
        (
            "reading",
            |u| u["activities"][12]["skill"] = json!("vocabulary"),
            "reading needs 2",
        ),
        ("speaking", |u| remove(u, 10), "speaking needs a roleplay"),
        (
            "writing",
            |u| {
                remove(u, 16);
                remove(u, 15);
                u["checkpoint"]["activity_ids"][6] = json!("a04-reorder-name");
            },
            "writing needs one more task",
        ),
    ];
    for (skill, mutate, message) in skills {
        let mut unit = example_value();
        mutate(&mut unit);
        let file = dir.join(format!("{skill}.json"));
        write_json(&file, &unit);
        let output = content_cli(&["validate", file.to_str().unwrap(), "--format", "json"]);
        assert_eq!(output.status.code(), Some(1), "{skill}");
        let report = json_report(&output);
        let findings = report["files"][0]["findings"].as_array().unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f["code"] == "E16" && f["message"].as_str().unwrap().contains(message)),
            "{skill}: {findings:?}"
        );
    }
}
