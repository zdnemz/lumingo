//! `tutor-cli workshop`: a writing round — rule findings at once, structured
//! errors and rubric bands when a provider is reachable, a revised draft
//! compared with the earlier one, and a queue for the drafts that wait (S4-12).
//!
//! The engine is `tutor_engine::Workshop`: this module is the terminal around
//! it. It loads the task and the rubric, connects the provider, takes the drafts
//! from a script or from the keyboard, prints each layer as it arrives and
//! writes a result file.
//!
//! Every draft is practice: its attempts carry the origin `free_mode` and
//! `counts_toward_estimate = false`, so nothing here can move a level estimate.
//! A draft whose layers two and three need a provider that is not there is
//! stored and queued instead of being lost. The queue is the workshop's own
//! kind (`writing_draft`), which `unit score-pending` does not drain: it scores
//! `rubric_response` entries only. There is no drain command yet, so this
//! command never exits 3 — that code is reserved for one.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use assessment_engine::Level;
use curriculum::validate::WordLevels;
use llm_client::LlmError;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    DimensionStatus, DraftFeedback, DraftStatus, DraftSubmission, EngineError, Resolution,
    RubricCatalog, RubricResult, TaskFamily, TopicBank, Workshop, WorkshopConfig, WorkshopEnv,
    WorkshopRubric, WorkshopTask, clean_topic,
};

use crate::args::{LevelArg, WorkshopArgs};
use crate::chat::Failure;
use crate::exit::Exit;
use crate::unit::{Connection, ProviderChoice, connect, ensure_profile, open_database};

/// The default rubrics folder: the repository layout, like `chat`'s default
/// scenario. `--rubrics` overrides it.
const DEFAULT_RUBRICS: &str = "curriculum/catalogs/rubrics";
/// The topic bank sits next to the rubrics folder: `catalogs/topics.json`.
const TOPICS_FILE: &str = "topics.json";

fn other(error: impl Into<anyhow::Error>) -> Failure {
    Failure {
        exit: Exit::Failure,
        error: error.into(),
    }
}

/// The task of the round: what the learner was asked to write, and the rubric
/// the bands come from. Resolved before the session starts, so a bad task or a
/// bad script cannot leave a half-written session behind.
pub struct TaskInput {
    pub task: WorkshopTask,
    pub prompt_id: Option<String>,
    pub rubric: Option<WorkshopRubric>,
}

fn level_of(level: LevelArg) -> Level {
    match level {
        LevelArg::A1 => Level::A1,
        LevelArg::A2 => Level::A2,
        LevelArg::B1 => Level::B1,
        LevelArg::B2 => Level::B2,
        LevelArg::C1 => Level::C1,
        LevelArg::C2 => Level::C2,
    }
}

/// The task from the command line: the learner's own topic, or a prompt of the
/// bank. A topic without a bank entry has no content points, so X3 has nothing
/// to check — the same shape the app gives a typed topic.
fn task_of(
    topic: Option<&str>,
    prompt: Option<&str>,
    bank: Option<&TopicBank>,
    level: Level,
    rubric: Option<WorkshopRubric>,
) -> Result<TaskInput> {
    let (task, prompt_id) = match (topic, prompt) {
        (Some(text), _) => {
            let prompt = clean_topic(text);
            if prompt.is_empty() {
                return Err(anyhow!("the topic is empty"));
            }
            (
                WorkshopTask {
                    prompt,
                    content_points: Vec::new(),
                    min_words: None,
                },
                None,
            )
        }
        (None, Some(id)) => {
            let bank = bank.ok_or_else(|| {
                anyhow!(
                    "the topic bank is not installed; use --topic to write about your own subject"
                )
            })?;
            let found = bank.writing_prompt(level, id).ok_or_else(|| {
                anyhow!("the topic bank has no writing prompt `{id}` at the chosen level")
            })?;
            (
                WorkshopTask {
                    prompt: found.prompt.en.clone(),
                    content_points: found.content_points.clone(),
                    min_words: None,
                },
                Some(found.id.clone()),
            )
        }
        (None, None) => return Err(anyhow!("a topic or a prompt id is needed")),
    };
    Ok(TaskInput {
        task,
        prompt_id,
        rubric,
    })
}

