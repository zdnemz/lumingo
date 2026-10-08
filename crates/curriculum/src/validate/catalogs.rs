//! Checks for the catalog files and the syllabus: JSON Schema first (K01), then the rules the
//! schema cannot say (K02), then, when the curriculum is declared complete, coverage (K03).
//!
//! The spec describes the catalogs in words and ships no schemas. The shapes are in
//! `curriculum/schema/` (rubric, anchors, placement_items, errors, pron_tips, topics and
//! syllabus) and were inferred from `docs/CURRICULUM_SPEC.md` sections 1, 7 and 9,
//! `docs/ASSESSMENT_SPEC.md` sections 5, 6, 7 and 10, and the roadmap tasks C-02 to C-05.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use serde_json::Value;

use super::report::{FileKind, FileReport, Finding, RuleCode};
use super::text::word_count;
use super::texts::localized_texts;
use super::unit_rules::ERROR_CATEGORIES;
use crate::model::Level;
use crate::schema::{SchemaChecker, SchemaLoadError};
use crate::syllabus::SYLLABUS_SCHEMA_JSON;

const RUBRIC_SCHEMA_JSON: &str = include_str!("../../../../curriculum/schema/rubric.schema.json");
const ANCHORS_SCHEMA_JSON: &str = include_str!("../../../../curriculum/schema/anchors.schema.json");
const PLACEMENT_SCHEMA_JSON: &str =
    include_str!("../../../../curriculum/schema/placement_items.schema.json");
const ERRORS_SCHEMA_JSON: &str = include_str!("../../../../curriculum/schema/errors.schema.json");
const PRON_TIPS_SCHEMA_JSON: &str =
    include_str!("../../../../curriculum/schema/pron_tips.schema.json");
const TOPICS_SCHEMA_JSON: &str = include_str!("../../../../curriculum/schema/topics.schema.json");

/// The 39 ARPAbet phonemes of English, without stress digits.
pub const ARPABET: [&str; 39] = [
    "AA", "AE", "AH", "AO", "AW", "AY", "B", "CH", "D", "DH", "EH", "ER", "EY", "F", "G", "HH",
    "IH", "IY", "JH", "K", "L", "M", "N", "NG", "OW", "OY", "P", "R", "S", "SH", "T", "TH", "UH",
    "UW", "V", "W", "Y", "Z", "ZH",
];

/// Folders a directory walk skips: they hold schemas, word lists, level checkpoints (whose
/// format is not defined yet) and build output, not units or catalogs.
pub const SKIPPED_DIRECTORIES: [&str; 6] = [
    "schema",
    "data",
    "checkpoints",
    "bindings",
    "node_modules",
    "target",
];

/// Guesses what a JSON file is from where it lives: `rubrics/`, `syllabus/`, `anchors.json`,
/// `errors.json`, `pron_tips.json`, `topics.json`, `placement/items.json`. Anything else is a unit.
pub fn detect_kind(path: &Path) -> FileKind {
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("");
    match (parent, file) {
        ("rubrics", _) => FileKind::Rubric,
        ("syllabus", _) => FileKind::Syllabus,
        ("placement", "items.json") => FileKind::Placement,
        (_, "anchors.json") => FileKind::Anchors,
        (_, "errors.json") => FileKind::Errors,
        (_, "pron_tips.json") => FileKind::PronTips,
        (_, "topics.json") => FileKind::Topics,
        _ => FileKind::Unit,
    }
}

fn schema_json(kind: FileKind) -> Option<&'static str> {
    match kind {
        FileKind::Unit => None,
        FileKind::Syllabus => Some(SYLLABUS_SCHEMA_JSON),
        FileKind::Rubric => Some(RUBRIC_SCHEMA_JSON),
        FileKind::Anchors => Some(ANCHORS_SCHEMA_JSON),
        FileKind::Placement => Some(PLACEMENT_SCHEMA_JSON),
        FileKind::Errors => Some(ERRORS_SCHEMA_JSON),
        FileKind::PronTips => Some(PRON_TIPS_SCHEMA_JSON),
        FileKind::Topics => Some(TOPICS_SCHEMA_JSON),
    }
}

