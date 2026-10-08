#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use common::example_value;
use curriculum::validate::{
    FileReport, Finding, GrammarCheck, RuleCode, Severity, UnitOptions, WordLevels,
    validate_unit_document,
};
use serde_json::{Value, json};

fn check(document: &Value) -> FileReport {
    validate_unit_document("test.json", document, &UnitOptions::default()).report
}

fn error_codes(report: &FileReport) -> BTreeSet<RuleCode> {
    report.errors().map(|f| f.code).collect()
}

fn describe(report: &FileReport) -> String {
    report
        .findings
        .iter()
        .map(|f| format!("{} {} {} {}", f.code, f.severity_label(), f.path, f.message))
        .collect::<Vec<_>>()
        .join("\n")
}

trait SeverityLabel {
    fn severity_label(&self) -> &'static str;
}

impl SeverityLabel for Finding {
    fn severity_label(&self) -> &'static str {
        match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// Applies `mutate` to the example and expects errors of exactly `code`, one of them at `path`.
fn expect_only(code: RuleCode, path: &str, mutate: impl FnOnce(&mut Value)) {
    let mut unit = example_value();
    mutate(&mut unit);
    let report = check(&unit);
    assert_eq!(
        error_codes(&report),
        BTreeSet::from([code]),
        "wrong codes for {code} at {path}:\n{}",
        describe(&report)
    );
    assert!(
        report.errors().any(|f| f.path == path),
        "no {code} finding at {path}:\n{}",
        describe(&report)
    );
}

/// Like `expect_only`, and the message of that finding must contain `needle`.
fn expect_message(code: RuleCode, path: &str, needle: &str, mutate: impl FnOnce(&mut Value)) {
    let mut unit = example_value();
    mutate(&mut unit);
    let report = check(&unit);
    assert_eq!(
        error_codes(&report),
        BTreeSet::from([code]),
        "wrong codes for {code} ({needle}):\n{}",
        describe(&report)
    );
    assert!(
        report
            .errors()
            .any(|f| f.path == path && f.message.contains(needle)),
        "no {code} finding at {path} mentioning \"{needle}\":\n{}",
        describe(&report)
    );
}

fn remove_activity(unit: &mut Value, index: usize) {
    unit["activities"].as_array_mut().unwrap().remove(index);
}

#[test]
fn the_example_unit_has_zero_errors() {
    let report = check(&example_value());
    assert_eq!(report.error_count(), 0, "{}", describe(&report));
    assert!(report.passed());
}

#[test]
fn the_example_reports_its_warnings_and_what_it_skipped() {
    let report = check(&example_value());
    // The 18-word audio of the listening set is below the 25 to 60 words of A1.
    let warnings: Vec<&Finding> = report.warnings().collect();
    assert!(
        warnings
            .iter()
            .any(|f| f.code == RuleCode::W07 && f.path == "/activities/14"),
        "{}",
        describe(&report)
    );
    // Without a word list and a grammar checker, those three rules are said to be skipped.
    let skipped: Vec<RuleCode> = report.skipped.iter().map(|s| s.code).collect();
    assert_eq!(skipped, [RuleCode::W01, RuleCode::W02, RuleCode::W03]);
    assert!(report.passed(), "warnings must not fail the unit");
}

// E01

#[test]
fn e01_schema_violations_carry_their_pointer() {
    expect_only(RuleCode::E01, "/theme", |u| u["theme"] = json!("Bad Theme"));
    expect_only(RuleCode::E01, "/activities/3/type", |u| {
        u["activities"][3]["type"] = json!("jigsaw");
    });
    expect_only(RuleCode::E01, "/activities/2/answers", |u| {
        u["activities"][2]["answers"] = json!("am");
    });
}

#[test]
fn e01_a_missing_required_field_is_reported_at_its_parent() {
    expect_only(RuleCode::E01, "", |u| {
        u.as_object_mut().unwrap().remove("title");
    });
}

#[test]
fn e01_unparseable_documents_stop_before_the_other_rules() {
    let mut unit = example_value();
    unit["activities"][3]["type"] = json!("jigsaw");
    unit["sequence"] = json!(7); // would be E02 if the unit could be read
    let report = check(&unit);
    assert_eq!(error_codes(&report), BTreeSet::from([RuleCode::E01]));
}

// E02

#[test]
fn e02_id_must_match_level_and_sequence() {
    expect_only(RuleCode::E02, "/id", |u| u["sequence"] = json!(2));
    expect_only(RuleCode::E02, "/id", |u| u["id"] = json!("a2-u01"));
}

// E03

#[test]
fn e03_ids_are_unique_within_each_collection() {
    expect_only(RuleCode::E03, "/targets/vocabulary/12/id", |u| {
        let first = u["targets"]["vocabulary"][0].clone();
        u["targets"]["vocabulary"]
            .as_array_mut()
            .unwrap()
            .push(first);
    });
    expect_only(RuleCode::E03, "/targets/grammar/3/id", |u| {
        let first = u["targets"]["grammar"][0].clone();
        u["targets"]["grammar"].as_array_mut().unwrap().push(first);
    });
    expect_only(RuleCode::E03, "/targets/pronunciation/1/id", |u| {
        let first = u["targets"]["pronunciation"][0].clone();
        u["targets"]["pronunciation"]
            .as_array_mut()
            .unwrap()
            .push(first);
    });
    expect_only(RuleCode::E03, "/dialogues/1/id", |u| {
        let first = u["dialogues"][0].clone();
        u["dialogues"].as_array_mut().unwrap().push(first);
    });
    expect_only(RuleCode::E03, "/activities/4/id", |u| {
        u["activities"][4]["id"] = json!("a01-listen-question");
    });
}

#[test]
fn e03_objective_ids_are_unique() {
    expect_only(RuleCode::E03, "/objectives/5/id", |u| {
        u["objectives"][5]["id"] = json!("o1-greet");
        for index in [11, 15, 16] {
            u["activities"][index]["objective_ids"] = json!(["o1-greet"]);
        }
    });
}

// E04

#[test]
fn e04_objective_references_must_resolve() {
    expect_only(RuleCode::E04, "/activities/0/objective_ids/0", |u| {
        u["activities"][0]["objective_ids"] = json!(["o9-missing"]);
    });
}

#[test]
fn e04_every_objective_needs_an_activity() {
    // Renaming the objective leaves two things wrong: its old id is referenced, and the new
    // id is served by nothing. Both are E04.
    expect_only(RuleCode::E04, "/objectives/4", |u| {
        u["objectives"][4]["id"] = json!("o5-unused");
    });
}

// E05

#[test]
fn e05_mcq_answer_index_must_be_inside_options() {
    expect_only(RuleCode::E05, "/activities/1/answer_index", |u| {
        u["activities"][1]["answer_index"] = json!(3);
    });
}

#[test]
fn e05_mcq_is_reading_or_listening_not_both() {
    expect_only(RuleCode::E05, "/activities/12", |u| {
        u["activities"][12]["audio_text"] = json!("Hi. My name is Budi.");
    });
}

// E06

#[test]
fn e06_gap_count_equals_answer_lists() {
    expect_only(RuleCode::E06, "/activities/2/answers", |u| {
        u["activities"][2]["answers"]
            .as_array_mut()
            .unwrap()
            .push(json!(["x"]));
    });
    expect_only(RuleCode::E06, "/activities/2/answers", |u| {
        u["activities"][2]["text"] = json!("Hello! I ___ Arif. I am ___ Makassar ___.");
    });
}

// E07

#[test]
fn e07_reorder_answer_uses_exactly_the_tokens() {
    expect_only(RuleCode::E07, "/activities/3/answer", |u| {
        u["activities"][3]["answer"] = json!("My name is Dewi Putri");
    });
    expect_only(RuleCode::E07, "/activities/3/answer", |u| {
        u["activities"][3]["answer"] = json!("My name is Dewi Dewi");
    });
}

#[test]
fn e07_tokens_must_not_start_in_answer_order() {
    expect_only(RuleCode::E07, "/activities/3/tokens", |u| {
        u["activities"][3]["tokens"] = json!(["My", "name", "is", "Dewi"]);
    });
}

// E08

#[test]
fn e08_shadowing_dialogue_must_exist() {
    expect_only(RuleCode::E08, "/activities/8/dialogue_id", |u| {
        u["activities"][8]["dialogue_id"] = json!("d-missing");
    });
}

// E09

#[test]
fn e09_exactly_one_model_answer_per_band() {
    expect_only(RuleCode::E09, "/activities/9/model_answers", |u| {
        u["activities"][9]["model_answers"][0]["band"] = json!("at");
    });
}

#[test]
fn e09_min_words_below_max_words() {
    expect_only(RuleCode::E09, "/activities/9/max_words", |u| {
        u["activities"][9]["min_words"] = json!(40);
        u["activities"][9]["max_words"] = json!(40);
    });
}

#[test]
fn e09_the_at_answer_is_inside_the_limits() {
    // The at answer of a10 has 8 words.
    expect_only(RuleCode::E09, "/activities/9/model_answers/1/text", |u| {
        u["activities"][9]["max_words"] = json!(6);
    });
    expect_only(RuleCode::E09, "/activities/11/model_answers/1/text", |u| {
        u["activities"][11]["min_words"] = json!(9);
    });
}

// E10

#[test]
fn e10_roleplay_targets_exist_in_the_unit() {
    expect_only(RuleCode::E10, "/activities/10/target_grammar_ids/0", |u| {
        u["activities"][10]["target_grammar_ids"][0] = json!("g-nope");
    });
    expect_only(RuleCode::E10, "/activities/10/target_vocab_ids/1", |u| {
        u["activities"][10]["target_vocab_ids"][1] = json!("v-nope");
    });
}

// E11

#[test]
fn e11_checkpoint_ids_exist() {
    expect_only(RuleCode::E11, "/checkpoint/activity_ids/0", |u| {
        u["checkpoint"]["activity_ids"][0] = json!("a99-missing");
    });
}

#[test]
fn e11_checkpoint_activities_are_scored() {
    // a11 is a roleplay with scoring none.
    expect_only(RuleCode::E11, "/checkpoint/activity_ids/1", |u| {
        u["checkpoint"]["activity_ids"][1] = json!("a11-roleplay-classmate");
    });
}

// E12

#[test]
fn e12_generation_policy_grammar_exists() {
    expect_only(
        RuleCode::E12,
        "/generation_policy/allowed_grammar_ids/0",
        |u| {
            u["generation_policy"]["allowed_grammar_ids"][0] = json!("g-nope");
        },
    );
}

// E13

#[test]
fn e13_review_items_point_to_existing_targets() {
    expect_only(RuleCode::E13, "/review_items/0/ref", |u| {
        u["review_items"][0]["ref"] = json!("v-nope");
    });
    // A vocabulary id is not a grammar point.
    expect_only(RuleCode::E13, "/review_items/4/ref", |u| {
        u["review_items"][4]["ref"] = json!("v-from");
    });
    expect_only(RuleCode::E13, "/review_items/5/ref", |u| {
        u["review_items"][5]["kind"] = json!("grammar");
    });
}

// E14

#[test]
fn e14_a1_to_b1_need_indonesian_everywhere() {
    expect_only(RuleCode::E14, "/title/id", |u| {
        u["title"].as_object_mut().unwrap().remove("id");
    });
    expect_only(RuleCode::E14, "/activities/0/explanation/id", |u| {
        u["activities"][0]["explanation"]
            .as_object_mut()
            .unwrap()
            .remove("id");
    });
    expect_only(RuleCode::E14, "/presentation/0/examples/1/id", |u| {
        u["presentation"][0]["examples"][1]
            .as_object_mut()
            .unwrap()
            .remove("id");
    });
    expect_only(RuleCode::E14, "/objectives/2/can_do/id", |u| {
        u["objectives"][2]["can_do"]["id"] = json!("   ");
    });
    expect_only(
        RuleCode::E14,
        "/activities/13/questions/2/explanation/id",
        |u| {
            u["activities"][13]["questions"][2]["explanation"]
                .as_object_mut()
                .unwrap()
                .remove("id");
        },
    );
}

#[test]
fn e14_is_not_asked_from_b2() {
    let mut unit = example_value();
    unit["level"] = json!("B2");
    unit["id"] = json!("b2-u01");
    unit["title"].as_object_mut().unwrap().remove("id");
    let report = check(&unit);
    assert!(
        !report.findings.iter().any(|f| f.code == RuleCode::E14),
        "{}",
        describe(&report)
    );
}

// E15

#[test]
fn e15_tts_text_uses_only_allowed_characters() {
    expect_only(RuleCode::E15, "/dialogues/0/turns/0/text", |u| {
        u["dialogues"][0]["turns"][0]["text"] = json!("Good evening (hello)!");
    });
    expect_only(RuleCode::E15, "/activities/0/audio_text", |u| {
        u["activities"][0]["audio_text"] = json!("Good morning & hello");
    });
    expect_only(RuleCode::E15, "/activities/5/audio_text", |u| {
        u["activities"][5]["audio_text"] = json!("Where are you from / who are you?");
    });
    expect_only(RuleCode::E15, "/activities/6/text", |u| {
        u["activities"][6]["text"] = json!("Thank you, e.g. now.");
    });
    expect_only(RuleCode::E15, "/activities/14/audio_text", |u| {
        u["activities"][14]["audio_text"] = json!("Hello!\nMy name is Putu.");
    });
}

#[test]
fn e15_dialogue_lines_have_length_limits() {
    // 15 words at A1, where 12 is the limit.
    expect_message(
        RuleCode::E15,
        "/dialogues/0/turns/1/text",
        "15 words",
        |u| {
            u["dialogues"][0]["turns"][1]["text"] =
                json!("Hi Dewi my name is Arif and I am from Makassar and you are Dewi");
        },
    );
    // More than 200 characters.
    expect_message(
        RuleCode::E15,
        "/dialogues/0/turns/1/text",
        "characters",
        |u| {
            let long = "word ".repeat(45);
            u["dialogues"][0]["turns"][1]["text"] = json!(long.trim());
        },
    );
}

// E16

#[test]
fn e16_needs_fourteen_activities() {
    expect_message(RuleCode::E16, "/activities", "14 or more", |u| {
        // Highest index first, so earlier indexes stay valid.
        for index in [16, 12, 4, 3] {
            remove_activity(u, index);
        }
    });
}

#[test]
fn e16_listening_needs_two_activities_and_a_listening_set() {
    expect_message(RuleCode::E16, "/activities", "listening needs 2", |u| {
        u["activities"][0]["skill"] = json!("vocabulary");
        u["activities"][5]["skill"] = json!("vocabulary");
    });
    expect_message(RuleCode::E16, "/activities", "one listening_set", |u| {
        remove_activity(u, 14);
        u["checkpoint"]["activity_ids"][2] = json!("a01-listen-question");
    });
}

#[test]
fn e16_reading_needs_two_activities_and_a_reading_set() {
    expect_message(RuleCode::E16, "/activities", "reading needs 2", |u| {
        u["activities"][12]["skill"] = json!("vocabulary");
    });
    expect_message(RuleCode::E16, "/activities", "one reading_set", |u| {
        remove_activity(u, 13);
        u["checkpoint"]["activity_ids"][3] = json!("a13-read-budi");
    });
}

#[test]
fn e16_speaking_needs_guided_speaking_and_roleplay() {
    expect_message(RuleCode::E16, "/activities", "guided_speaking", |u| {
        remove_activity(u, 9);
        u["activities"][9]["scoring"] = json!("rubric"); // the roleplay, now at index 9
        u["checkpoint"]["activity_ids"][4] = json!("a11-roleplay-classmate");
    });
    expect_message(RuleCode::E16, "/activities", "needs a roleplay", |u| {
        remove_activity(u, 10);
    });
}

#[test]
fn e16_writing_needs_guided_writing_and_one_more_task() {
    expect_message(
        RuleCode::E16,
        "/activities",
        "guided_writing activity",
        |u| {
            remove_activity(u, 11);
            u["checkpoint"]["activity_ids"][5] = json!("a17-fix-missing-from");
        },
    );
    expect_message(RuleCode::E16, "/activities", "one more task", |u| {
        remove_activity(u, 16);
        remove_activity(u, 15);
        u["checkpoint"]["activity_ids"][6] = json!("a04-reorder-name");
    });
}

#[test]
fn e16_grammar_and_vocabulary_items() {
    expect_message(RuleCode::E16, "/activities", "grammar needs 2", |u| {
        u["activities"][2]["skill"] = json!("vocabulary");
        u["activities"][3]["skill"] = json!("vocabulary");
    });
    expect_message(RuleCode::E16, "/activities", "vocabulary needs 2", |u| {
        u["activities"][1]["skill"] = json!("grammar");
        u["activities"][4]["skill"] = json!("grammar");
    });
}

#[test]
fn e16_a1_to_b1_need_a_pronunciation_drill() {
    expect_message(RuleCode::E16, "/activities", "pronunciation drill", |u| {
        remove_activity(u, 7);
        remove_activity(u, 6);
    });
}

#[test]
fn e16_b2_needs_the_drill_in_odd_units_and_mediation_every_third() {
    let b2 = |sequence: u64| {
        let mut unit = example_value();
        unit["level"] = json!("B2");
        unit["sequence"] = json!(sequence);
        unit["id"] = json!(format!("b2-u{sequence:02}"));
        // No drill at all.
        remove_activity(&mut unit, 7);
        remove_activity(&mut unit, 6);
        check(&unit)
    };
    let messages = |report: &FileReport| -> Vec<String> {
        report
            .findings
            .iter()
            .filter(|f| f.code == RuleCode::E16)
            .map(|f| f.message.clone())
            .collect()
    };
    let odd = messages(&b2(1));
    assert!(
        odd.iter().any(|m| m.contains("pronunciation drill")),
        "{odd:?}"
    );
    assert!(!odd.iter().any(|m| m.contains("mediation")), "{odd:?}");
    let third = messages(&b2(3));
    assert!(third.iter().any(|m| m.contains("mediation")), "{third:?}");
    let even = messages(&b2(2));
    assert!(
        !even.iter().any(|m| m.contains("pronunciation drill")),
        "{even:?}"
    );
    assert!(!even.iter().any(|m| m.contains("mediation")), "{even:?}");
}

#[test]
fn e16_b2_needs_only_one_grammar_and_one_vocabulary_item() {
    let mut unit = example_value();
    unit["level"] = json!("B2");
    unit["id"] = json!("b2-u02");
    unit["sequence"] = json!(2);
    unit["activities"][3]["skill"] = json!("vocabulary"); // grammar 2 -> 1
    unit["activities"][4]["skill"] = json!("pronunciation"); // vocabulary 2 -> 2 (a02, a04)
    let report = check(&unit);
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == RuleCode::E16 && f.message.contains("grammar needs")),
        "{}",
        describe(&report)
    );
}