/// Reads the topic bank next to the rubrics folder. Only `--prompt` needs it.
fn load_bank(rubrics_dir: &Path) -> Result<TopicBank> {
    let path = rubrics_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(TOPICS_FILE);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read the topic bank {}", path.display()))?;
    TopicBank::parse(&text)
        .with_context(|| format!("{} is not a usable topic bank", path.display()))
}

/// The rubrics folder, or an empty catalog with a note. A missing rubric is not
/// an error: the workshop then gives layers one and two only, and says so.
fn load_rubrics(dir: &Path) -> RubricCatalog {
    match RubricCatalog::load_dir(dir) {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!(
                "note: no rubrics were loaded from {}: {error}; the drafts get the rule findings and the errors only",
                dir.display()
            );
            RubricCatalog::default()
        }
    }
}

fn load_word_levels(args: &WorkshopArgs) -> Result<Option<Arc<WordLevels>>> {
    match &args.word_list {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read the word list {}", path.display()))?;
            Ok(Some(Arc::new(
                WordLevels::parse(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?,
            )))
        }
        None => Ok(None),
    }
}

/// The binary's entry point: prints the failure and returns its exit code, the
/// same shape `unit` and `probe` use.
pub async fn run_command(args: WorkshopArgs) -> Exit {
    match execute(&args).await {
        Ok(exit) => exit,
        Err(failure) => {
            eprintln!("error: {:#}", failure.error);
            failure.exit
        }
    }
}

pub async fn execute(args: &WorkshopArgs) -> Result<Exit, Failure> {
    let level = level_of(args.level);
    let rubrics_dir = args
        .rubrics
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_RUBRICS));
    let bank = match &args.prompt {
        Some(_) => Some(load_bank(&rubrics_dir).map_err(other)?),
        None => None,
    };
    let rubrics = load_rubrics(&rubrics_dir);
    let rubric = rubrics
        .for_family(level, TaskFamily::WrittenProduction)
        .map(|r| r.rubric.clone());
    if rubric.is_none() {
        eprintln!(
            "note: no written-production rubric was found in {}; the drafts get the rule findings and the errors only",
            rubrics_dir.display()
        );
    }
    let input = task_of(
        args.topic.as_deref(),
        args.prompt.as_deref(),
        bank.as_ref(),
        level,
        rubric,
    )
    .map_err(other)?;
    let word_levels = load_word_levels(args).map_err(other)?;
    let connection = connect(
        &ProviderChoice {
            offline: args.offline,
            provider: args.provider.as_deref(),
            providers_file: args.providers_file.as_deref(),
            data_dir: args.data_dir.as_deref(),
            timeout_ms: args.provider_timeout_ms,
        },
        "none: drafts are stored and queued",
    )
    .await;
    run(args, &input, word_levels, connection).await
}