fn checker(kind: FileKind) -> Result<&'static SchemaChecker, SchemaLoadError> {
    static CHECKERS: [OnceLock<Result<SchemaChecker, SchemaLoadError>>; 8] =
        [const { OnceLock::new() }; 8];
    let slot = FileKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
    let Some(json) = schema_json(kind) else {
        return Err(SchemaLoadError {
            name: kind.as_str().to_owned(),
            reason: "units are checked with validate_unit_document".to_owned(),
        });
    };
    CHECKERS[slot]
        .get_or_init(|| SchemaChecker::from_json(kind.as_str(), json))
        .as_ref()
        .map_err(Clone::clone)
}

/// K01 to K03 for one catalog or syllabus document. `complete` switches on the coverage rules
/// (K03), which only make sense once the catalog is meant to be finished.
pub fn validate_catalog(
    kind: FileKind,
    file: &str,
    document: &Value,
    complete: bool,
) -> FileReport {
    let mut report = FileReport {
        file: file.to_owned(),
        kind,
        id: document
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..FileReport::default()
    };
    let checker = match checker(kind) {
        Ok(checker) => checker,
        Err(err) => {
            report.push(Finding::new(RuleCode::K01, "", err.to_string()));
            return report;
        }
    };
    let issues = checker.issues(document);
    if !issues.is_empty() {
        for issue in issues {
            report.push(Finding::new(RuleCode::K01, issue.pointer, issue.message));
        }
        return report;
    }
    let mut out = Checks {
        report: &mut report,
        complete,
    };
    match kind {
        FileKind::Unit => {}
        FileKind::Syllabus => out.syllabus(document),
        FileKind::Rubric => out.rubric(document),
        FileKind::Anchors => out.anchors(document),
        FileKind::Placement => out.placement(document),
        FileKind::Errors => out.errors(document),
        FileKind::PronTips => out.pron_tips(document),
        FileKind::Topics => out.topics(document),
    }
    if !complete {
        report.skip(
            RuleCode::K03,
            "coverage is checked only with --complete, because a catalog under construction is not expected to cover everything yet",
        );
    }
    report
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

/// Whether the level named `level` needs Indonesian text (A1 to B1).
fn needs_indonesian(level: &str) -> bool {
    Level::parse(level).is_some_and(|l| l <= Level::B1)
}

struct Checks<'a> {
    report: &'a mut FileReport,
    complete: bool,
}

