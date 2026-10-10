//! `tutor-cli unit`: play a unit from its first activity to its checkpoint, and
//! score the responses that waited for a provider.
//!
//! This is a thin terminal around `tutor_engine::UnitPlayer`: it loads the unit,
//! the rubrics and the provider, takes the responses from a script or from the
//! keyboard, prints what the library returns and writes a result file. The
//! scoring, the evidence rows and the checkpoint are all the library's.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use curriculum::LoadedUnit;
use curriculum::validate::WordLevels;
use serde_json::{Value, json};
use storage::Database;
use tokio::io::{AsyncBufRead, BufReader};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    ActivityError, ActivityResult, Body, EngineError, NoProvider, ReplyOutcome, ResultOutcome,
    RubricCatalog, RubricScorer, ScorerEnv, UnitConfig, UnitEnv, UnitPlayer, UnitSummary,
    ensure_indexed,
};

use crate::args::{ScorePendingArgs, UnitArgs, UnitCommand, UnitRunArgs};
use crate::chat::Failure;
use crate::exit::Exit;
use crate::provider;
use crate::unit_source::Source;
use crate::unit_view::{render_checkpoint, render_presentation, render_result};

pub async fn run(args: UnitArgs) -> Exit {
    let result = match args.command {
        UnitCommand::Run(run) => execute_run(&run).await,
        UnitCommand::ScorePending(score) => execute_score_pending(&score).await,
        UnitCommand::Practice(practice) => crate::practice::execute(&practice).await,
    };
    match result {
        Ok(exit) => exit,
        Err(failure) => {
            eprintln!("error: {:#}", failure.error);
            failure.exit
        }
    }
}

fn other(error: impl Into<anyhow::Error>) -> Failure {
    Failure {
        exit: Exit::Failure,
        error: error.into(),
    }
}

/// What a played unit leaves for the caller.
#[derive(Debug)]
pub struct RunReport {
    pub results: Vec<ActivityResult>,
    /// Activities that were skipped: no scripted response, or the input ended.
    pub skipped: Vec<String>,
    /// `None` when the checkpoint has activities that were not answered.
    pub summary: Option<UnitSummary>,
    pub session_id: i64,
}

fn status_name(result: &ActivityResult) -> &'static str {
    match (&result.outcome, result.score) {
        (ResultOutcome::Queued, _) => "pending",
        (ResultOutcome::Unscored(_), _) => "unscored",
        (_, Some(_)) => "scored",
        (_, None) => "needs_review",
    }
}

/// The result file: one JSON object, with task scores and no level anywhere.
pub fn result_json(unit_id: &str, report: &RunReport, database: Option<&Path>) -> Value {
    let activities: Vec<Value> = report
        .results
        .iter()
        .map(|r| {
            json!({
                "id": r.activity_id,
                "type": r.activity_type.as_str(),
                "skill": r.skill,
                "status": status_name(r),
                "score": r.score,
                "confidence": r.confidence,
            })
        })
        .chain(
            report
                .skipped
                .iter()
                .map(|id| json!({ "id": id, "status": "skipped", "score": null })),
        )
        .collect();
    let checkpoint = report.summary.as_ref().map(|s| {
        json!({
            "mean": s.checkpoint.outcome.mean,
            "pass_mark": s.checkpoint.pass_mark,
            "passed": s.checkpoint.outcome.passed,
            "provisional": s.checkpoint.outcome.provisional,
            "unit_status": format!("{:?}", s.status),
        })
    });
    json!({
        "unit": unit_id,
        "session_id": report.session_id,
        "database": database.map(|p| p.display().to_string()),
        "activities": activities,
        "checkpoint": checkpoint,
    })
}

