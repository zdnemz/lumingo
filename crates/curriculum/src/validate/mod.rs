//! The content validators of `docs/CURRICULUM_SPEC.md` section 6.
//!
//! * [`validate_unit_document`] runs E01 to E20 and W01 to W07 on one unit document.
//! * [`validate_set`] runs a whole folder of units and adds the rules across units, X01 to X06.
//! * [`check_activity_alone`] is the part of the rules that needs only one activity, so the
//!   tutor can check a generated `mcq`, `gap_fill` or `reorder` before a learner sees it.
//!
//! Nothing here reads the file system. The caller hands over parsed documents, word lists,
//! syllabi and rubric ids, and a rule whose input is missing is listed as skipped in the report.

pub mod cross;
mod report;
pub mod text;
mod texts;
mod unit_rules;
mod warnings;

use std::collections::{HashMap, HashSet};

use serde_json::Value;

pub use report::{Finding, RuleCode, Severity, Skipped, UnitReport};
pub use texts::{LocalizedAt, TextScope, localized_texts, unit_texts};
pub use unit_rules::{
    ERROR_CATEGORIES, check_activity_alone, check_unit, listening_range, reading_range,
    writing_range,
};
pub use warnings::{GrammarCheck, UnitOptions, WordLevels, WordListError, check_warnings};

use crate::model::{Activity, Level, Unit};
use crate::schema::{SchemaIssue, unit_checker};
use crate::syllabus::Syllabus;
use cross::{Located, UnitRef};

/// A unit document after validation: its report, and the typed unit when the document
/// could be read into the model at all.
#[derive(Debug)]
pub struct UnitCheck {
    pub report: UnitReport,
    pub unit: Option<Unit>,
}

/// Runs E01 to E20 and W01 to W07 on one parsed unit document. `file` only labels the report.
pub fn validate_unit_document(
    file: &str,
    document: &Value,
    options: &UnitOptions<'_>,
) -> UnitCheck {
    let mut report = UnitReport {
        file: file.to_owned(),
        unit_id: document
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..UnitReport::default()
    };

    let issues = match unit_checker() {
        Ok(checker) => checker.issues(document),
        Err(err) => {
            report.push(Finding::new(RuleCode::E01, "", err.to_string()));
            return UnitCheck { report, unit: None };
        }
    };

    let unit: Unit = match serde_json::from_value(document.clone()) {
        Ok(unit) => unit,
        Err(err) => {
            for issue in &issues {
                report.push(schema_finding(issue));
            }
            if issues.is_empty() {
                // The schema accepted a document the model cannot read: schema and model drifted.
                report.push(Finding::new(
                    RuleCode::E01,
                    "",
                    format!("the document does not fit the Rust model: {err}"),
                ));
            }
            return UnitCheck { report, unit: None };
        }
    };

    for issue in &issues {
        // The schema's own "exactly one of audio_text and dialogue_id" is rule E17.
        if !is_listening_source_issue(issue, &unit) {
            report.push(schema_finding(issue));
        }
    }
    for finding in check_unit(&unit, document) {
        report.push(finding);
    }
    check_warnings(&unit, document, options, &mut report);
    UnitCheck {
        report,
        unit: Some(unit),
    }
}

fn schema_finding(issue: &SchemaIssue) -> Finding {
    Finding::new(RuleCode::E01, issue.pointer.clone(), issue.message.clone())
}

fn is_listening_source_issue(issue: &SchemaIssue, unit: &Unit) -> bool {
    if issue.keyword != "oneOf" {
        return false;
    }
    let Some(index) = issue
        .pointer
        .strip_prefix("/activities/")
        .and_then(|rest| rest.parse::<usize>().ok())
    else {
        return false;
    };
    matches!(unit.activities.get(index), Some(Activity::ListeningSet(_)))
}

/// Checks a generated activity on its own, for the tutor. Paths are relative to the activity,
/// for example `/answer_index`.
pub fn validate_generated_item(activity: &Activity) -> Vec<Finding> {
    let mut out = Vec::new();
    check_activity_alone(activity, "", &mut out);
    out
}

/// One unit file handed to [`validate_set`]: the file label and what reading it gave.
pub struct UnitInput {
    pub file: String,
    /// The parsed document, or the reason the file could not be read or parsed.
    pub document: Result<Value, String>,
}