#[test]
fn e16_the_checkpoint_covers_each_skill() {
    expect_message(
        RuleCode::E16,
        "/checkpoint/activity_ids",
        "no reading",
        |u| {
            u["checkpoint"]["activity_ids"][3] = json!("a04-reorder-name");
        },
    );
    expect_message(
        RuleCode::E16,
        "/checkpoint/activity_ids",
        "no listening",
        |u| {
            u["checkpoint"]["activity_ids"][2] = json!("a05-match-phrases");
        },
    );
    expect_message(
        RuleCode::E16,
        "/checkpoint/activity_ids",
        "no speaking",
        |u| {
            u["checkpoint"]["activity_ids"][4] = json!("a05-match-phrases");
        },
    );
    expect_message(
        RuleCode::E16,
        "/checkpoint/activity_ids",
        "no writing",
        |u| {
            u["checkpoint"]["activity_ids"][5] = json!("a05-match-phrases");
            u["checkpoint"]["activity_ids"][6] = json!("a04-reorder-name");
        },
    );
}

#[test]
fn e16_the_checkpoint_writing_item_must_be_a_productive_task() {
    // Only an error_correction left for writing: it is writing, but not a productive task.
    expect_message(
        RuleCode::E16,
        "/checkpoint/activity_ids",
        "productive writing",
        |u| {
            u["checkpoint"]["activity_ids"][5] = json!("a05-match-phrases");
        },
    );
}