/// Plays the unit with `source` and prints to `out`.
///
/// An activity with no response is skipped and listed; the checkpoint can only be
/// decided when every one of its activities was answered.
pub async fn play_unit<R: AsyncBufRead + Unpin, W: Write>(
    player: &mut UnitPlayer,
    source: &mut Source<R>,
    out: &mut W,
    show_audio_text: bool,
    cancel: &CancellationToken,
) -> Result<RunReport> {
    let mut results = Vec::new();
    let mut skipped = Vec::new();
    for id in player.activity_ids().to_vec() {
        let shown = player.present(&id)?;
        write!(out, "{}", render_presentation(&shown, show_audio_text))?;
        if let Body::Roleplay { .. } = shown.body {
            let result = play_roleplay(player, source, &id, out, cancel).await?;
            write!(out, "{}", render_result(&result))?;
            results.push(result);
            continue;
        }
        let Some(answer) = source.answer(&shown, out).await? else {
            writeln!(out, "(skipped: no response)")?;
            skipped.push(id);
            continue;
        };
        for play in 1..=answer.plays {
            match player.play_audio(&id) {
                Ok(played) if answer.plays > 1 => {
                    writeln!(
                        out,
                        "[audio played: {} time(s), replays left: {}]",
                        played.plays,
                        played
                            .replays_left
                            .map_or("no limit".to_owned(), |n| n.to_string())
                    )?;
                }
                Ok(_) => {}
                Err(EngineError::Activity(ActivityError::NoAudio(_))) => break,
                Err(EngineError::Activity(ActivityError::NoReplaysLeft { allowed })) => {
                    writeln!(
                        out,
                        "[play {play} refused: only {allowed} replay(s) are allowed]"
                    )?;
                    break;
                }
                Err(other) => return Err(other.into()),
            }
        }
        match player.submit(&id, answer.response, cancel).await {
            Ok(result) => {
                write!(out, "{}", render_result(&result))?;
                results.push(result);
            }
            Err(EngineError::Activity(error)) => {
                // A response the activity cannot take is the script's mistake: say
                // which, store nothing, and go on to the next activity.
                writeln!(out, "(refused: {error})")?;
                skipped.push(id);
            }
            Err(other) => return Err(other.into()),
        }
    }

    let session_id = player.session_id();
    let summary = match player.finish().await {
        Ok(summary) => {
            write!(
                out,
                "{}",
                render_checkpoint(&summary.checkpoint, &format!("{:?}", summary.status))
            )?;
            Some(summary)
        }
        Err(EngineError::Refused(reason)) => {
            let report = player.checkpoint().await?;
            writeln!(
                out,
                "\ncheckpoint not decided: {reason}: {}",
                report.unanswered.join(", ")
            )?;
            player.abort().await?;
            None
        }
        Err(other) => return Err(other.into()),
    };
    Ok(RunReport {
        results,
        skipped,
        summary,
        session_id,
    })
}

async fn play_roleplay<R: AsyncBufRead + Unpin, W: Write>(
    player: &mut UnitPlayer,
    source: &mut Source<R>,
    id: &str,
    out: &mut W,
    cancel: &CancellationToken,
) -> Result<ActivityResult> {
    let mut run = player.start_roleplay(id)?;
    let opening = run.open(|_| {}, cancel).await?;
    let mut reachable = opening.outcome != ReplyOutcome::ProviderUnavailable;
    if reachable {
        writeln!(out, "tutor> {}", opening.text)?;
    } else {
        writeln!(
            out,
            "(the provider is not available: the roleplay cannot run)"
        )?;
    }
    while reachable && run.turns_left() > 0 {
        let Some(line) = source.roleplay_turn(id, out).await? else {
            break;
        };
        writeln!(out, "you> {line}")?;
        let reply = run.say(&line, |_| {}, cancel).await?;
        match reply.outcome {
            ReplyOutcome::ProviderUnavailable => {
                writeln!(out, "(the provider stopped answering)")?;
                reachable = false;
            }
            _ => writeln!(out, "tutor> {}", reply.text)?,
        }
    }
    Ok(player.finish_roleplay(run, cancel).await?)
}

pub(crate) async fn open_database(path: &Path) -> Result<Database, Failure> {
    Database::open(path)
        .await
        .map_err(|error| other(anyhow!("the database could not be opened: {error}")))
}

pub(crate) async fn ensure_profile(db: &Database) -> Result<i64, Failure> {
    let existing = db
        .profiles()
        .first()
        .await
        .map_err(|e| other(anyhow!("{e}")))?;
    if let Some(profile) = existing {
        return Ok(profile.id);
    }
    let created = db
        .profiles()
        .create(&storage::NewProfile {
            display_name: "Learner".to_owned(),
            ui_language: storage::UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: storage::L1HelpMode::Auto,
            created_at: storage::Timestamp::now(),
        })
        .await
        .map_err(|e| other(anyhow!("{e}")))?;
    Ok(created.id)
}

