//! Printing a run: a text report for people and a JSON report for machines.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use curriculum::validate::{FileReport, RuleCode, Severity};
use serde::Serialize;

use crate::Outcome;

/// The text report. One block per file with findings, then the rules that were not evaluated,
/// then a one-line summary.
pub fn text(outcome: &Outcome) -> String {
    let mut out = String::new();
    let report = &outcome.report;
    let files = report.units.iter().chain(report.catalogs.iter());
    for file in files {
        write_file(&mut out, file);
    }
    if !report.set.findings.is_empty() {
        let _ = writeln!(out, "across all files");
        write_findings(&mut out, &report.set);
        out.push('\n');
    }

    let skipped = distinct_skipped(outcome);
    if !skipped.is_empty() {
        let _ = writeln!(out, "rules not evaluated");
        for (code, reason) in skipped {
            let _ = writeln!(out, "  skipped {code}  {reason}");
        }
        out.push('\n');
    }

    if outcome.files_examined == 0 {
        let _ = writeln!(out, "no JSON files found, so nothing was validated");
    }
    let errors = report.error_count();
    let warnings = report.warning_count();
    let _ = writeln!(
        out,
        "{} file(s) checked: {} error(s), {} warning(s). {}",
        outcome.files_examined,
        errors,
        warnings,
        if errors == 0 { "PASS" } else { "FAIL" }
    );
    out
}

fn write_file(out: &mut String, file: &FileReport) {
    let id = file
        .id
        .as_deref()
        .map(|id| format!(" {id}"))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "{}  ({}{}) {} error(s), {} warning(s)",
        file.file,
        file.kind.as_str(),
        id,
        file.error_count(),
        file.warning_count()
    );
    write_findings(out, file);
    out.push('\n');
}

fn write_findings(out: &mut String, file: &FileReport) {
    for finding in &file.findings {
        let severity = match finding.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let path = if finding.path.is_empty() {
            "(document)"
        } else {
            finding.path.as_str()
        };
        let _ = writeln!(
            out,
            "  {severity:<7} {}  {path}  {}",
            finding.code, finding.message
        );
    }
}

/// Every skipped rule once, however many files it was skipped for.
fn distinct_skipped(outcome: &Outcome) -> BTreeSet<(RuleCode, String)> {
    let report = &outcome.report;
    report
        .units
        .iter()
        .chain(report.catalogs.iter())
        .chain(std::iter::once(&report.set))
        .flat_map(|file| file.skipped.iter())
        .map(|skipped| (skipped.code, skipped.reason.clone()))
        .collect()
}

#[derive(Serialize)]
struct JsonFile<'a> {
    #[serde(flatten)]
    report: &'a FileReport,
    passed: bool,
    errors: usize,
    warnings: usize,
}

#[derive(Serialize)]
struct JsonReport<'a> {
    passed: bool,
    errors: usize,
    warnings: usize,
    files_examined: usize,
    files: Vec<JsonFile<'a>>,
    /// Findings and skipped rules that belong to no single file.
    set: JsonFile<'a>,
}

fn json_file(report: &FileReport) -> JsonFile<'_> {
    JsonFile {
        report,
        passed: report.passed(),
        errors: report.error_count(),
        warnings: report.warning_count(),
    }
}

/// The JSON report: one object per file with `findings` and `skipped`, plus totals.
pub fn json(outcome: &Outcome) -> Result<String, serde_json::Error> {
    let report = &outcome.report;
    let body = JsonReport {
        passed: report.passed(),
        errors: report.error_count(),
        warnings: report.warning_count(),
        files_examined: outcome.files_examined,
        files: report
            .units
            .iter()
            .chain(report.catalogs.iter())
            .map(json_file)
            .collect(),
        set: json_file(&report.set),
    };
    serde_json::to_string_pretty(&body)
}