// E17

#[test]
fn e17_question_answer_index_is_inside_options() {
    expect_only(
        RuleCode::E17,
        "/activities/13/questions/0/answer_index",
        |u| {
            u["activities"][13]["questions"][0]["answer_index"] = json!(4);
        },
    );
    expect_only(
        RuleCode::E17,
        "/activities/14/questions/1/answer_index",
        |u| {
            u["activities"][14]["questions"][1]["answer_index"] = json!(3);
        },
    );
}

#[test]
fn e17_no_two_questions_share_a_stem() {
    expect_only(RuleCode::E17, "/activities/13/questions/1/stem", |u| {
        let stem = u["activities"][13]["questions"][0]["stem"].clone();
        u["activities"][13]["questions"][1]["stem"] = stem;
    });
    expect_only(RuleCode::E17, "/activities/14/questions/2/stem", |u| {
        u["activities"][14]["questions"][2]["stem"] = json!("  PUTU: \"i am from ___.\" ");
    });
}

#[test]
fn e17_a_listening_set_has_exactly_one_audio_source() {
    expect_only(RuleCode::E17, "/activities/14", |u| {
        u["activities"][14]["dialogue_id"] = json!("d-new-classmates");
    });
    expect_only(RuleCode::E17, "/activities/14", |u| {
        u["activities"][14]
            .as_object_mut()
            .unwrap()
            .remove("audio_text");
    });
}

