//! `content-cli validate <dir> [--complete]`: loads every unit under `dir`,
//! runs the per-unit and cross-unit rules, prints one block per unit and exits
//! with 1 when there is any error. Warnings are listed and never fail the run.

use anyhow::{Result, bail};
use curriculum::{
    Diagnostic, LoadError, SetOptions, Severity, UnitLoader, load_dir, validate_set, validate_unit,
};
use std::{collections::BTreeMap, path::PathBuf, process::ExitCode};

const USAGE: &str = "usage: content-cli validate <dir> [--complete]\n  --complete  also require units 1 to 30 at every level (rule X01)";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

/// Returns whether the content is free of errors.
fn run() -> Result<bool> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("validate") {
        bail!(USAGE);
    }
    let (mut dir, mut complete) = (None, false);
    for a in args {
        match a.as_str() {
            "--complete" => complete = true,
            _ if dir.is_none() && !a.starts_with("--") => dir = Some(PathBuf::from(a)),
            _ => bail!(USAGE),
        }
    }
    let Some(dir) = dir else { bail!(USAGE) };

    let loader = UnitLoader::new();
    let mut units = Vec::new();
    let mut report: BTreeMap<String, Vec<Diagnostic>> = BTreeMap::new();
    let mut load_failures = 0;
    for (path, result) in load_dir(&loader, &dir).map_err(|e| anyhow::anyhow!("{e}"))? {
        match result {
            Ok(unit) => {
                report
                    .entry(unit.id.clone())
                    .or_default()
                    .extend(validate_unit(&unit));
                units.push(unit);
            }
            Err(e) => {
                load_failures += 1;
                println!("{}: E01 {}", path.display(), describe(&e));
            }
        }
    }
    for d in validate_set(
        &units,
        SetOptions {
            require_complete: complete,
        },
    ) {
        report
            .entry(if d.unit.is_empty() {
                "(set)".to_owned()
            } else {
                d.unit.clone()
            })
            .or_default()
            .push(d);
    }

    let (mut errors, mut warnings) = (load_failures, 0);
    for (unit, diagnostics) in &report {
        if diagnostics.is_empty() {
            println!("{unit}: ok");
            continue;
        }
        println!("{unit}:");
        for d in diagnostics {
            let kind = match d.severity {
                Severity::Error => {
                    errors += 1;
                    "error"
                }
                Severity::Warning => {
                    warnings += 1;
                    "warning"
                }
            };
            println!(
                "  {kind} {} {} {}",
                d.code,
                if d.path.is_empty() { "/" } else { &d.path },
                d.message
            );
        }
    }
    println!(
        "{} unit(s), {errors} error(s), {warnings} warning(s)",
        units.len() + load_failures
    );
    Ok(errors == 0)
}

fn describe(e: &LoadError) -> String {
    match e {
        LoadError::Schema(issues) => issues
            .iter()
            .take(5)
            .map(|i| {
                format!(
                    "{} {}",
                    if i.path.is_empty() { "/" } else { &i.path },
                    i.message.chars().take(120).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join(" | "),
        other => other.to_string(),
    }
}
