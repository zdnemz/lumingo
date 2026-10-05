#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use curriculum::Level;
use curriculum::validate::catalogs::{ARPABET, detect_kind, validate_catalog};
use curriculum::validate::{ERROR_CATEGORIES, FileKind, FileReport, RuleCode};
use serde_json::{Value, json};

fn describe(report: &FileReport) -> String {
    report
        .findings
        .iter()
        .map(|f| format!("{} {} {}", f.code, f.path, f.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn check(kind: FileKind, document: &Value, complete: bool) -> FileReport {
    validate_catalog(kind, "test.json", document, complete)
}

fn assert_clean(kind: FileKind, document: &Value, complete: bool) {
    let report = check(kind, document, complete);
    assert!(report.findings.is_empty(), "{}", describe(&report));
}

fn assert_finding(report: &FileReport, code: RuleCode, path: &str) {
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == code && f.path == path),
        "no {code} at {path}:\n{}",
        describe(report)
    );
}

fn only_codes(report: &FileReport, code: RuleCode) {
    assert!(
        !report.findings.is_empty() && report.findings.iter().all(|f| f.code == code),
        "expected only {code}:\n{}",
        describe(report)
    );
}

// Rubrics

fn bands() -> Value {
    json!((0..5)
        .map(|band| json!({"band": band, "description": {"en": format!("Band {band} in plain words.")}}))
        .collect::<Vec<_>>())
}

fn rubric(id: &str, level: &str, family: &str) -> Value {
    let mut dimensions = json!({
        "task_achievement": bands(), "range": bands(), "accuracy": bands(), "coherence": bands()
    });
    if family == "spoken_interaction" {
        dimensions["interaction"] = bands();
    }
    json!({
        "schema_version": "1.0", "id": id, "version": 1, "level": level,
        "task_family": family, "dimensions": dimensions
    })
}

#[test]
fn a_valid_rubric_passes() {
    assert_clean(
        FileKind::Rubric,
        &rubric("rubric-a1-spoken-production", "A1", "spoken_production"),
        false,
    );
    assert_clean(
        FileKind::Rubric,
        &rubric("rubric-b2-roleplay", "B2", "spoken_interaction"),
        true,
    );
}

#[test]
fn rubric_interaction_is_for_interactive_tasks_only() {
    let mut with = rubric("rubric-a1-spoken-production", "A1", "spoken_production");
    with["dimensions"]["interaction"] = bands();
    let report = check(FileKind::Rubric, &with, false);
    only_codes(&report, RuleCode::K01);
    assert_finding(&report, RuleCode::K01, "/dimensions");

    let mut without = rubric("rubric-a1-roleplay", "A1", "spoken_interaction");
    without["dimensions"]
        .as_object_mut()
        .unwrap()
        .remove("interaction");
    let report = check(FileKind::Rubric, &without, false);
    only_codes(&report, RuleCode::K01);
    assert_finding(&report, RuleCode::K01, "/dimensions");
}

#[test]
fn rubric_bands_come_in_order_and_there_are_five() {
    let mut doc = rubric("rubric-a1-spoken-production", "A1", "spoken_production");
    doc["dimensions"]["range"][2]["band"] = json!(3);
    let report = check(FileKind::Rubric, &doc, false);
    only_codes(&report, RuleCode::K01);
    assert_finding(&report, RuleCode::K01, "/dimensions/range/2/band");

    let mut short = rubric("rubric-a1-spoken-production", "A1", "spoken_production");
    short["dimensions"]["accuracy"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert_finding(
        &check(FileKind::Rubric, &short, false),
        RuleCode::K01,
        "/dimensions/accuracy",
    );
}

#[test]
fn a_rubric_id_must_agree_with_its_level() {
    let doc = rubric("rubric-a2-spoken-production", "A1", "spoken_production");
    let report = check(FileKind::Rubric, &doc, false);
    only_codes(&report, RuleCode::K02);
    assert_finding(&report, RuleCode::K02, "/level");
    assert_eq!(report.id.as_deref(), Some("rubric-a2-spoken-production"));
}

// Anchors

fn anchor(id: &str, level: &str, kind: &str) -> Value {
    json!({
        "id": id, "level": level, "type": kind,
        "prompt": {"en": "Introduce yourself.", "id": "Perkenalkan diri Anda."},
        "content_points": ["a greeting", "the name"],
        "rubric_id": "rubric-a1-spoken-production",
        "model_answers": [
            {"band": "below", "text": "Dewi."},
            {"band": "at", "text": "Hello. My name is Dewi. I am from Bandung."},
            {"band": "above", "text": "Good morning. My name is Dewi and I am from Bandung."}
        ],
        "off_task_answer": "I like football because it is a team sport.",
        "min_words": 5, "max_words": 40
    })
}

fn anchors(count_per_level: usize) -> Value {
    let mut list = Vec::new();
    for level in Level::ALL {
        for n in 0..count_per_level {
            list.push(anchor(
                &format!("anchor-{}-{n}", level.lowercase()),
                level.as_str(),
                if n == 0 {
                    "guided_speaking"
                } else {
                    "guided_writing"
                },
            ));
        }
    }
    json!({"schema_version": "1.0", "version": 1, "anchors": list})
}

#[test]
fn a_full_anchor_set_passes_in_complete_mode() {
    assert_clean(FileKind::Anchors, &anchors(2), true);
}

#[test]
fn anchors_need_one_answer_per_band_and_sane_limits() {
    let mut doc = anchors(2);
    doc["anchors"][0]["model_answers"][0]["band"] = json!("at");
    let report = check(FileKind::Anchors, &doc, false);
    only_codes(&report, RuleCode::K02);
    assert_finding(&report, RuleCode::K02, "/anchors/0/model_answers");

    let mut doc = anchors(2);
    doc["anchors"][1]["min_words"] = json!(40);
    assert_finding(
        &check(FileKind::Anchors, &doc, false),
        RuleCode::K02,
        "/anchors/1/max_words",
    );

    let mut doc = anchors(2);
    doc["anchors"][2]["max_words"] = json!(6);
    assert_finding(
        &check(FileKind::Anchors, &doc, false),
        RuleCode::K02,
        "/anchors/2/model_answers/1/text",
    );
}

#[test]
fn anchor_ids_are_unique_and_early_levels_have_indonesian() {
    let mut doc = anchors(2);
    doc["anchors"][3]["id"] = json!("anchor-a1-0");
    assert_finding(
        &check(FileKind::Anchors, &doc, false),
        RuleCode::K02,
        "/anchors/3/id",
    );

    let mut doc = anchors(2);
    doc["anchors"][0]["prompt"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert_finding(
        &check(FileKind::Anchors, &doc, false),
        RuleCode::K02,
        "/anchors/0/prompt/id",
    );

    // From B2 the Indonesian text is optional.
    let mut doc = anchors(2);
    doc["anchors"][6]["level"] = json!("B2");
    doc["anchors"][6]["prompt"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert!(check(FileKind::Anchors, &doc, false).passed());
}

#[test]
fn an_anchor_without_an_off_task_answer_is_a_schema_error() {
    let mut doc = anchors(2);
    doc["anchors"][0]
        .as_object_mut()
        .unwrap()
        .remove("off_task_answer");
    let report = check(FileKind::Anchors, &doc, false);
    only_codes(&report, RuleCode::K01);
    assert_finding(&report, RuleCode::K01, "/anchors/0");
}

#[test]
fn the_anchor_set_covers_two_tasks_per_level_when_complete() {
    let report = check(FileKind::Anchors, &anchors(1), true);
    only_codes(&report, RuleCode::K03);
    assert!(
        report.findings[0].message.contains("12 tasks"),
        "{}",
        describe(&report)
    );
    // The same file is fine while the set is being written.
    assert_clean_with_skip(&anchors(1));
}

fn assert_clean_with_skip(doc: &Value) {
    let report = check(FileKind::Anchors, doc, false);
    assert!(report.findings.is_empty(), "{}", describe(&report));
    assert!(report.skipped.iter().any(|s| s.code == RuleCode::K03));
}

// Placement

fn placement() -> Value {
    let mut items = Vec::new();
    for level in Level::ALL {
        let l = level.lowercase();
        items.push(json!({
            "id": format!("p-{l}-listen"), "type": "mcq", "level": level, "skill": "listening",
            "audio_text": "Hello. My name is Putu.", "stem": "Putu: \"My name is ___.\"",
            "options": ["Putu", "Budi", "Sari"], "answer_index": 0
        }));
        items.push(json!({
            "id": format!("p-{l}-read"), "type": "mcq", "level": level, "skill": "reading",
            "passage": "Hi! My name is Budi.", "stem": "Budi: \"My name is ___.\"",
            "options": ["Putu", "Budi", "Sari"], "answer_index": 1
        }));
        items.push(json!({
            "id": format!("p-{l}-grammar"), "type": "gap_fill", "level": level, "skill": "grammar",
            "text": "I ___ from Bali.", "answers": [["am"]]
        }));
        items.push(json!({
            "id": format!("p-{l}-vocab"), "type": "reorder", "level": level, "skill": "vocabulary",
            "tokens": ["name", "My", "is", "Dewi"], "answer": "My name is Dewi"
        }));
    }
    let tasks: Vec<Value> = Level::ALL
        .iter()
        .map(|level| {
            json!({
                "id": format!("s-{}", level.lowercase()), "level": level,
                "prompt": {"en": "Say hello.", "id": "Ucapkan halo."},
                "content_points": ["a greeting"], "rubric_id": "rubric-a1-spoken-production",
                "min_words": 3, "max_words": 30
            })
        })
        .collect();
    json!({"schema_version": "1.0", "version": 1, "items": items, "speaking_tasks": tasks})
}

#[test]
fn a_valid_placement_bank_passes() {
    assert_clean(FileKind::Placement, &placement(), true);
}

#[test]
fn placement_item_rules() {
    type Case = (
        &'static str,
        Box<dyn Fn(&mut Value)>,
        RuleCode,
        &'static str,
    );
    let cases: Vec<Case> = vec![
        (
            "answer outside options",
            Box::new(|d| d["items"][0]["answer_index"] = json!(4)),
            RuleCode::K02,
            "/items/0/answer_index",
        ),
        (
            "listening item without audio",
            Box::new(|d| {
                d["items"][0].as_object_mut().unwrap().remove("audio_text");
            }),
            RuleCode::K02,
            "/items/0",
        ),
        (
            "reading item without passage",
            Box::new(|d| {
                d["items"][1].as_object_mut().unwrap().remove("passage");
            }),
            RuleCode::K02,
            "/items/1",
        ),
        (
            "gap count",
            Box::new(|d| d["items"][2]["text"] = json!("I ___ from ___.")),
            RuleCode::K02,
            "/items/2/answers",
        ),
        (
            "reorder answer",
            Box::new(|d| d["items"][3]["answer"] = json!("My name is Budi")),
            RuleCode::K02,
            "/items/3/answer",
        ),
        (
            "reorder already ordered",
            Box::new(|d| d["items"][3]["tokens"] = json!(["My", "name", "is", "Dewi"])),
            RuleCode::K02,
            "/items/3/tokens",
        ),
        (
            "duplicate id",
            Box::new(|d| d["items"][5]["id"] = json!("p-a1-listen")),
            RuleCode::K02,
            "/items/5/id",
        ),
        (
            "speaking id clashes with an item",
            Box::new(|d| d["speaking_tasks"][0]["id"] = json!("p-a1-read")),
            RuleCode::K02,
            "/speaking_tasks/0/id",
        ),
        (
            "unknown item type",
            Box::new(|d| d["items"][2]["type"] = json!("match")),
            RuleCode::K01,
            "/items/2/type",
        ),
        (
            "seven speaking tasks",
            Box::new(|d| {
                let extra = d["speaking_tasks"][0].clone();
                d["speaking_tasks"].as_array_mut().unwrap().push(extra);
            }),
            RuleCode::K01,
            "/speaking_tasks",
        ),
    ];
    for (name, mutate, code, path) in cases {
        let mut doc = placement();
        mutate(&mut doc);
        let report = check(FileKind::Placement, &doc, false);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == code && f.path == path),
            "{name}: expected {code} at {path}\n{}",
            describe(&report)
        );
    }
}

#[test]
fn placement_needs_six_speaking_tasks_and_every_skill_per_level_when_complete() {
    let mut doc = placement();
    doc["speaking_tasks"].as_array_mut().unwrap().pop();
    doc["items"].as_array_mut().unwrap().remove(2); // the A1 grammar item
    let report = check(FileKind::Placement, &doc, true);
    only_codes(&report, RuleCode::K03);
    let text = describe(&report);
    assert!(text.contains("6 speaking tasks"), "{text}");
    assert!(text.contains("level A1 has no grammar"), "{text}");
}

// Error catalog

fn error_entry(category: &str) -> Value {
    json!({
        "category": category,
        "name": {"en": "Name", "id": "Nama"},
        "rule": {"en": "One rule.", "id": "Satu aturan."},
        "example": {"wrong": "I Dewi.", "right": "I am Dewi."},
        "explanation": {
            "simple": {"en": "Simple.", "id": "Sederhana."},
            "full": {"en": "Full.", "id": "Lengkap."}
        }
    })
}

fn errors_catalog() -> Value {
    let entries: Vec<Value> = ERROR_CATEGORIES.iter().map(|c| error_entry(c)).collect();
    json!({"schema_version": "1.0", "version": 1, "categories": entries})
}

#[test]
fn a_full_error_catalog_passes() {
    assert_clean(FileKind::Errors, &errors_catalog(), true);
}

#[test]
fn the_error_catalog_is_checked_against_the_contract_categories() {
    let mut doc = errors_catalog();
    doc["categories"][0]["category"] = json!("not_a_category");
    let report = check(FileKind::Errors, &doc, false);
    only_codes(&report, RuleCode::K02);
    assert_finding(&report, RuleCode::K02, "/categories/0/category");

    let mut doc = errors_catalog();
    doc["categories"][2]["category"] = json!(ERROR_CATEGORIES[1]);
    assert_finding(
        &check(FileKind::Errors, &doc, false),
        RuleCode::K02,
        "/categories/2/category",
    );
}

#[test]
fn every_category_needs_an_entry_when_complete() {
    let mut doc = errors_catalog();
    doc["categories"].as_array_mut().unwrap().truncate(26);
    let report = check(FileKind::Errors, &doc, true);
    only_codes(&report, RuleCode::K03);
    assert!(
        report.findings[0].message.contains("2 categories"),
        "{}",
        describe(&report)
    );
    assert!(report.findings[0].message.contains("unclear_meaning"));
}

#[test]
fn an_error_entry_needs_both_languages_and_both_explanations() {
    let mut doc = errors_catalog();
    doc["categories"][0]["name"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    let report = check(FileKind::Errors, &doc, false);
    only_codes(&report, RuleCode::K01);
    assert_finding(&report, RuleCode::K01, "/categories/0/name");

    let mut doc = errors_catalog();
    doc["categories"][1]["explanation"]
        .as_object_mut()
        .unwrap()
        .remove("full");
    assert_finding(
        &check(FileKind::Errors, &doc, false),
        RuleCode::K01,
        "/categories/1/explanation",
    );
}

// Pronunciation tips

fn pron_tips() -> Value {
    let generic: Vec<Value> = ARPABET
        .iter()
        .map(|p| {
            json!({
                "phoneme": p,
                "tip": {"en": "Tongue, lips, voice.", "id": "Lidah, bibir, suara."},
                "examples": ["one", "two"]
            })
        })
        .collect();
    json!({
        "schema_version": "1.0", "version": 1,
        "pairs": [
            {"expected": "TH", "heard": "T",
             "tip": {"en": "Put the tongue between the teeth.", "id": "Letakkan lidah di antara gigi."},
             "examples": ["thank", "three"]},
            {"expected": "CH", "heard": "none",
             "tip": {"en": "Say the end sound.", "id": "Ucapkan bunyi akhir."},
             "examples": ["watch", "much"]}
        ],
        "generic": generic
    })
}

#[test]
fn valid_pronunciation_tips_pass() {
    assert_clean(FileKind::PronTips, &pron_tips(), true);
}

#[test]
fn pronunciation_tip_rules() {
    let mut doc = pron_tips();
    doc["pairs"][0]["examples"] = json!(["thank"]);
    let report = check(FileKind::PronTips, &doc, false);
    only_codes(&report, RuleCode::K01);
    assert_finding(&report, RuleCode::K01, "/pairs/0/examples");

    let mut doc = pron_tips();
    doc["pairs"][0]["expected"] = json!("QQ");
    assert_finding(
        &check(FileKind::PronTips, &doc, false),
        RuleCode::K02,
        "/pairs/0/expected",
    );

    let mut doc = pron_tips();
    doc["pairs"][1] = doc["pairs"][0].clone();
    assert_finding(
        &check(FileKind::PronTips, &doc, false),
        RuleCode::K02,
        "/pairs/1",
    );

    let mut doc = pron_tips();
    doc["generic"][1]["phoneme"] = json!("AA");
    assert_finding(
        &check(FileKind::PronTips, &doc, false),
        RuleCode::K02,
        "/generic/1/phoneme",
    );

    let mut doc = pron_tips();
    doc["pairs"][0]["tip"].as_object_mut().unwrap().remove("id");
    assert_finding(
        &check(FileKind::PronTips, &doc, false),
        RuleCode::K01,
        "/pairs/0/tip",
    );
}

#[test]
fn every_phoneme_needs_a_generic_tip_when_complete() {
    let mut doc = pron_tips();
    doc["generic"].as_array_mut().unwrap().truncate(36);
    let report = check(FileKind::PronTips, &doc, true);
    only_codes(&report, RuleCode::K03);
    assert!(
        report.findings[0].message.contains("3 phonemes"),
        "{}",
        describe(&report)
    );
}

// Topics

fn topic_bank(level: &str) -> Value {
    let l = level.to_lowercase();
    json!({
        "conversations": [{
            "id": format!("c-{l}-1"),
            "title": {"en": "At the shop", "id": "Di toko"},
            "scenario": {"en": "You buy a book.", "id": "Anda membeli buku."},
            "tutor_role": "A shop assistant", "learner_role": "A customer",
            "goals": ["Ask for a book"]
        }],
        "writing_prompts": [{
            "id": format!("w-{l}-1"),
            "prompt": {"en": "Write to a friend.", "id": "Tulis untuk teman."},
            "reader": "a friend", "purpose": "to invite", "content_points": ["the day"]
        }],
        "reading_topics": [{
            "id": format!("r-{l}-1"), "title": {"en": "A small market", "id": "Pasar kecil"}
        }]
    })
}

fn topics() -> Value {
    let mut levels = serde_json::Map::new();
    for level in Level::ALL {
        levels.insert(level.as_str().to_owned(), topic_bank(level.as_str()));
    }
    json!({"schema_version": "1.0", "version": 1, "levels": levels})
}

#[test]
fn valid_topic_banks_pass() {
    assert_clean(FileKind::Topics, &topics(), true);
}

#[test]
fn topic_bank_rules() {
    let mut doc = topics();
    doc["levels"]["A1"]["conversations"][0]["scenario"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    let report = check(FileKind::Topics, &doc, false);
    only_codes(&report, RuleCode::K02);
    assert_finding(
        &report,
        RuleCode::K02,
        "/levels/A1/conversations/0/scenario/id",
    );

    // The same gap at C1 is allowed.
    let mut doc = topics();
    doc["levels"]["C1"]["conversations"][0]["scenario"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert!(check(FileKind::Topics, &doc, false).findings.is_empty());

    let mut doc = topics();
    let extra = doc["levels"]["A2"]["reading_topics"][0].clone();
    doc["levels"]["A2"]["reading_topics"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    assert_finding(
        &check(FileKind::Topics, &doc, false),
        RuleCode::K02,
        "/levels/A2/reading_topics/1/id",
    );

    let mut doc = topics();
    doc["levels"]["B1"]["writing_prompts"] = json!([]);
    assert_finding(
        &check(FileKind::Topics, &doc, false),
        RuleCode::K01,
        "/levels/B1/writing_prompts",
    );
}

#[test]
fn every_level_needs_its_topic_lists_when_complete() {
    let mut doc = topics();
    doc["levels"].as_object_mut().unwrap().remove("C2");
    let report = check(FileKind::Topics, &doc, true);
    only_codes(&report, RuleCode::K03);
    assert!(report.findings[0].message.contains("level C2"));
    assert!(check(FileKind::Topics, &doc, false).findings.is_empty());
}

// Syllabus

fn syllabus(entries: usize) -> Value {
    let units: Vec<Value> = (1..=entries)
        .map(|n| {
            json!({
                "id": format!("a1-u{n:02}"), "sequence": n,
                "title": {"en": "Title", "id": "Judul"}, "theme": "greetings",
                "objectives": [
                    {"skill": "listening", "can_do": {"en": "x", "id": "y"}},
                    {"skill": "writing", "can_do": {"en": "x", "id": "y"}}
                ],
                "grammar_ids": ["g-be"], "vocabulary_field": "greetings",
                "functions": ["greeting"], "pronunciation_focus": []
            })
        })
        .collect();
    json!({"schema_version": "1.0", "level": "A1", "units": units})
}

#[test]
fn a_syllabus_is_checked_for_ids_and_sequences() {
    assert_clean(FileKind::Syllabus, &syllabus(30), true);
    assert_clean(FileKind::Syllabus, &syllabus(10), false);

    let mut doc = syllabus(5);
    doc["units"][2]["sequence"] = json!(4);
    let report = check(FileKind::Syllabus, &doc, false);
    only_codes(&report, RuleCode::K02);
    assert_finding(&report, RuleCode::K02, "/units/2/sequence");

    let mut doc = syllabus(5);
    doc["units"][1]["id"] = json!("a2-u02");
    assert_finding(
        &check(FileKind::Syllabus, &doc, false),
        RuleCode::K02,
        "/units/1/id",
    );

    let mut doc = syllabus(5);
    doc["units"][0]["title"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert_finding(
        &check(FileKind::Syllabus, &doc, false),
        RuleCode::K02,
        "/units/0/title/id",
    );
}

#[test]
fn a_complete_syllabus_has_thirty_entries() {
    let report = check(FileKind::Syllabus, &syllabus(29), true);
    only_codes(&report, RuleCode::K03);
}

// Kinds

#[test]
fn files_are_classified_by_where_they_live() {
    let cases = [
        ("curriculum/units/a1/a1-u01.json", FileKind::Unit),
        ("curriculum/examples/a1-u01.example.json", FileKind::Unit),
        (
            "curriculum/catalogs/rubrics/a1-spoken.json",
            FileKind::Rubric,
        ),
        ("curriculum/syllabus/a1.json", FileKind::Syllabus),
        ("curriculum/catalogs/anchors.json", FileKind::Anchors),
        ("curriculum/catalogs/errors.json", FileKind::Errors),
        ("curriculum/catalogs/pron_tips.json", FileKind::PronTips),
        ("curriculum/catalogs/topics.json", FileKind::Topics),
        ("curriculum/placement/items.json", FileKind::Placement),
        ("elsewhere/items.json", FileKind::Unit),
    ];
    for (path, kind) in cases {
        assert_eq!(detect_kind(Path::new(path)), kind, "{path}");
    }
    for kind in FileKind::ALL {
        assert_eq!(FileKind::parse(kind.as_str()), Some(kind));
    }
}

#[test]
fn units_are_not_catalogs() {
    let report = check(FileKind::Unit, &json!({}), false);
    only_codes(&report, RuleCode::K01);
}

#[test]
fn the_arpabet_list_has_thirty_nine_distinct_symbols() {
    let unique: std::collections::HashSet<&str> = ARPABET.iter().copied().collect();
    assert_eq!(unique.len(), 39);
}