#[test]
fn e17_the_listening_dialogue_exists() {
    expect_only(RuleCode::E17, "/activities/14/dialogue_id", |u| {
        let set = u["activities"][14].as_object_mut().unwrap();
        set.remove("audio_text");
        set.insert("dialogue_id".into(), json!("d-missing"));
    });
}

#[test]
fn e17_a_listening_set_may_use_a_dialogue() {
    let mut unit = example_value();
    let set = unit["activities"][14].as_object_mut().unwrap();
    set.remove("audio_text");
    set.insert("dialogue_id".into(), json!("d-new-classmates"));
    let report = check(&unit);
    assert_eq!(report.error_count(), 0, "{}", describe(&report));
}

// E18

#[test]
fn e18_reading_passage_length_follows_the_level() {
    expect_message(
        RuleCode::E18,
        "/activities/13/passage",
        "A1 passages have 60 to 100",
        |u| {
            u["activities"][13]["passage"] = json!("Hello. I am Sari. I am from Surabaya.");
        },
    );
    expect_message(RuleCode::E18, "/activities/13/passage", "words", |u| {
        u["activities"][13]["passage"] = json!("word ".repeat(101).trim());
    });
}

// E19

#[test]
fn e19_error_category_is_one_of_the_contract_categories() {
    expect_only(RuleCode::E19, "/activities/15/error_category", |u| {
        u["activities"][15]["error_category"] = json!("not_a_category");
    });
}