/// The round itself, with the task resolved and the provider connected. The
/// tests call this directly with a fake client; the binary goes through
/// [`execute`].
pub async fn run(
    args: &WorkshopArgs,
    input: &TaskInput,
    word_levels: Option<Arc<WordLevels>>,
    connection: Connection,
) -> Result<Exit, Failure> {
    // A script that cannot be read is refused before a session exists.
    let scripted = match &args.script {
        Some(path) => Some(load_drafts(path).map_err(other)?),
        None => None,
    };
    let database_path = args.db.clone().unwrap_or_else(default_database);
    let db = open_database(&database_path).await?;
    let profile_id = ensure_profile(&db).await?;
    let now = tutor_engine::system_clock();
    let env = WorkshopEnv {
        client: connection.client.clone(),
        db,
        clock: now,
        model: connection.model.clone(),
        provider_profile_id: None,
        grammar: Some(Arc::new(tutor_engine::HarperCheck::new())),
        word_levels,
        provider_qualified: false,
    };
    let workshop = Workshop::start(
        env,
        WorkshopConfig {
            profile_id,
            level: level_of(args.level),
            first_language: args.first_language.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            prompt_id: input.prompt_id.clone(),
            rubric: input.rubric.clone(),
            task: input.task.clone(),
        },
    )
    .await
    .map_err(|e| other(anyhow!("{e}")))?;

    println!("workshop: {}", input.task.prompt);
    println!("provider: {}", connection.label);
    println!("database: {}", database_path.display());
    println!(
        "rubric: {}",
        input.rubric.as_ref().map_or_else(
            || "none: the drafts get the rule findings and the errors only".to_owned(),
            |rubric| format!(
                "{} v{} ({} dimension(s))",
                rubric.id,
                rubric.version,
                rubric.dimensions.len()
            ),
        )
    );

    let cancel = CancellationToken::new();
    let watcher = cancel.clone();
    let interrupt = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            watcher.cancel();
        }
    });
    let mut stdout = std::io::stdout();
    let played = match scripted {
        Some(texts) => {
            play(
                &workshop,
                Drafts::<BufReader<tokio::io::Empty>>::script(texts),
                &mut stdout,
                &cancel,
            )
            .await
        }
        None => {
            play(
                &workshop,
                Drafts::typed(BufReader::new(tokio::io::stdin())),
                &mut stdout,
                &cancel,
            )
            .await
        }
    };
    interrupt.abort();
    let drafts = match played {
        Ok(drafts) => drafts,
        Err(error) => {
            let _ = workshop.finish(true).await;
            if cancel.is_cancelled() {
                eprintln!("interrupted: {error:#}");
                return Ok(Exit::Interrupted);
            }
            return Err(other(error));
        }
    };
    workshop
        .finish(false)
        .await
        .map_err(|e| other(anyhow!("the session could not be closed: {e}")))?;

    let report = RoundReport {
        session_id: workshop.session_id(),
        task: input.task.clone(),
        prompt_id: input.prompt_id.clone(),
        rubric: input.rubric.clone(),
        drafts,
    };
    let waiting = report.awaiting();
    println!(
        "\n{} draft(s); {waiting} wait for a provider",
        report.drafts.len()
    );
    println!("every draft here is practice: none of it counts toward a level estimate");
    if waiting > 0 {
        println!(
            "their feedback is queued for when a provider is reachable; there is no drain command yet"
        );
    }
    if let Some(path) = &args.out {
        let body = serde_json::to_string_pretty(&result_json(&report, Some(&database_path)))
            .map_err(|e| other(anyhow!("{e}")))?;
        std::fs::write(path, body)
            .with_context(|| format!("cannot write {}", path.display()))
            .map_err(other)?;
        println!("result file: {}", path.display());
    }
    Ok(Exit::Done)
}

fn default_database() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    std::env::temp_dir().join(format!(
        "lumingo-workshop-{}-{stamp}.sqlite",
        std::process::id()
    ))
}

/// The file of scripted drafts: one JSON object, `{ "drafts": ["first", ...] }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftScript {
    drafts: Vec<String>,
}

fn load_drafts(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the script {}", path.display()))?;
    let script: DraftScript = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a valid drafts file", path.display()))?;
    if script.drafts.is_empty() {
        return Err(anyhow!("{} has no drafts", path.display()));
    }
    Ok(script.drafts)
}

/// Where the drafts come from: a script file, or lines typed at the terminal.
enum Drafts<R> {
    Script { texts: Vec<String>, used: usize },
    Typed { reader: R },
}

impl<R> Drafts<R> {
    fn script(texts: Vec<String>) -> Self {
        Self::Script { texts, used: 0 }
    }

    fn typed(reader: R) -> Self {
        Self::Typed { reader }
    }
}