/// The rubrics folder: the one given, or the `catalogs/rubrics` folder that sits
/// next to the unit's folder (`curriculum/examples/x.json` finds
/// `curriculum/catalogs/rubrics`).
fn rubrics_dir(args: &UnitRunArgs) -> PathBuf {
    args.rubrics.clone().unwrap_or_else(|| {
        args.unit
            .parent()
            .and_then(Path::parent)
            .map_or_else(PathBuf::new, |root| root.join("catalogs").join("rubrics"))
    })
}

fn default_database(unit_id: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    std::env::temp_dir().join(format!(
        "lumingo-unit-{unit_id}-{}-{stamp}.sqlite",
        std::process::id()
    ))
}

pub struct Connection {
    pub client: Arc<dyn llm_client::LlmClient>,
    pub model: String,
    pub label: String,
}

/// How the caller asked for a provider, shared by the commands that connect.
pub(crate) struct ProviderChoice<'a> {
    pub offline: bool,
    pub provider: Option<&'a str>,
    pub providers_file: Option<&'a std::path::Path>,
    pub data_dir: Option<&'a std::path::Path>,
    pub timeout_ms: u64,
}

/// The provider of the run, or none: a learner with no provider can still play
/// the unit, and its productive responses wait. `none_label` says what happens
/// without one, which differs per command.
pub(crate) async fn connect(choice: &ProviderChoice<'_>, none_label: &str) -> Connection {
    if !choice.offline {
        match provider::connect(
            &llm_client::EnvProfileLoader::with_default_paths(),
            choice.data_dir,
            choice.providers_file,
            choice.provider,
            Duration::from_millis(choice.timeout_ms),
        ) {
            Ok(connected) => {
                // Productive responses are scored with structured calls: the
                // probe must run first, or they start at ladder level 1 and
                // fail on a gateway that ignores the native schema (S4-06).
                // A probe failure that is not about the key still lets the run
                // continue; the responses then wait as pending.
                if connected.probe().await.continues() {
                    let client = connected.client();
                    return Connection {
                        label: format!("{} ({})", connected.host, connected.model),
                        model: connected.model,
                        client,
                    };
                }
                eprintln!("note: the provider did not accept the key");
            }
            Err(error) => eprintln!("note: no provider: {error:#}"),
        }
    }
    Connection {
        client: Arc::new(NoProvider),
        model: "none".to_owned(),
        label: none_label.to_owned(),
    }
}