// E20

#[test]
fn e20_the_sentence_must_differ_from_the_accepted_answers() {
    expect_only(RuleCode::E20, "/activities/15/sentence", |u| {
        u["activities"][15]["sentence"] = json!("I am Dewi.");
    });
    // Equal after normalisation: case, spacing, final mark and contraction.
    expect_only(RuleCode::E20, "/activities/15/sentence", |u| {
        u["activities"][15]["sentence"] = json!("  i'M   dewi");
    });
}

// Warnings

#[test]
fn warnings_do_not_fail_a_unit() {
    let mut unit = example_value();
    unit["activities"][11]["max_words"] = json!(100);
    unit["activities"][11]["model_answers"][1]["text"] = json!("word ".repeat(60).trim());
    let report = check(&unit);
    assert!(report.warning_count() >= 2, "{}", describe(&report));
    assert_eq!(report.error_count(), 0, "{}", describe(&report));
    assert!(report.passed());
}

#[test]
fn w04_near_duplicate_activity_texts() {
    let mut unit = example_value();
    let stem = unit["activities"][1]["stem"].clone();
    unit["activities"][12]["stem"] = stem;
    let report = check(&unit);
    assert!(
        report
            .warnings()
            .any(|f| f.code == RuleCode::W04 && f.path == "/activities/12/stem"),
        "{}",
        describe(&report)
    );
    assert_eq!(report.error_count(), 0, "{}", describe(&report));
}