async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<String>> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    Ok((read > 0).then(|| line.trim_end_matches(['\r', '\n']).to_owned()))
}

impl<R: AsyncBufRead + Unpin> Drafts<R> {
    /// The next draft, or `None` when there are no more. A typed draft runs to a
    /// blank line, because a draft is prose and one line is not a draft.
    async fn next(&mut self, number: usize, out: &mut impl Write) -> Result<Option<String>> {
        match self {
            Self::Script { texts, used } => {
                let text = texts.get(*used).cloned();
                if text.is_some() {
                    *used += 1;
                }
                Ok(text)
            }
            Self::Typed { reader } => {
                write!(
                    out,
                    "\ndraft {number} (end with a blank line, /quit to stop)> "
                )?;
                out.flush()?;
                let Some(first) = read_line(reader).await? else {
                    return Ok(None);
                };
                let trimmed = first.trim();
                if trimmed.is_empty() || trimmed == "/quit" {
                    return Ok(None);
                }
                let mut lines = vec![first];
                loop {
                    let Some(line) = read_line(reader).await? else {
                        break;
                    };
                    if line.trim().is_empty() || line.trim() == "/end" {
                        break;
                    }
                    lines.push(line);
                }
                Ok(Some(lines.join("\n")))
            }
        }
    }
}

/// One draft of the round, as the terminal and the result file see it.
pub struct RoundDraft {
    pub index: usize,
    pub submission: DraftSubmission,
    pub feedback: DraftFeedback,
}

/// Everything the result file needs about one round.
pub struct RoundReport {
    pub session_id: i64,
    pub task: WorkshopTask,
    pub prompt_id: Option<String>,
    pub rubric: Option<WorkshopRubric>,
    pub drafts: Vec<RoundDraft>,
}

impl RoundReport {
    /// Drafts whose feedback a provider still owes. Each of them holds one queue
    /// entry, so the count and the queue agree.
    pub fn awaiting(&self) -> usize {
        self.drafts
            .iter()
            .filter(|d| d.feedback.status != DraftStatus::Analysed)
            .count()
    }
}

/// Plays the drafts in order and prints each layer as it arrives.
///
/// A draft is stored and checked by rules before the provider is called, so the
/// first layer is on screen while the second one waits for the network. A
/// provider failure is not an error: the draft comes back queued and the round
/// goes on, so a run that loses the provider halfway keeps what it has.
async fn play<R: AsyncBufRead + Unpin, W: Write>(
    workshop: &Workshop,
    mut drafts: Drafts<R>,
    out: &mut W,
    cancel: &CancellationToken,
) -> Result<Vec<RoundDraft>> {
    let mut round = Vec::new();
    let mut index = 1;
    loop {
        if cancel.is_cancelled() {
            return Err(anyhow!("interrupted"));
        }
        let Some(text) = drafts.next(index, out).await? else {
            break;
        };
        let submission = workshop
            .submit_draft(&text)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        print_submission(index, &submission, out)?;
        let feedback = match workshop.analyse_draft(&submission, cancel).await {
            Ok(feedback) => feedback,
            Err(EngineError::Llm(LlmError::Cancelled)) => {
                return Err(anyhow!("interrupted"));
            }
            Err(error) => return Err(anyhow!("{error}")),
        };
        print_feedback(&feedback, out)?;
        round.push(RoundDraft {
            index,
            submission,
            feedback,
        });
        index += 1;
    }
    Ok(round)
}

fn print_submission<W: Write>(
    index: usize,
    submission: &DraftSubmission,
    out: &mut W,
) -> Result<()> {
    writeln!(out, "\ndraft {index}: {} word(s)", submission.words)?;
    if !submission.rule_checked {
        writeln!(out, "  rule check: skipped (no checker is linked)")?;
    } else if submission.rule_findings.is_empty() {
        writeln!(out, "  rule check: nothing to report")?;
    } else {
        writeln!(
            out,
            "  rule check: {} finding(s)",
            submission.rule_findings.len()
        )?;
        for finding in &submission.rule_findings {
            writeln!(out, "    - {finding}")?;
        }
    }
    if submission.below_minimum {
        writeln!(out, "  shorter than the task asks for")?;
    }
    Ok(())
}