impl Checks<'_> {
    fn k02(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.report.push(Finding::new(RuleCode::K02, path, message));
    }

    fn k03(&mut self, message: impl Into<String>) {
        if self.complete {
            self.report.push(Finding::new(RuleCode::K03, "", message));
        }
    }

    /// K02 for every repeat of a string field inside one list.
    fn unique(&mut self, items: &[Value], key: &str, base: &str) {
        let mut seen = HashSet::new();
        for (i, item) in items.iter().enumerate() {
            if let Some(value) = text(item, key)
                && !seen.insert(value)
            {
                self.k02(
                    format!("{base}/{i}/{key}"),
                    format!("\"{value}\" is used twice in this list"),
                );
            }
        }
    }

    fn indonesian_required(&mut self, level: &str, localized: &Value, path: &str) {
        let missing = text(localized, "id").is_none_or(|t| t.trim().is_empty());
        if needs_indonesian(level) && missing {
            self.k02(
                format!("{path}/id"),
                "the Indonesian text is missing; levels A1 to B1 need it",
            );
        }
    }

    /// Model answers and word limits of a productive task, shared by anchors.
    fn production(&mut self, task: &Value, base: &str) {
        let answers = list(task, "model_answers");
        for band in ["below", "at", "above"] {
            let count = answers
                .iter()
                .filter(|a| text(a, "band") == Some(band))
                .count();
            if count != 1 {
                self.k02(
                    format!("{base}/model_answers"),
                    format!("needs exactly one {band} model answer, found {count}"),
                );
            }
        }
        let (Some(min), Some(max)) = (number(task, "min_words"), number(task, "max_words")) else {
            return;
        };
        if min >= max {
            self.k02(
                format!("{base}/max_words"),
                format!("min_words ({min}) must be below max_words ({max})"),
            );
        }
        if let Some((i, at)) = answers
            .iter()
            .enumerate()
            .find(|(_, a)| text(a, "band") == Some("at"))
        {
            let words = word_count(text(at, "text").unwrap_or(""));
            if (words as u64) < min || (words as u64) > max {
                self.k02(
                    format!("{base}/model_answers/{i}/text"),
                    format!("the at answer has {words} words, outside {min} to {max}"),
                );
            }
        }
    }

    fn syllabus(&mut self, doc: &Value) {
        let level = text(doc, "level").unwrap_or("");
        let entries = list(doc, "units");
        self.unique(entries, "id", "/units");
        let mut sequences = HashSet::new();
        for (i, entry) in entries.iter().enumerate() {
            let id = text(entry, "id").unwrap_or("");
            let sequence = number(entry, "sequence").unwrap_or(0);
            if !id.starts_with(&format!("{}-u", level.to_lowercase())) {
                self.k02(
                    format!("/units/{i}/id"),
                    format!("\"{id}\" does not belong to level {level}"),
                );
            }
            let expected = format!("{}-u{sequence:02}", level.to_lowercase());
            if id != expected {
                self.k02(
                    format!("/units/{i}/sequence"),
                    format!("sequence {sequence} does not match the id \"{id}\" (expected \"{expected}\")"),
                );
            }
            if !sequences.insert(sequence) {
                self.k02(
                    format!("/units/{i}/sequence"),
                    format!("sequence {sequence} is used twice"),
                );
            }
            if let Some(objectives) = entry.get("objectives")
                && let Some(can_do) = objectives.as_array()
            {
                for (o, objective) in can_do.iter().enumerate() {
                    if let Some(localized) = objective.get("can_do") {
                        self.indonesian_required(
                            level,
                            localized,
                            &format!("/units/{i}/objectives/{o}/can_do"),
                        );
                    }
                }
            }
            if let Some(title) = entry.get("title") {
                self.indonesian_required(level, title, &format!("/units/{i}/title"));
            }
        }
        if entries.len() != 30 {
            self.k03(format!(
                "the syllabus has {} entries, a level has 30",
                entries.len()
            ));
        }
    }

    fn rubric(&mut self, doc: &Value) {
        let (Some(id), Some(level)) = (text(doc, "id"), text(doc, "level")) else {
            return;
        };
        let in_id = id
            .strip_prefix("rubric-")
            .and_then(|rest| rest.split('-').next())
            .unwrap_or("");
        if !in_id.eq_ignore_ascii_case(level) {
            self.k02(
                "/level",
                format!("the id \"{id}\" names level {in_id} but level is {level}"),
            );
        }
    }

    fn anchors(&mut self, doc: &Value) {
        let anchors = list(doc, "anchors");
        self.unique(anchors, "id", "/anchors");
        for (i, anchor) in anchors.iter().enumerate() {
            let base = format!("/anchors/{i}");
            self.production(anchor, &base);
            if let (Some(level), Some(prompt)) = (text(anchor, "level"), anchor.get("prompt")) {
                self.indonesian_required(level, prompt, &format!("{base}/prompt"));
            }
        }
        if self.complete {
            let mut per_level: Vec<(Level, usize)> = Level::ALL
                .iter()
                .map(|level| {
                    let count = anchors
                        .iter()
                        .filter(|a| text(a, "level") == Some(level.as_str()))
                        .count();
                    (*level, count)
                })
                .collect();
            per_level.retain(|(_, count)| *count != 2);
            if anchors.len() != 12 || !per_level.is_empty() {
                let detail: Vec<String> = per_level
                    .iter()
                    .map(|(level, count)| format!("{level} has {count}"))
                    .collect();
                self.k03(format!(
                    "the anchor set needs 12 tasks, two per level; it has {} ({})",
                    anchors.len(),
                    detail.join(", ")
                ));
            }
        }
    }

    fn placement(&mut self, doc: &Value) {
        let items = list(doc, "items");
        let tasks = list(doc, "speaking_tasks");
        let mut seen: HashSet<&str> = HashSet::new();
        for (base, group) in [("/items", items), ("/speaking_tasks", tasks)] {
            for (i, item) in group.iter().enumerate() {
                if let Some(id) = text(item, "id")
                    && !seen.insert(id)
                {
                    self.k02(
                        format!("{base}/{i}/id"),
                        format!("\"{id}\" is used twice among items and speaking tasks"),
                    );
                }
            }
        }
        for (i, item) in items.iter().enumerate() {
            self.placement_item(item, &format!("/items/{i}"));
        }
        for (i, task) in tasks.iter().enumerate() {
            let base = format!("/speaking_tasks/{i}");
            if let (Some(min), Some(max)) = (number(task, "min_words"), number(task, "max_words"))
                && min >= max
            {
                self.k02(
                    format!("{base}/max_words"),
                    format!("min_words ({min}) must be below max_words ({max})"),
                );
            }
            if let (Some(level), Some(prompt)) = (text(task, "level"), task.get("prompt")) {
                self.indonesian_required(level, prompt, &format!("{base}/prompt"));
            }
        }
        if self.complete {
            if tasks.len() != 6 {
                self.k03(format!(
                    "the placement bank needs 6 speaking tasks, it has {}",
                    tasks.len()
                ));
            }
            for level in Level::ALL {
                for skill in ["listening", "reading", "grammar", "vocabulary"] {
                    let has = items.iter().any(|item| {
                        text(item, "level") == Some(level.as_str())
                            && text(item, "skill") == Some(skill)
                    });
                    if !has {
                        self.k03(format!("level {level} has no {skill} placement item"));
                    }
                }
            }
        }
    }

    fn placement_item(&mut self, item: &Value, base: &str) {
        let skill = text(item, "skill").unwrap_or("");
        let kind = text(item, "type").unwrap_or("");
        match kind {
            "mcq" => {
                let options = list(item, "options").len() as u64;
                if let Some(index) = number(item, "answer_index")
                    && index >= options
                {
                    self.k02(
                        format!("{base}/answer_index"),
                        format!("answer_index {index} is outside the {options} options"),
                    );
                }
                let has_passage = item.get("passage").is_some();
                let has_audio = item.get("audio_text").is_some();
                if has_passage && has_audio {
                    self.k02(base, "passage and audio_text are both present");
                }
                if skill == "listening" && !has_audio {
                    self.k02(base, "a listening item needs audio_text");
                }
                if skill == "reading" && !has_passage {
                    self.k02(base, "a reading item needs a passage");
                }
            }
            "gap_fill" => {
                let gaps = text(item, "text").map_or(0, |t| t.matches("___").count());
                let lists = list(item, "answers").len();
                if gaps != lists {
                    self.k02(
                        format!("{base}/answers"),
                        format!("the text has {gaps} gap(s) but there are {lists} answer list(s)"),
                    );
                }
            }
            "reorder" => {
                let answer = text(item, "answer").unwrap_or("");
                let mut expected: Vec<&str> = answer.split_whitespace().collect();
                let tokens: Vec<&str> = list(item, "tokens")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                let in_order = tokens == expected;
                let mut actual = tokens.clone();
                expected.sort_unstable();
                actual.sort_unstable();
                if expected != actual {
                    self.k02(
                        format!("{base}/answer"),
                        "answer does not use exactly the tokens",
                    );
                } else if in_order {
                    self.k02(
                        format!("{base}/tokens"),
                        "the tokens are already in answer order",
                    );
                }
            }
            _ => {}
        }
        if matches!(skill, "listening" | "reading") && kind != "mcq" {
            self.k02(
                format!("{base}/type"),
                format!("a {skill} item must be an mcq with audio_text or a passage"),
            );
        }
    }

    fn errors(&mut self, doc: &Value) {
        let entries = list(doc, "categories");
        self.unique(entries, "category", "/categories");
        for (i, entry) in entries.iter().enumerate() {
            if let Some(category) = text(entry, "category")
                && !ERROR_CATEGORIES.contains(&category)
            {
                self.k02(
                    format!("/categories/{i}/category"),
                    format!(
                        "\"{category}\" is not a category of contracts/turn_analysis.schema.json"
                    ),
                );
            }
        }
        let present: HashSet<&str> = entries
            .iter()
            .filter_map(|entry| text(entry, "category"))
            .collect();
        let missing: Vec<&str> = ERROR_CATEGORIES
            .iter()
            .copied()
            .filter(|category| !present.contains(category))
            .collect();
        if !missing.is_empty() {
            self.k03(format!(
                "{} categories of the turn analysis contract have no entry: {}",
                missing.len(),
                missing.join(", ")
            ));
        }
    }

    fn pron_tips(&mut self, doc: &Value) {
        let known = |symbol: &str| ARPABET.contains(&symbol);
        let mut pairs = HashSet::new();
        for (i, pair) in list(doc, "pairs").iter().enumerate() {
            let expected = text(pair, "expected").unwrap_or("");
            let heard = text(pair, "heard").unwrap_or("");
            if !known(expected) {
                self.k02(
                    format!("/pairs/{i}/expected"),
                    format!("\"{expected}\" is not an ARPAbet phoneme"),
                );
            }
            if heard != "none" && !known(heard) {
                self.k02(
                    format!("/pairs/{i}/heard"),
                    format!("\"{heard}\" is not an ARPAbet phoneme or none"),
                );
            }
            if !pairs.insert((expected, heard)) {
                self.k02(
                    format!("/pairs/{i}"),
                    format!("the pair {expected} heard as {heard} appears twice"),
                );
            }
        }
        let generic = list(doc, "generic");
        self.unique(generic, "phoneme", "/generic");
        for (i, tip) in generic.iter().enumerate() {
            let phoneme = text(tip, "phoneme").unwrap_or("");
            if !known(phoneme) {
                self.k02(
                    format!("/generic/{i}/phoneme"),
                    format!("\"{phoneme}\" is not an ARPAbet phoneme"),
                );
            }
        }
        let present: BTreeSet<&str> = generic
            .iter()
            .filter_map(|tip| text(tip, "phoneme"))
            .collect();
        let missing: Vec<&str> = ARPABET
            .iter()
            .copied()
            .filter(|phoneme| !present.contains(phoneme))
            .collect();
        if !missing.is_empty() {
            self.k03(format!(
                "{} phonemes have no generic tip: {}",
                missing.len(),
                missing.join(", ")
            ));
        }
    }

    fn topics(&mut self, doc: &Value) {
        for level in Level::ALL {
            let name = level.as_str();
            let Some(bank) = doc.get("levels").and_then(|levels| levels.get(name)) else {
                self.k03(format!("level {name} has no topic lists"));
                continue;
            };
            for list_name in ["conversations", "writing_prompts", "reading_topics"] {
                self.unique(
                    list(bank, list_name),
                    "id",
                    &format!("/levels/{name}/{list_name}"),
                );
            }
        }
        // Indonesian is required from A1 to B1: every localized text under those levels.
        for text in localized_texts(doc) {
            let level = text
                .pointer
                .strip_prefix("/levels/")
                .and_then(|rest| rest.split('/').next())
                .unwrap_or("");
            if needs_indonesian(level) && text.id.is_none_or(|id| id.trim().is_empty()) {
                self.k02(
                    format!("{}/id", text.pointer),
                    "the Indonesian text is missing; levels A1 to B1 need it",
                );
            }
        }
    }
}