#[test]
fn w05_indonesian_text_far_longer_or_shorter_than_english() {
    let mut unit = example_value();
    unit["title"]["id"] = json!("Halo ".repeat(20).trim());
    unit["presentation"][1]["text"]["id"] = json!("Bukan sapaan.");
    let report = check(&unit);
    let paths: Vec<&str> = report
        .warnings()
        .filter(|f| f.code == RuleCode::W05)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["/presentation/1/text/id", "/title/id"],
        "{}",
        describe(&report)
    );
}

#[test]
fn w06_the_at_writing_answer_is_inside_the_level_range() {
    let mut unit = example_value();
    unit["activities"][11]["max_words"] = json!(100);
    unit["activities"][11]["model_answers"][1]["text"] = json!("word ".repeat(60).trim());
    let report = check(&unit);
    assert!(
        report
            .warnings()
            .any(|f| f.code == RuleCode::W06 && f.path == "/activities/11/model_answers/1/text"),
        "{}",
        describe(&report)
    );
}

#[test]
fn w07_the_listening_audio_is_inside_the_level_range() {
    let mut unit = example_value();
    unit["activities"][14]["audio_text"] = json!("word ".repeat(30).trim());
    let report = check(&unit);
    assert!(
        !report.warnings().any(|f| f.code == RuleCode::W07),
        "{}",
        describe(&report)
    );
    unit["activities"][14]["audio_text"] = json!("word ".repeat(61).trim());
    let report = check(&unit);
    assert!(
        report
            .warnings()
            .any(|f| f.code == RuleCode::W07 && f.path == "/activities/14"),
        "{}",
        describe(&report)
    );
}