fn resolution_name(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::Fixed => "fixed",
        Resolution::Remaining => "remaining",
    }
}

fn alarm_name(alarm: tutor_engine::Alarm) -> &'static str {
    match alarm {
        tutor_engine::Alarm::Range => "the range band is not supported by the word list",
        tutor_engine::Alarm::Accuracy => "the rule findings disagree with the accuracy band",
    }
}

fn print_feedback<W: Write>(feedback: &DraftFeedback, out: &mut W) -> Result<()> {
    match feedback.status {
        DraftStatus::Analysed => {}
        DraftStatus::RubricPending => {
            writeln!(
                out,
                "  the errors are in; the rubric bands wait for a provider"
            )?;
        }
        DraftStatus::Pending => {
            writeln!(
                out,
                "  waiting for a provider: the draft is stored and queued"
            )?;
        }
    }
    if let Some(record) = &feedback.analysis {
        if record.analysis.errors.is_empty() {
            writeln!(out, "  errors: none reported")?;
        } else {
            let dropped = if record.dropped > 0 {
                format!(", {} dropped by the checks", record.dropped)
            } else {
                String::new()
            };
            writeln!(out, "  {} error(s){dropped}", record.analysis.errors.len())?;
            for error in &record.analysis.errors {
                writeln!(
                    out,
                    "    - [{}] \"{}\" -> \"{}\" ({})",
                    error.category,
                    error.quote,
                    error.correction,
                    error.severity.as_str()
                )?;
            }
        }
    }
    if let Some(comparison) = &feedback.comparison {
        writeln!(
            out,
            "  against the earlier draft: {} fixed, {} remaining, {} new",
            comparison.fixed(),
            comparison.remaining(),
            comparison.new.len()
        )?;
        for earlier in &comparison.earlier {
            writeln!(
                out,
                "    {}: \"{}\" [{}]",
                resolution_name(earlier.resolution),
                earlier.error.quote,
                earlier.error.category
            )?;
        }
        for error in &comparison.new {
            writeln!(out, "    new: \"{}\" [{}]", error.quote, error.category)?;
        }
    }
    if let Some(rubric) = &feedback.rubric {
        let bands: Vec<String> = rubric
            .dimensions
            .iter()
            .map(|d| match d.band {
                Some(band) => format!("{} {band}", d.dimension.as_str()),
                None => format!("{} needs review", d.dimension.as_str()),
            })
            .collect();
        writeln!(
            out,
            "  bands: {} (confidence {:.2})",
            bands.join(", "),
            rubric.confidence
        )?;
        if !rubric.on_task {
            writeln!(out, "  off task: every band is 0, try the task again")?;
        }
        for alarm in &rubric.alarms {
            writeln!(out, "  alarm: {}", alarm_name(*alarm))?;
        }
        if !rubric.feedback_en.is_empty() {
            writeln!(out, "  feedback: {}", rubric.feedback_en)?;
        }
    }
    Ok(())
}

fn status_name(status: DraftStatus) -> &'static str {
    match status {
        DraftStatus::Analysed => "analysed",
        DraftStatus::RubricPending => "rubric_pending",
        DraftStatus::Pending => "pending",
    }
}

fn error_json(error: &tutor_engine::ErrorFinding) -> Value {
    json!({
        "category": error.category,
        "quote": error.quote,
        "correction": error.correction,
        "severity": error.severity.as_str(),
    })
}

