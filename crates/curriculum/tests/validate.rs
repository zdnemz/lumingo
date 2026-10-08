#![allow(clippy::unwrap_used)] // test helpers; clippy.toml only exempts #[test] functions

use curriculum::{
    Diagnostic, GrammarCheck, SetOptions, Severity, Unit, UnitLoader, UnitOptions, validate_set,
    validate_unit, validate_unit_with,
};
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("../../../curriculum/examples/a1-u01.example.json");

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn load(v: Value) -> Unit {
    UnitLoader::new()
        .load_value(v)
        .unwrap_or_else(|e| panic!("a semantic mutation must still pass the schema: {e}"))
}

fn codes(ds: &[Diagnostic]) -> Vec<&'static str> {
    ds.iter().map(|d| d.code).collect()
}

/// Applies `change` to the example and returns the diagnostics for it.
fn with(change: impl FnOnce(&mut Value)) -> Vec<Diagnostic> {
    let mut v = example();
    change(&mut v);
    validate_unit(&load(v))
}

fn activity<'a>(v: &'a mut Value, kind: &str) -> &'a mut Value {
    v["activities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|a| a["type"] == kind)
        .unwrap()
}

#[test]
fn the_example_unit_has_no_errors() {
    let ds = validate_unit(&load(example()));
    let errors: Vec<_> = ds
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

fn expect(code: &str, ds: Vec<Diagnostic>) {
    assert!(
        ds.iter().any(|d| d.code == code),
        "expected {code}, got {:?}",
        codes(&ds)
    );
}

#[test]
fn e02_the_id_must_match_level_and_sequence() {
    expect("E02", with(|v| v["sequence"] = json!(2)));
}

#[test]
fn e03_ids_are_unique_within_a_group() {
    expect(
        "E03",
        with(|v| v["activities"][1]["id"] = v["activities"][0]["id"].clone()),
    );
    expect(
        "E03",
        with(|v| v["targets"]["vocabulary"][1]["id"] = v["targets"]["vocabulary"][0]["id"].clone()),
    );
}

#[test]
fn e04_objectives_exist_and_are_served() {
    expect(
        "E04",
        with(|v| v["activities"][0]["objective_ids"] = json!(["no-such-objective"])),
    );
    // an objective that no activity serves: point every activity at another objective
    expect(
        "E04",
        with(|v| {
            for a in v["activities"].as_array_mut().unwrap() {
                let moved: Vec<Value> = a["objective_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| {
                        if o == "o6-write-intro" {
                            json!("o1-greet")
                        } else {
                            o.clone()
                        }
                    })
                    .collect();
                a["objective_ids"] = json!(moved);
            }
        }),
    );
}

#[test]
fn e05_mcq_answer_inside_options_and_one_of_passage_or_audio() {
    expect(
        "E05",
        with(|v| activity(v, "mcq")["answer_index"] = json!(4)),
    );
    expect(
        "E05",
        with(|v| {
            let a = activity(v, "mcq");
            a["passage"] = json!("A passage.");
            a["audio_text"] = json!("Some audio.");
        }),
    );
}

#[test]
fn e06_gaps_match_answer_lists() {
    expect(
        "E06",
        with(|v| {
            activity(v, "gap_fill")["answers"]
                .as_array_mut()
                .unwrap()
                .push(json!(["extra"]))
        }),
    );
}