#[test]
fn w01_and_w02_use_the_word_list() {
    let list = WordLevels::parse(
        "hello,A1\nhi,A1\ngoodbye,A1\nname,A1\nfrom,A2\nfine,A1\nclass,C1\nmeet,C1\n",
    )
    .unwrap();
    let options = UnitOptions {
        word_levels: Some(&list),
        grammar_check: None,
    };
    let report = validate_unit_document("test.json", &example_value(), &options).report;
    let w02: Vec<&str> = report
        .warnings()
        .filter(|f| f.code == RuleCode::W02)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(
        w02,
        ["/targets/vocabulary/9/level_tag"],
        "{}",
        describe(&report)
    );
    assert!(
        !report
            .skipped
            .iter()
            .any(|s| s.code == RuleCode::W01 || s.code == RuleCode::W02),
        "the rules ran, so they are not skipped"
    );
    assert_eq!(report.error_count(), 0, "{}", describe(&report));
}

#[test]
fn w01_flags_text_above_the_unit_level() {
    // Every word of the unit that the list knows is C2, and none is a unit target.
    let mut entries = String::new();
    for word in [
        "good",
        "morning",
        "afternoon",
        "evening",
        "night",
        "teacher",
        "student",
        "class",
        "message",
        "listen",
        "write",
        "read",
        "speak",
        "sentence",
        "choose",
        "match",
        "phrase",
        "meaning",
        "sound",
        "tongue",
        "teeth",
        "air",
        "stop",
    ] {
        entries.push_str(&format!("{word},C2\n"));
    }
    let list = WordLevels::parse(&entries).unwrap();
    let options = UnitOptions {
        word_levels: Some(&list),
        grammar_check: None,
    };
    let report = validate_unit_document("test.json", &example_value(), &options).report;
    assert!(
        report
            .warnings()
            .any(|f| f.code == RuleCode::W01 && f.path.is_empty()),
        "{}",
        describe(&report)
    );
}