fn comparison_json(comparison: &tutor_engine::DraftComparison) -> Value {
    json!({
        "earlier": comparison.earlier.iter().map(|earlier| json!({
            "quote": earlier.error.quote,
            "category": earlier.error.category,
            "resolution": match earlier.resolution {
                Resolution::Fixed => "fixed",
                Resolution::Remaining => "remaining",
            },
        })).collect::<Vec<_>>(),
        "new": comparison.new.iter().map(|error| json!({
            "quote": error.quote,
            "category": error.category,
        })).collect::<Vec<_>>(),
        "fixed": comparison.fixed(),
        "remaining": comparison.remaining(),
    })
}

fn rubric_json(result: &RubricResult) -> Value {
    json!({
        "on_task": result.on_task,
        "confidence": result.confidence,
        "alarms": result.alarms.iter().map(|a| alarm_name(*a)).collect::<Vec<_>>(),
        "dimensions": result.dimensions.iter().map(|d| json!({
            "dimension": d.dimension.as_str(),
            "band": d.band,
            "reason": d.reason,
            "status": match d.status {
                DimensionStatus::Scored => "scored",
                DimensionStatus::NeedsReview => "needs_review",
            },
            "capped": d.capped,
        })).collect::<Vec<_>>(),
        "feedback_en": result.feedback_en,
        "feedback_l1": result.feedback_l1,
    })
}