#[test]
fn e07_reorder_tokens_and_answer() {
    expect(
        "E07",
        with(|v| activity(v, "reorder")["answer"] = json!("not the same words at all")),
    );
    expect(
        "E07",
        with(|v| {
            let a = activity(v, "reorder");
            let tokens: Vec<String> = a["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.as_str().unwrap().to_owned())
                .collect();
            a["answer"] = json!(tokens.join(" "));
        }),
    );
}

#[test]
fn e08_shadowing_needs_an_existing_dialogue() {
    expect(
        "E08",
        with(|v| activity(v, "shadowing")["dialogue_id"] = json!("missing-dialogue")),
    );
}

#[test]
fn e09_model_answers_and_word_limits() {
    expect(
        "E09",
        with(|v| activity(v, "guided_speaking")["model_answers"][0]["band"] = json!("at")),
    );
    expect(
        "E09",
        with(|v| {
            let a = activity(v, "guided_writing");
            a["min_words"] = json!(50);
            a["max_words"] = json!(40);
        }),
    );
    expect(
        "E09",
        with(|v| activity(v, "guided_writing")["max_words"] = json!(2)),
    );
}

#[test]
fn e10_roleplay_targets_exist() {
    expect(
        "E10",
        with(|v| activity(v, "roleplay")["target_grammar_ids"] = json!(["missing"])),
    );
    expect(
        "E10",
        with(|v| activity(v, "roleplay")["target_vocab_ids"] = json!(["missing"])),
    );
}

#[test]
fn e11_checkpoint_ids_exist_and_are_scored() {
    expect(
        "E11",
        with(|v| v["checkpoint"]["activity_ids"][0] = json!("missing")),
    );
    // the checkpoint of the example does not contain the roleplay, so put it there
    expect(
        "E11",
        with(|v| {
            let id = activity(v, "roleplay")["id"].clone();
            activity(v, "roleplay")["scoring"] = json!("none");
            v["checkpoint"]["activity_ids"]
                .as_array_mut()
                .unwrap()
                .push(id);
        }),
    );
}

#[test]
fn e12_and_e13_generation_policy_and_review_items_point_at_targets() {
    expect(
        "E12",
        with(|v| v["generation_policy"]["allowed_grammar_ids"] = json!(["missing"])),
    );
    expect(
        "E13",
        with(|v| v["review_items"][0]["ref"] = json!("missing")),
    );
}

#[test]
fn e14_early_levels_need_indonesian_everywhere() {
    expect(
        "E14",
        with(|v| {
            v["title"]
                .as_object_mut()
                .unwrap()
                .remove("id")
                .map(drop)
                .unwrap()
        }),
    );
    expect(
        "E14",
        with(|v| activity(v, "mcq")["instructions"]["id"] = json!("  ")),
    );
}

#[test]
fn e15_spoken_text_alphabet_and_line_length() {
    expect(
        "E15",
        with(|v| activity(v, "dictation")["audio_text"] = json!("See you (e.g. tomorrow)")),
    );
    expect(
        "E15",
        with(|v| {
            v["dialogues"][0]["turns"][0]["text"] =
                json!("one two three four five six seven eight nine ten eleven twelve thirteen")
        }),
    ); // A1: 13 words
    expect(
        "E15",
        with(|v| v["dialogues"][0]["turns"][0]["text"] = json!("a".repeat(201))),
    );
}

#[test]
fn e16_minimum_content_and_checkpoint_coverage() {
    expect(
        "E16",
        with(|v| {
            v["activities"]
                .as_array_mut()
                .unwrap()
                .retain(|a| a["type"] != "roleplay")
        }),
    );
    expect(
        "E16",
        with(|v| {
            v["activities"]
                .as_array_mut()
                .unwrap()
                .retain(|a| a["type"] != "listening_set")
        }),
    );
    // six activities, but none for listening, reading, speaking or writing
    expect(
        "E16",
        with(|v| {
            let ids: Vec<Value> = v["activities"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|a| {
                    matches!(
                        a["skill"].as_str(),
                        Some("vocabulary" | "grammar" | "pronunciation")
                    )
                })
                .map(|a| a["id"].clone())
                .take(6)
                .collect();
            assert_eq!(
                ids.len(),
                6,
                "the example has enough vocabulary, grammar and pronunciation items"
            );
            v["checkpoint"]["activity_ids"] = json!(ids);
        }),
    );
}

#[test]
fn e17_sets_have_valid_answers_unique_stems_and_one_audio_source() {
    expect(
        "E17",
        with(|v| activity(v, "reading_set")["questions"][0]["answer_index"] = json!(4)),
    );
    expect(
        "E17",
        with(|v| {
            let first = activity(v, "reading_set")["questions"][0]["stem"].clone();
            activity(v, "reading_set")["questions"][1]["stem"] = first;
        }),
    );
    // the schema already forbids both sources at once, so only a dangling dialogue can reach E17
    expect(
        "E17",
        with(|v| {
            let a = activity(v, "listening_set");
            a.as_object_mut().unwrap().remove("audio_text");
            a["dialogue_id"] = json!("missing");
        }),
    );
}

#[test]
fn e18_reading_passage_length_follows_the_level() {
    expect(
        "E18",
        with(|v| activity(v, "reading_set")["passage"] = json!("Far too short.")),
    );
}

#[test]
fn e19_and_e20_error_correction_rules() {
    expect(
        "E19",
        with(|v| activity(v, "error_correction")["error_category"] = json!("not_a_category")),
    );
    expect(
        "E20",
        with(|v| {
            let a = activity(v, "error_correction");
            a["sentence"] = json!("I am Rina.");
            a["accepted_answers"] = json!(["i am rina"]);
        }),
    );
    // contractions count as the same sentence
    expect(
        "E20",
        with(|v| {
            let a = activity(v, "error_correction");
            a["sentence"] = json!("I'm Rina");
            a["accepted_answers"] = json!(["I am Rina."]);
        }),
    );
}

#[test]
fn w05_to_w07_are_warnings_not_errors() {
    let ds = with(|v| {
        v["title"]["id"] = json!("x".repeat(200));
        v["title"]["en"] = json!("A title of a normal length here");
    });
    assert!(
        ds.iter()
            .any(|d| d.code == "W05" && d.severity == Severity::Warning)
    );
    let ds = with(|v| {
        let a = activity(v, "guided_writing");
        a["min_words"] = json!(1);
        a["max_words"] = json!(500);
        a["model_answers"][1]["text"] = json!("word ".repeat(100).trim());
    });
    assert!(
        ds.iter()
            .any(|d| d.code == "W06" && d.severity == Severity::Warning),
        "{:?}",
        codes(&ds)
    );
    let ds = with(|v| activity(v, "listening_set")["audio_text"] = json!("Too short."));
    assert!(
        ds.iter()
            .any(|d| d.code == "W07" && d.severity == Severity::Warning)
    );
}

/// A fake W03 checker: flags any text containing "I has".
struct FlagsIHas;

impl GrammarCheck for FlagsIHas {
    fn findings(&mut self, text: &str) -> Vec<String> {
        if text.contains("I has") {
            vec!["the verb must agree with the pronoun".to_owned()]
        } else {
            Vec::new()
        }
    }
}

/// Runs the per-unit rules with the fake checker linked.
fn with_checker(change: impl FnOnce(&mut Value)) -> Vec<Diagnostic> {
    let mut v = example();
    change(&mut v);
    let mut checker = FlagsIHas;
    validate_unit_with(
        &load(v),
        UnitOptions {
            grammar_check: Some(&mut checker),
        },
    )
}

#[test]
fn w03_reports_grammar_findings_in_at_answers_only() {
    // The above answer contains the flagged phrase too, but only `at` is checked.
    let ds = with_checker(|v| {
        let a = activity(v, "guided_speaking");
        a["model_answers"][1]["text"] = json!("Hello! I has Arif. I am from Makassar.");
        a["model_answers"][2]["text"] =
            json!("Good morning! I has Arif and I am from Makassar, in West Java.");
    });
    let w03: Vec<&Diagnostic> = ds.iter().filter(|d| d.code == "W03").collect();
    assert_eq!(w03.len(), 1, "{ds:#?}");
    assert_eq!(w03[0].severity, Severity::Warning);
    assert_eq!(w03[0].path, "/activities/9/model_answers/1/text");
    assert!(w03[0].message.contains("the verb must agree"), "{w03:?}");
}

#[test]
fn w03_without_a_checker_is_skipped_never_clean() {
    let ds = validate_unit(&load(example()));
    let w03: Vec<&Diagnostic> = ds.iter().filter(|d| d.code == "W03").collect();
    assert_eq!(w03.len(), 1, "{ds:#?}");
    assert_eq!(w03[0].severity, Severity::Skipped);
    assert!(
        w03[0].message.contains("no rule-based grammar checker"),
        "{w03:?}"
    );
    // with a checker linked the example is clean of W03: no skip, no finding
    let ds = with_checker(|_| {});
    assert!(ds.iter().all(|d| d.code != "W03"), "{ds:#?}");
}

#[test]
fn w04_the_example_unit_has_no_near_duplicates() {
    let ds = validate_unit(&load(example()));
    assert!(ds.iter().all(|d| d.code != "W04"), "{ds:#?}");
}

#[test]
fn w04_a_copied_stem_is_reported() {
    let ds = with(|v| {
        let stem = v["activities"][1]["stem"].clone();
        v["activities"][12]["stem"] = stem;
    });
    let w04: Vec<&Diagnostic> = ds.iter().filter(|d| d.code == "W04").collect();
    assert_eq!(w04.len(), 1, "{ds:#?}");
    assert_eq!(w04[0].severity, Severity::Warning);
    assert_eq!(w04[0].path, "/activities/12/stem");
    assert!(w04[0].message.contains("duplicates"), "{w04:?}");
    assert!(w04[0].message.contains("/activities/1/stem"), "{w04:?}");
}

#[test]
fn w04_nearly_equal_long_texts_are_reported() {
    // Put a one-word-changed copy of the reading passage into the earlier mcq:
    // the pair is reported once, at the later activity, naming both.
    let ds = with(|v| {
        let passage = v["activities"][13]["passage"]
            .as_str()
            .unwrap()
            .replace("Surabaya", "Semarang");
        v["activities"][12]["passage"] = json!(passage);
    });
    let w04: Vec<&Diagnostic> = ds.iter().filter(|d| d.code == "W04").collect();
    assert_eq!(w04.len(), 1, "{ds:#?}");
    assert_eq!(w04[0].severity, Severity::Warning);
    assert_eq!(w04[0].path, "/activities/13/passage");
    assert!(w04[0].message.contains("nearly"), "{w04:?}");
    assert!(w04[0].message.contains("/activities/12/passage"), "{w04:?}");
}

#[test]
fn w04_short_texts_are_only_compared_for_equality() {
    // "My name is Dewi" (four words) copied into another activity is caught...
    let ds = with(|v| v["activities"][15]["sentence"] = json!("My name is Dewi"));
    assert!(
        ds.iter()
            .any(|d| d.code == "W04" && d.path == "/activities/15/sentence"),
        "{ds:#?}"
    );
    // ...a two-word copy too, because equality needs no length...
    let ds = with(|v| v["activities"][16]["sentence"] = json!("I Dewi."));
    assert!(
        ds.iter()
            .any(|d| d.code == "W04" && d.path == "/activities/16/sentence"),
        "{ds:#?}"
    );
    // ...but a short near miss does not count as "nearly the same".
    let ds = with(|v| v["activities"][15]["sentence"] = json!("My name is Budi"));
    assert!(ds.iter().all(|d| d.code != "W04"), "{ds:#?}");
}

fn second_unit() -> Value {
    let mut v = example();
    v["id"] = json!("a1-u02");
    v["sequence"] = json!(2);
    v
}

#[test]
fn x01_a_complete_curriculum_has_units_1_to_30_per_level() {
    let units = vec![load(example())];
    assert!(
        validate_set(&units, SetOptions::default())
            .iter()
            .all(|d| d.code != "X01")
    );
    let ds = validate_set(
        &units,
        SetOptions {
            require_complete: true,
        },
    );
    assert!(
        ds.iter()
            .any(|d| d.code == "X01" && d.message.contains("A1 has no unit with sequence 2"))
    );
    assert!(
        ds.iter()
            .any(|d| d.code == "X01" && d.message.contains("C2"))
    );
}

#[test]
fn x02_prerequisites_exist_and_come_earlier() {
    let mut u2 = second_unit();
    u2["prerequisites"] = json!(["a1-u01"]);
    let mut u1 = example();
    u1["prerequisites"] = json!(["a1-u02"]); // points forward
    let ds = validate_set(&[load(u1), load(u2)], SetOptions::default());
    assert_eq!(ds.iter().filter(|d| d.code == "X02").count(), 1, "{ds:#?}");
    let mut missing = second_unit();
    missing["prerequisites"] = json!(["a1-u09"]);
    assert!(
        validate_set(&[load(missing)], SetOptions::default())
            .iter()
            .any(|d| d.code == "X02")
    );
}

#[test]
fn x04_a_long_sentence_must_not_appear_in_two_units() {
    let long = "Good morning everyone and welcome to our class today.";
    let with_line = |mut v: Value| {
        v["dialogues"][0]["turns"][1]["text"] = json!(long);
        v
    };
    let ds = validate_set(
        &[load(with_line(example())), load(with_line(second_unit()))],
        SetOptions::default(),
    );
    let x04: Vec<_> = ds.iter().filter(|d| d.code == "X04").collect();
    assert_eq!(x04.len(), 1, "{ds:#?}");
    assert_eq!(x04[0].unit, "a1-u02", "the later unit is the one reported");
    // short sentences are free to repeat, and the same unit may repeat itself
    let ds = validate_set(
        &[load(example()), load(second_unit())],
        SetOptions::default(),
    );
    assert!(ds.iter().all(|d| d.code != "X04"), "{ds:#?}");
    let mut twice = example();
    twice["dialogues"][0]["turns"][0]["text"] = json!(long);
    twice["dialogues"][0]["turns"][1]["text"] = json!(long);
    assert!(
        validate_set(&[load(twice)], SetOptions::default())
            .iter()
            .all(|d| d.code != "X04")
    );
}