struct FlagsAm;

impl GrammarCheck for FlagsAm {
    fn findings(&self, text: &str) -> Vec<String> {
        if text.contains("I am") {
            vec!["use the short form here".to_owned()]
        } else {
            Vec::new()
        }
    }
}

#[test]
fn w03_reports_grammar_checker_findings_in_at_answers_only() {
    let checker = FlagsAm;
    let options = UnitOptions {
        word_levels: None,
        grammar_check: Some(&checker),
    };
    let mut unit = example_value();
    // The above answer also contains "I am", but only the at answer is checked.
    unit["activities"][11]["model_answers"][2]["text"] = json!("I am Arif and I am from Makassar.");
    let report = validate_unit_document("test.json", &unit, &options).report;
    let paths: Vec<&str> = report
        .warnings()
        .filter(|f| f.code == RuleCode::W03)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["/activities/11/model_answers/1/text"],
        "{}",
        describe(&report)
    );
    assert!(!report.skipped.iter().any(|s| s.code == RuleCode::W03));
}

#[test]
fn the_error_categories_match_the_turn_analysis_contract() {
    let contract: Value = serde_json::from_slice(
        &std::fs::read(common::repo_root().join("contracts/turn_analysis.schema.json")).unwrap(),
    )
    .unwrap();
    let categories: Vec<&str> = contract["properties"]["turns"]["items"]["properties"]["errors"]
        ["items"]["properties"]["category"]["enum"]
        .as_array()
        .expect("the contract lists the categories")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(categories, curriculum::validate::ERROR_CATEGORIES);
}

#[test]
fn a_generated_item_can_be_checked_on_its_own() {
    use curriculum::{Activity, validate::validate_generated_item};
    let unit = example_value();
    let mut mcq: Activity = serde_json::from_value(unit["activities"][1].clone()).unwrap();
    assert!(validate_generated_item(&mcq).is_empty());
    if let Activity::Mcq(item) = &mut mcq {
        item.answer_index = 4;
    }
    let findings = validate_generated_item(&mcq);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, RuleCode::E05);
    assert_eq!(findings[0].path, "/answer_index");

    let mut gap: Activity = serde_json::from_value(unit["activities"][2].clone()).unwrap();
    if let Activity::GapFill(item) = &mut gap {
        item.answers.pop();
    }
    assert_eq!(validate_generated_item(&gap)[0].code, RuleCode::E06);

    let mut reorder: Activity = serde_json::from_value(unit["activities"][3].clone()).unwrap();
    if let Activity::Reorder(item) = &mut reorder {
        item.tokens = vec!["My".into(), "name".into(), "is".into(), "Dewi".into()];
    }
    assert_eq!(validate_generated_item(&reorder)[0].code, RuleCode::E07);
}

// Rule table

#[test]
fn every_error_rule_of_the_unit_has_a_test_above() {
    // A reminder that fails loudly if a rule is added to the code without a test here.
    let covered = [
        "E01", "E02", "E03", "E04", "E05", "E06", "E07", "E08", "E09", "E10", "E11", "E12", "E13",
        "E14", "E15", "E16", "E17", "E18", "E19", "E20",
    ];
    let unit_rules: Vec<&str> = RuleCode::ALL
        .iter()
        .map(|c| c.as_str())
        .filter(|c| c.starts_with('E'))
        .collect();
    assert_eq!(unit_rules, covered);
}
