//! Live check for S4-12: a two-draft writing workshop round against the real
//! provider. Owner-run, never in CI (PROMPT_CONTRACTS section 11: live calls
//! are manual).
//!
//! ```text
//! cargo run -p tutor-engine --example workshop_live
//! ```
//!
//! The provider comes from the four `TUTOR_LLM_*` environment variables or a
//! `.env` file in the working directory, exactly as the server reads them. The
//! protocol and model name are printed; the key never is. This is a tool, not
//! library code, so errors print and exit.
//!
//! The check writes a first draft with several mistakes, gets the three layers
//! that exist on this line (rule-based findings at once; T2 errors; the
//! comparison is for the second draft), then revises it and shows the fixed /
//! remaining / new classification. Rubric bands (layer three, T3) are S5-03's
//! and are not part of this run; ADR-051 records that.
//!
//! The probe runs first: this gateway ignores the native schema, and without
//! the probe the T2 calls would start at ladder level 1 and fail (the S4-11
//! lesson). `TUTOR_LLM_FORCE_LEVEL=1..4` forces a level for comparison.

use std::error::Error;

use llm_client::{Timeouts, load_env_profile};
use tokio_util::sync::CancellationToken;
use tutor_engine::{DraftRun, DraftSource, RuleChecker, Workshop, WorkshopConfig};

/// The first draft, with deliberate mistakes at A1.
const FIRST_DRAFT: &str = "I has a dog. He name is Bruno. Every day we go to the park and \
     he run very fast. Bruno like to play with a ball. I am very happy with he.";

/// The revision: some mistakes fixed, some kept, one new sentence with its own
/// mistake.
const SECOND_DRAFT: &str = "I have a dog. His name is Bruno. Every day we go to the park and \
     he runs very fast. Bruno likes to play with a ball. I am very happy with him. \
     Yesterday we was at the park all afternoon.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let Some(config) = load_env_profile(Some(std::path::Path::new(".env")))? else {
        eprintln!(
            "No provider configured. Set TUTOR_LLM_PROTOCOL, TUTOR_LLM_BASE_URL, \
             TUTOR_LLM_MODEL and TUTOR_LLM_API_KEY in the environment or in .env."
        );
        return Ok(());
    };
    let protocol = config.protocol.name();
    let model = config.model.clone();
    let client = llm_client::LlmClient::new(config, Timeouts::default())?;

    let mut workshop = Workshop::start(WorkshopConfig {
        level: curriculum::Level::A1,
        first_language: "Indonesian".to_owned(),
        source: DraftSource::Topic("My pet".to_owned()),
    })?;
    let mut checker = RuleChecker::new();
    let cancel = CancellationToken::new();

    println!("== live writing workshop: {protocol}, model {model} ==");
    println!("== two drafts, text only, no speech model ==");

    let caps = client.probe(cancel.clone()).await;
    println!(
        "probe: auth {}, stream {}, structured level {:?}, first token {:?} ms",
        caps.auth_ok, caps.stream_ok, caps.structured_level, caps.ttft_ms
    );
    if !caps.auth_ok {
        eprintln!("the provider did not accept the key; stopping the live check");
        return Ok(());
    }
    if caps.structured_level.is_none() {
        eprintln!(
            "warning: no structured-output ladder level worked in the probe; T2 calls will fail"
        );
    }
    if let Ok(forced) = std::env::var("TUTOR_LLM_FORCE_LEVEL")
        && let Some(level) = forced.parse().ok().and_then(llm_client::Level::from_number)
    {
        client.set_structured_level(level);
        println!("forced structured level {}", level.number());
    }

    let mut emit = |event: tutor_engine::UiEvent| {
        if let tutor_engine::UiEvent::TurnState { turn } = event {
            println!("  [state: {turn:?}]");
        }
    };

    // Draft one: layer one arrives with the submission, before any provider call.
    println!("\n-- draft 1 --\n{FIRST_DRAFT}");
    let first = workshop.submit_draft(FIRST_DRAFT, &mut checker, &mut emit)?;
    println!(
        "layer 1 (rule-based, offline): checked {}, {} finding(s)",
        first.rule.checked,
        first.rule.findings.len()
    );
    for finding in &first.rule.findings {
        println!(
            "  [{}..{}] {}: {}",
            finding.start, finding.end, finding.kind, finding.message
        );
    }
    let run = workshop.analyse(&client, &cancel, &mut emit).await?;
    let DraftRun::Analysed { outcome, .. } = run else {
        eprintln!("draft 1: the analysis did not run (provider unreachable)");
        return Ok(());
    };
    let errors_first = outcome
        .filtered
        .turns
        .first()
        .map_or(0, |entry| entry.errors.len());
    println!(
        "layer 2 (T2): ladder level {}, {errors_first} error(s), {} dropped",
        outcome.ladder_level, outcome.filtered.counts.dropped
    );
    if let Some(entry) = outcome.filtered.turns.first() {
        for error in &entry.errors {
            println!(
                "  {}: \"{}\" -> \"{}\" ({})",
                error.severity, error.quote, error.correction, error.category
            );
        }
    }

    // Draft two: the revision, compared with the first.
    println!("\n-- draft 2 (revision) --\n{SECOND_DRAFT}");
    let second = workshop.submit_draft(SECOND_DRAFT, &mut checker, &mut emit)?;
    println!(
        "layer 1 (rule-based, offline): checked {}, {} finding(s)",
        second.rule.checked,
        second.rule.findings.len()
    );
    for finding in &second.rule.findings {
        println!(
            "  [{}..{}] {}: {}",
            finding.start, finding.end, finding.kind, finding.message
        );
    }
    let run = workshop.analyse(&client, &cancel, &mut emit).await?;
    let DraftRun::Analysed {
        outcome,
        comparison,
    } = run
    else {
        eprintln!("the second analysis did not run");
        return Ok(());
    };
    println!(
        "layer 2 (T2): ladder level {}, {} error(s), {} dropped",
        outcome.ladder_level,
        outcome.filtered.turns[0].errors.len(),
        outcome.filtered.counts.dropped
    );
    match comparison {
        Some(comparison) => {
            println!(
                "revision: {} fixed, {} remaining, {} new",
                comparison.fixed(),
                comparison.remaining(),
                comparison.new.len()
            );
            for earlier in &comparison.earlier {
                println!(
                    "  {:?}: \"{}\" ({})",
                    earlier.resolution, earlier.error.quote, earlier.error.category
                );
            }
            for new in &comparison.new {
                println!("  new: \"{}\" ({})", new.quote, new.category);
            }
        }
        None => println!("no comparison: the first draft had no analysis"),
    }
    println!(
        "\nlayer 3 (rubric bands, T3): S5-03, not part of this build (ADR-051). \
         Layer 1 first findings: {}, second: {}",
        errors_first,
        outcome.filtered.turns[0].errors.len()
    );
    Ok(())
}