/// What [`validate_set`] needs besides the units.
#[derive(Clone, Copy)]
pub struct SetConfig<'a> {
    pub unit_options: UnitOptions<'a>,
    /// The inputs are a whole folder of units, so X01, X02, X04 and X05 make sense.
    pub whole_set: bool,
    /// Every level that has a unit must have all 30 (strict X01) and the syllabi must be
    /// covered (X05).
    pub complete: bool,
    /// Syllabi by level, for X03 and X05.
    pub syllabi: &'a HashMap<Level, Syllabus>,
    /// Ids found in `catalogs/rubrics/`, for X06. `None` when that folder does not exist.
    pub rubric_ids: Option<&'a HashSet<String>>,
}

/// The report of a whole run.
#[derive(Debug, Default)]
pub struct SetReport {
    /// One report per input file, in input order.
    pub units: Vec<UnitReport>,
    /// Findings and skipped rules that belong to no single unit.
    pub set: UnitReport,
}

impl SetReport {
    pub fn error_count(&self) -> usize {
        self.units
            .iter()
            .map(UnitReport::error_count)
            .sum::<usize>()
            + self.set.error_count()
    }

    pub fn warning_count(&self) -> usize {
        self.units
            .iter()
            .map(UnitReport::warning_count)
            .sum::<usize>()
            + self.set.warning_count()
    }

    /// True only when there is no error anywhere. This decides the exit code.
    pub fn passed(&self) -> bool {
        self.error_count() == 0
    }
}

/// Validates every input, then the rules across units.
pub fn validate_set(inputs: &[UnitInput], config: &SetConfig<'_>) -> SetReport {
    let mut report = SetReport::default();
    let mut typed: Vec<Option<Unit>> = Vec::with_capacity(inputs.len());
    for input in inputs {
        match &input.document {
            Ok(document) => {
                let check = validate_unit_document(&input.file, document, &config.unit_options);
                report.units.push(check.report);
                typed.push(check.unit);
            }
            Err(reason) => {
                report.units.push(UnitReport {
                    file: input.file.clone(),
                    unit_id: None,
                    findings: vec![Finding::new(RuleCode::E01, "", reason.clone())],
                    skipped: Vec::new(),
                });
                typed.push(None);
            }
        }
    }

    // Report index of each unit that could be read, in the order the cross rules see them.
    let report_index: Vec<usize> = typed
        .iter()
        .enumerate()
        .filter_map(|(i, unit)| unit.as_ref().map(|_| i))
        .collect();
    let refs: Vec<UnitRef<'_>> = report_index
        .iter()
        .filter_map(|&i| {
            typed[i].as_ref().map(|unit| UnitRef {
                file: inputs[i].file.as_str(),
                unit,
            })
        })
        .collect();

    let mut located: Vec<Located> = Vec::new();
    if config.whole_set {
        located.extend(cross::check_sequences(&refs, config.complete));
        located.extend(cross::check_prerequisites(&refs));
        located.extend(cross::check_repeated_sentences(&refs));
    } else {
        for code in [RuleCode::X01, RuleCode::X02, RuleCode::X04] {
            report
                .set
                .skip(code, "needs a folder of units, not a single file");
        }
    }

    if config.syllabi.is_empty() {
        report.set.skip(
            RuleCode::X03,
            "no syllabus file was found (syllabus/<level>.json)",
        );
    } else {
        located.extend(cross::check_against_syllabus(&refs, config.syllabi));
        let missing: Vec<&str> = refs
            .iter()
            .map(|u| u.unit.level)
            .filter(|level| !config.syllabi.contains_key(level))
            .map(Level::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !missing.is_empty() {
            report.set.skip(
                RuleCode::X03,
                format!("no syllabus file for level {}", missing.join(", ")),
            );
        }
    }

    if !config.whole_set {
        report
            .set
            .skip(RuleCode::X05, "needs a folder of units, not a single file");
    } else if !config.complete {
        report.set.skip(
            RuleCode::X05,
            "checked only with --complete, because a partial curriculum cannot cover its syllabus",
        );
    } else if config.syllabi.is_empty() {
        report.set.skip(
            RuleCode::X05,
            "no syllabus file was found (syllabus/<level>.json)",
        );
    } else {
        located.extend(cross::check_grammar_coverage(&refs, config.syllabi));
    }

    match config.rubric_ids {
        Some(ids) => located.extend(cross::check_rubric_ids(&refs, ids)),
        None => report
            .set
            .skip(RuleCode::X06, "no catalogs/rubrics folder was found"),
    }

    for item in located {
        match item.unit {
            Some(position) => report.units[report_index[position]].push(item.finding),
            None => report.set.push(item.finding),
        }
    }
    report
}