fn draft_json(draft: &RoundDraft) -> Value {
    let feedback = &draft.feedback;
    json!({
        "index": draft.index,
        "turn_id": draft.submission.turn_id,
        "turn_seq": draft.submission.turn_seq,
        "words": draft.submission.words,
        "below_minimum": draft.submission.below_minimum,
        "rule_checked": draft.submission.rule_checked,
        "rule_findings": draft.submission.rule_findings,
        "status": status_name(feedback.status),
        "dropped": feedback.analysis.as_ref().map_or(0, |r| r.dropped),
        "errors": feedback.analysis.as_ref().map(|r| {
            r.analysis.errors.iter().map(error_json).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "comparison": feedback.comparison.as_ref().map(comparison_json),
        "rubric": feedback.rubric.as_ref().map(rubric_json),
    })
}

/// Every resolved earlier error of the round, flattened across the drafts that
/// have a comparison, in draft order.
fn flattened(drafts: &[RoundDraft]) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut fixed = Vec::new();
    let mut remaining = Vec::new();
    let mut new = Vec::new();
    for draft in drafts {
        let Some(comparison) = &draft.feedback.comparison else {
            continue;
        };
        for earlier in &comparison.earlier {
            let entry = json!({
                "quote": earlier.error.quote,
                "category": earlier.error.category,
            });
            match earlier.resolution {
                Resolution::Fixed => fixed.push(entry),
                Resolution::Remaining => remaining.push(entry),
            }
        }
        for error in &comparison.new {
            new.push(json!({
                "quote": error.quote,
                "category": error.category,
            }));
        }
    }
    (fixed, remaining, new)
}

/// The result file: the task, the rubric and every draft with its rule
/// findings, errors, comparison and bands. Nothing here names a level.
pub fn result_json(report: &RoundReport, database: Option<&Path>) -> Value {
    let (fixed, remaining, new) = flattened(&report.drafts);
    json!({
        "session_id": report.session_id,
        "database": database.map(|p| p.display().to_string()),
        "task": {
            "prompt": report.task.prompt,
            "prompt_id": report.prompt_id,
            "content_points": report.task.content_points,
            "min_words": report.task.min_words,
        },
        "rubric": report.rubric.as_ref().map(|rubric| json!({
            "id": rubric.id,
            "version": rubric.version,
            "dimensions": rubric.dimensions.iter().map(|d| d.dimension.as_str()).collect::<Vec<_>>(),
        })),
        "drafts": report.drafts.iter().map(draft_json).collect::<Vec<_>>(),
        "fixed": fixed,
        "remaining": remaining,
        "new": new,
        "awaiting_feedback": report.awaiting(),
        "counts_toward_estimate": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANK: &str = r#"{
        "schema_version": "1.0", "version": 1,
        "levels": { "A1": { "writing_prompts": [{
            "id": "note", "prompt": {"en": "Write a note to a friend."},
            "reader": "a friend", "purpose": "invite", "content_points": ["when", "where"]
        }] } }
    }"#;

    #[test]
    fn a_typed_topic_is_cleaned_and_an_empty_one_is_refused() {
        let input = task_of(
            Some("  Write \n about my weekend.  "),
            None,
            None,
            Level::A1,
            None,
        )
        .expect("a topic");
        assert_eq!(input.task.prompt, "Write about my weekend.");
        assert!(input.task.content_points.is_empty());
        assert!(input.prompt_id.is_none());
        assert!(task_of(Some("   "), None, None, Level::A1, None).is_err());
        assert!(task_of(None, None, None, Level::A1, None).is_err());
    }

    #[test]
    fn a_prompt_id_comes_from_the_bank_and_needs_one_installed() {
        let bank = TopicBank::parse(BANK).expect("the bank parses");
        let input = task_of(None, Some("note"), Some(&bank), Level::A1, None).expect("a prompt");
        assert_eq!(input.task.prompt, "Write a note to a friend.");
        assert_eq!(input.task.content_points, ["when", "where"]);
        assert_eq!(input.prompt_id.as_deref(), Some("note"));
        assert_eq!(
            input.task.min_words, None,
            "the bank gives no minimum, the same as the app's writing task"
        );

        let missing = task_of(None, Some("note"), None, Level::A1, None)
            .err()
            .expect("no bank")
            .to_string();
        assert!(missing.contains("topic bank"), "{missing}");
        let unknown = task_of(None, Some("nope"), Some(&bank), Level::A1, None)
            .err()
            .expect("no such prompt")
            .to_string();
        assert!(unknown.contains("nope"), "{unknown}");
    }

    #[test]
    fn a_script_without_drafts_or_with_an_unknown_field_is_refused() {
        let dir = tempfile::TempDir::new().expect("a temp folder");
        let empty = dir.path().join("empty.json");
        std::fs::write(&empty, r#"{"drafts": []}"#).expect("write");
        assert!(load_drafts(&empty).is_err());

        let unknown = dir.path().join("unknown.json");
        std::fs::write(&unknown, r#"{"draft": ["one"]}"#).expect("write");
        assert!(load_drafts(&unknown).is_err());

        let ok = dir.path().join("ok.json");
        std::fs::write(&ok, r#"{"drafts": ["one", "two"]}"#).expect("write");
        assert_eq!(load_drafts(&ok).expect("two drafts").len(), 2);
    }

    /// A typed draft runs to a blank line, so a draft can be more than one line;
    /// `/quit` on the first line ends the round. This is the only place a draft
    /// is not one line of a script file.
    #[tokio::test]
    async fn a_typed_draft_ends_at_a_blank_line_and_quit_stops_the_round() {
        let input = "First line.\nSecond line.\n\nAnother draft.\n\n/quit\n";
        let mut drafts = Drafts::typed(BufReader::new(input.as_bytes()));
        let mut out = Vec::new();
        let first = drafts
            .next(1, &mut out)
            .await
            .expect("read")
            .expect("a draft");
        assert_eq!(first, "First line.\nSecond line.");
        let second = drafts
            .next(2, &mut out)
            .await
            .expect("read")
            .expect("a draft");
        assert_eq!(second, "Another draft.");
        assert!(drafts.next(3, &mut out).await.expect("read").is_none());
        let printed = String::from_utf8(out).expect("utf8");
        assert!(printed.contains("draft 1"), "{printed}");
    }

    #[tokio::test]
    async fn an_empty_typed_line_at_the_start_ends_the_round_without_a_draft() {
        let mut drafts = Drafts::typed(BufReader::new("\n".as_bytes()));
        let mut out = Vec::new();
        assert!(drafts.next(1, &mut out).await.expect("read").is_none());
    }
}