async fn execute_run(args: &UnitRunArgs) -> Result<Exit, Failure> {
    let loaded: LoadedUnit = curriculum::load_unit_file(&args.unit)
        .map_err(|error| other(anyhow!("{}: {error}", args.unit.display())))?;
    let rubric_dir = rubrics_dir(args);
    let rubrics = match RubricCatalog::load_dir(&rubric_dir) {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!(
                "note: no rubrics were loaded from {}: {error}; productive responses will be stored unscored",
                rubric_dir.display()
            );
            RubricCatalog::default()
        }
    };
    let word_levels = match &args.word_list {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read the word list {}", path.display()))
                .map_err(other)?;
            Some(Arc::new(
                WordLevels::parse(&text).map_err(|e| other(anyhow!("{}: {e}", path.display())))?,
            ))
        }
        None => None,
    };
    let connection = connect(
        &ProviderChoice {
            offline: args.offline,
            provider: args.provider.as_deref(),
            providers_file: args.providers_file.as_deref(),
            data_dir: args.data_dir.as_deref(),
            timeout_ms: args.provider_timeout_ms,
        },
        "none: productive responses will be stored as pending",
    )
    .await;
    let database_path = args
        .db
        .clone()
        .unwrap_or_else(|| default_database(&loaded.unit.id));
    let db = open_database(&database_path).await?;
    let profile_id = ensure_profile(&db).await?;
    let now = tutor_engine::system_clock();
    ensure_indexed(&db, &loaded.unit, &loaded.checksum, now())
        .await
        .map_err(|e| other(anyhow!("{e}")))?;

    let env = UnitEnv {
        client: connection.client,
        db: db.clone(),
        clock: now,
        model: connection.model,
        provider_profile_id: None,
        provider_qualified: false,
        grammar: None,
        word_levels,
        rubrics: Arc::new(rubrics),
        drill: None,
        tts: None,
    };
    let config = UnitConfig {
        profile_id,
        first_language: args.first_language.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let unit_id = loaded.unit.id.clone();
    let title = loaded.unit.title.en.clone();
    let mut player = UnitPlayer::start(env, config, loaded.unit)
        .await
        .map_err(|e| other(anyhow!("{e}")))?;

    println!("unit {unit_id}: {title}");
    println!("provider: {}", connection.label);
    println!("database: {}", database_path.display());

    let cancel = CancellationToken::new();
    let watcher = cancel.clone();
    let interrupt = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            watcher.cancel();
        }
    });
    let mut stdout = std::io::stdout();
    let report = if let Some(script) = &args.script {
        let file = crate::unit_script::load(script).map_err(other)?;
        let base = script.parent().unwrap_or_else(|| Path::new("."));
        let mut source = Source::<BufReader<tokio::io::Empty>>::script(file, base);
        play_unit(
            &mut player,
            &mut source,
            &mut stdout,
            !args.hide_audio_text,
            &cancel,
        )
        .await
    } else {
        let mut source = Source::typed(BufReader::new(tokio::io::stdin()));
        play_unit(
            &mut player,
            &mut source,
            &mut stdout,
            !args.hide_audio_text,
            &cancel,
        )
        .await
    };
    interrupt.abort();
    let report = match report {
        Ok(report) => report,
        Err(error) if cancel.is_cancelled() => {
            eprintln!("interrupted: {error:#}");
            let _ = player.abort().await;
            return Ok(Exit::Interrupted);
        }
        Err(error) => {
            let _ = player.abort().await;
            return Err(other(error));
        }
    };
    if let Some(path) = &args.out {
        let body =
            serde_json::to_string_pretty(&result_json(&unit_id, &report, Some(&database_path)))
                .map_err(|e| other(anyhow!("{e}")))?;
        std::fs::write(path, body)
            .with_context(|| format!("cannot write {}", path.display()))
            .map_err(other)?;
        println!("result file: {}", path.display());
    }
    let pending = report
        .results
        .iter()
        .filter(|r| matches!(r.outcome, ResultOutcome::Queued))
        .count();
    if pending > 0 {
        println!(
            "\n{pending} response(s) wait for a provider: run `tutor-cli unit score-pending --db {}` once one is reachable",
            database_path.display()
        );
    }
    Ok(if report.summary.is_some() {
        Exit::Done
    } else {
        Exit::Failure
    })
}

async fn execute_score_pending(args: &ScorePendingArgs) -> Result<Exit, Failure> {
    if !args.db.exists() {
        return Err(other(anyhow!(
            "the database {} does not exist",
            args.db.display()
        )));
    }
    let db = open_database(&args.db).await?;
    let connected = provider::connect(
        &llm_client::EnvProfileLoader::with_default_paths(),
        args.data_dir.as_deref(),
        args.providers_file.as_deref(),
        args.provider.as_deref(),
        Duration::from_millis(args.provider_timeout_ms),
    )
    .map_err(|error| Failure {
        exit: Exit::ProviderUnavailable,
        error,
    })?;
    println!("provider: {} ({})", connected.host, connected.model);
    // Scoring the backlog is all structured calls: the probe must run first
    // (S4-06), or they start at ladder level 1 and fail on a gateway that
    // ignores the native schema.
    if !connected.probe().await.continues() {
        return Err(Failure {
            exit: Exit::ProviderUnavailable,
            error: anyhow!("the provider did not accept the key"),
        });
    }
    let scorer = RubricScorer::new(ScorerEnv {
        client: connected.client(),
        db,
        clock: tutor_engine::system_clock(),
        model: connected.model,
        provider_profile_id: None,
        grammar: None,
        word_levels: None,
        provider_qualified: false,
    });
    let cancel = CancellationToken::new();
    let report = scorer
        .score_pending_backlog(&cancel)
        .await
        .map_err(|e| other(anyhow!("{e}")))?;
    println!(
        "scored {}, given up {}, skipped {}, still waiting {}",
        report.scored, report.given_up, report.skipped, report.remaining
    );
    if report.provider_unreachable {
        eprintln!("the provider could not be reached: the rest stays queued");
        return Ok(Exit::ProviderUnavailable);
    }
    Ok(Exit::Done)
}
