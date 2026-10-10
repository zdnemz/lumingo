//! `tutor-cli unit practice`: extra practice items for one unit (S4-10).
//!
//! The generation, the item checks, the regeneration and the fallback are
//! `tutor_engine::PracticeGenerator`; the scoring is
//! `tutor_engine::record_practice_attempt`, the same runtime a unit answer goes
//! through. This module is the terminal around them: it loads the unit, connects
//! the provider, takes the responses from a script or the keyboard, prints the
//! scores and writes a result file.
//!
//! Every answer is practice: the attempt rows carry the item's origin
//! (`generated`, or `authored` for a replayed item) and
//! `counts_toward_estimate = false`, so nothing here can move a level estimate.
//! The items themselves are stored with this run's session and are deleted with
//! it.
//!
//! A script answers a generated item by its position, `gen-1`, `gen-2`, ...:
//! a generated id is scoped to the session, which a script written in advance
//! cannot know. A replayed authored item keeps its authored id.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use curriculum::LoadedUnit;
use curriculum::validate::WordLevels;
use serde_json::{Value, json};
use storage::{AttemptOrigin, Database, SessionKind, SessionStatus};
use tokio::io::{AsyncBufRead, BufReader};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    ActivityResult, EvidenceRecorder, FallbackReason, PracticeAnswer, PracticeConfig,
    PracticeGenerator, PracticeSet, PracticeSource, Response, ResultOutcome, evidence_skill,
    present, score_deterministic,
};

use crate::args::PracticeArgs;
use crate::chat::Failure;
use crate::exit::Exit;
use crate::unit::{Connection, ProviderChoice, connect, ensure_profile, open_database};
use crate::unit_source::Source;
use crate::unit_view::{render_presentation, render_result};

fn other(error: impl Into<anyhow::Error>) -> Failure {
    Failure {
        exit: Exit::Failure,
        error: error.into(),
    }
}

pub async fn execute(args: &PracticeArgs) -> Result<Exit, Failure> {
    if args.count == 0 {
        return Err(other(anyhow!("--count must be at least 1")));
    }
    let loaded: LoadedUnit = curriculum::load_unit_file(&args.unit)
        .map_err(|error| other(anyhow!("{}: {error}", args.unit.display())))?;
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
        "none: the authored fallback will be replayed",
    )
    .await;
    run(args, &loaded, word_levels, connection).await
}

/// The run itself, with the unit loaded and the provider connected. The tests
/// call this directly with a fake client; the binary goes through [`execute`].
pub async fn run(
    args: &PracticeArgs,
    loaded: &LoadedUnit,
    word_levels: Option<Arc<WordLevels>>,
    connection: Connection,
) -> Result<Exit, Failure> {
    let database_path = args
        .db
        .clone()
        .unwrap_or_else(|| default_database(&loaded.unit.id));
    let db = open_database(&database_path).await?;
    let profile_id = ensure_profile(&db).await?;
    let now = tutor_engine::system_clock();
    tutor_engine::ensure_indexed(&db, &loaded.unit, &loaded.checksum, now())
        .await
        .map_err(|e| other(anyhow!("{e}")))?;

    let unit_id = loaded.unit.id.clone();
    let title = loaded.unit.title.en.clone();
    // The session the items are stored with and deleted with. A drill session
    // is practice: it never settles the unit's progress.
    let session = db
        .sessions()
        .create(&storage::NewSession {
            profile_id,
            kind: SessionKind::Drill,
            unit_id: Some(unit_id.clone()),
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            started_at: now(),
        })
        .await
        .map_err(|e| other(anyhow!("the session could not be stored: {e}")))?;

    let wrong = wrong_activity_ids(&db, profile_id).await?;
    let generator = PracticeGenerator::new(
        PracticeConfig {
            profile_id,
            session_id: session.id,
            provider_profile_id: None,
            model: connection.model.clone(),
            first_language: args.first_language.clone(),
            word_levels,
        },
        connection.client.clone(),
        db.clone(),
        now.clone(),
    );

    println!("practice for unit {unit_id}: {title}");
    println!("provider: {}", connection.label);
    println!("database: {}", database_path.display());

    let cancel = CancellationToken::new();
    let watcher = cancel.clone();
    let interrupt = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            watcher.cancel();
        }
    });
    let set = generator
        .generate(&loaded.unit, usize::from(args.count), &wrong, &cancel)
        .await;
    let set = match set {
        Ok(set) => set,
        Err(error) => {
            interrupt.abort();
            let _ = finish(&db, session.id, SessionStatus::Aborted, &now).await;
            if cancel.is_cancelled() {
                eprintln!("interrupted: {error}");
                return Ok(Exit::Interrupted);
            }
            return Err(other(anyhow!("{error}")));
        }
    };
    print_set(&set);

    let mut stdout = std::io::stdout();
    let recorder = EvidenceRecorder::new(db.clone(), now.clone());
    let played = if let Some(script) = &args.script {
        let file = crate::unit_script::load(script).map_err(other)?;
        let base = script.parent().unwrap_or_else(|| Path::new("."));
        let mut source = Source::<BufReader<tokio::io::Empty>>::script(file, base);
        play(
            &loaded.unit,
            &set,
            &recorder,
            profile_id,
            session.id,
            &mut source,
            &mut stdout,
            &cancel,
        )
        .await
    } else {
        let mut source = Source::typed(BufReader::new(tokio::io::stdin()));
        play(
            &loaded.unit,
            &set,
            &recorder,
            profile_id,
            session.id,
            &mut source,
            &mut stdout,
            &cancel,
        )
        .await
    };
    interrupt.abort();
    let played = match played {
        Ok(played) => played,
        Err(error) => {
            let _ = finish(&db, session.id, SessionStatus::Aborted, &now).await;
            if cancel.is_cancelled() {
                eprintln!("interrupted: {error:#}");
                return Ok(Exit::Interrupted);
            }
            return Err(other(error));
        }
    };
    finish(&db, session.id, SessionStatus::Completed, &now)
        .await
        .map_err(|e| other(anyhow!("the session could not be closed: {e}")))?;

    let report = PracticeReport {
        session_id: session.id,
        set: &set,
        played,
    };
    println!(
        "\nscored {} of {} item(s); {} skipped",
        report.played.items.len(),
        report.set.items.len(),
        report.played.skipped.len()
    );
    println!("every answer here is practice: none of it counts toward a level estimate");
    if let Some(path) = &args.out {
        let body =
            serde_json::to_string_pretty(&result_json(&unit_id, &report, Some(&database_path)))
                .map_err(|e| other(anyhow!("{e}")))?;
        std::fs::write(path, body)
            .with_context(|| format!("cannot write {}", path.display()))
            .map_err(other)?;
        println!("result file: {}", path.display());
    }
    Ok(Exit::Done)
}

fn default_database(unit_id: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    std::env::temp_dir().join(format!(
        "lumingo-practice-{unit_id}-{}-{stamp}.sqlite",
        std::process::id()
    ))
}

async fn finish(
    db: &Database,
    session_id: i64,
    status: SessionStatus,
    now: &tutor_engine::Clock,
) -> Result<(), storage::StorageError> {
    db.sessions()
        .finish(session_id, status, &now(), Some(&json!({})))
        .await
}

/// The activity ids the learner got wrong in the newest lesson or checkpoint
/// run of this profile, first wrong first. Empty when there is none, or when
/// that run was perfect.
async fn wrong_activity_ids(db: &Database, profile_id: i64) -> Result<Vec<String>, Failure> {
    let sessions = db
        .sessions()
        .list_for_profile(profile_id, 50)
        .await
        .map_err(|e| other(anyhow!("{e}")))?;
    let Some(newest) = sessions
        .iter()
        .find(|s| matches!(s.kind, SessionKind::Lesson | SessionKind::Checkpoint))
    else {
        return Ok(Vec::new());
    };
    let attempts = db
        .attempts()
        .for_session(newest.id)
        .await
        .map_err(|e| other(anyhow!("{e}")))?;
    let mut ids: Vec<String> = Vec::new();
    for attempt in attempts
        .iter()
        .filter(|a| a.normalized.is_some_and(|n| n < 1.0))
    {
        if !ids.contains(&attempt.activity_id) {
            ids.push(attempt.activity_id.clone());
        }
    }
    Ok(ids)
}

fn fallback_text(reason: FallbackReason) -> &'static str {
    match reason {
        FallbackReason::PolicyForbids => "the unit's generation policy allows no generated items",
        FallbackReason::SessionLimit => {
            "this session already holds as many generated items as the policy allows"
        }
        FallbackReason::ProviderUnavailable => "the provider could not be used",
        FallbackReason::InvalidOutput => "the model's reply was not usable",
        FallbackReason::TooFewValid => "too few of the model's items passed the checks",
    }
}

fn print_set(set: &PracticeSet) {
    match set.source {
        PracticeSource::Generated => {
            println!(
                "set: generated, {} item(s){}{}",
                set.items.len(),
                if set.rejected.is_empty() {
                    String::new()
                } else {
                    format!(", {} dropped", set.rejected.len())
                },
                if set.regenerated {
                    ", after one regeneration"
                } else {
                    ""
                }
            );
        }
        PracticeSource::Authored => {
            println!(
                "set: authored fallback ({}), {} item(s)",
                set.fallback.map_or("no reason recorded", fallback_text),
                set.items.len(),
            );
        }
    }
    if !set.vocabulary_checked {
        println!("vocabulary check: skipped (no --word-list)");
    }
}

/// One practice item that was answered.
#[derive(Debug)]
pub struct PlayedItem {
    /// The id as the script names it: `gen-1` for a generated item, the
    /// authored id for a replayed one.
    pub id: String,
    /// The activity's own id, which is what the attempt rows carry.
    pub activity_id: String,
    pub activity_type: String,
    pub origin: AttemptOrigin,
    pub score: f64,
}

/// What a practice run leaves for the caller.
#[derive(Debug)]
pub struct Played {
    pub items: Vec<PlayedItem>,
    pub skipped: Vec<String>,
}

/// Everything the result file needs about one practice run.
pub struct PracticeReport<'a> {
    pub session_id: i64,
    pub set: &'a PracticeSet,
    pub played: Played,
}

/// The result file: the items, their scores and why the set is what it is.
/// Nothing here names a level.
pub fn result_json(unit_id: &str, report: &PracticeReport<'_>, database: Option<&Path>) -> Value {
    let items: Vec<Value> = report
        .played
        .items
        .iter()
        .map(|item| {
            json!({
                "id": item.id,
                "activity_id": item.activity_id,
                "type": item.activity_type,
                "origin": item.origin.as_str(),
                "status": "scored",
                "score": item.score,
            })
        })
        .chain(
            report
                .played
                .skipped
                .iter()
                .map(|id| json!({ "id": id, "status": "skipped", "score": null })),
        )
        .collect();
    json!({
        "unit": unit_id,
        "session_id": report.session_id,
        "database": database.map(|p| p.display().to_string()),
        "source": match report.set.source {
            PracticeSource::Generated => "generated",
            PracticeSource::Authored => "authored",
        },
        "fallback": report.set.fallback.map(fallback_name),
        "regenerated": report.set.regenerated,
        "vocabulary_checked": report.set.vocabulary_checked,
        "dropped": report.set.rejected.len(),
        "items": items,
    })
}

fn fallback_name(reason: FallbackReason) -> &'static str {
    match reason {
        FallbackReason::PolicyForbids => "policy_forbids",
        FallbackReason::SessionLimit => "session_limit",
        FallbackReason::ProviderUnavailable => "provider_unavailable",
        FallbackReason::InvalidOutput => "invalid_output",
        FallbackReason::TooFewValid => "too_few_valid",
    }
}

fn practice_answer(response: &Response) -> Result<PracticeAnswer> {
    Ok(match response {
        Response::Choice(index) => PracticeAnswer::Choice(*index),
        Response::Gaps(given) => PracticeAnswer::Gaps(given.clone()),
        Response::Order(given) => PracticeAnswer::Order(given.clone()),
        _ => {
            return Err(anyhow!(
                "this practice item cannot take that kind of answer"
            ));
        }
    })
}

/// Plays the set with `source` and prints to `out`. An item with no response is
/// skipped and listed; a response the item cannot take is refused by name. The
/// run goes on either way. The scores come from the same runtime a unit answer
/// uses, so a practice answer and a unit answer are marked alike.
#[allow(clippy::too_many_arguments)]
async fn play<R: AsyncBufRead + Unpin, W: Write>(
    unit: &curriculum::Unit,
    set: &PracticeSet,
    recorder: &EvidenceRecorder,
    profile_id: i64,
    session_id: i64,
    source: &mut Source<R>,
    out: &mut W,
    cancel: &CancellationToken,
) -> Result<Played> {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let mut generated = 0_usize;
    for item in &set.items {
        if cancel.is_cancelled() {
            return Err(anyhow!("interrupted"));
        }
        // A generated id is session-scoped, so a script cannot name it. The
        // position stands in: gen-1, gen-2, ... A replayed authored item keeps
        // its authored id, which a script can name.
        let id = match item.origin {
            AttemptOrigin::Generated => {
                generated += 1;
                format!("gen-{generated}")
            }
            _ => item.activity.id().to_owned(),
        };
        let mut shown = present(unit, &item.activity).map_err(|e| anyhow!("{e}"))?;
        shown.id.clone_from(&id);
        write!(out, "{}", render_presentation(&shown, true))?;
        let Some(answer) = source.answer(&shown, out).await? else {
            writeln!(out, "(skipped: no response)")?;
            skipped.push(id);
            continue;
        };
        // A response the item cannot take is the script's mistake: say which,
        // store nothing, and go on to the next item. This mirrors `unit run`.
        let practice = match practice_answer(&answer.response) {
            Ok(practice) => practice,
            Err(error) => {
                writeln!(out, "(refused: {error})")?;
                skipped.push(id);
                continue;
            }
        };
        let scored = match score_deterministic(&item.activity, &answer.response) {
            Ok(scored) => scored,
            Err(error) => {
                writeln!(out, "(refused: {error})")?;
                skipped.push(id);
                continue;
            }
        };
        let (score, attempt) = tutor_engine::record_practice_attempt(
            recorder, profile_id, session_id, unit, item, &practice,
        )
        .await
        .map_err(|e| anyhow!("{e}"))?;
        let result = ActivityResult {
            activity_id: id.clone(),
            activity_type: item.activity.activity_type(),
            skill: evidence_skill(&item.activity, false).to_owned(),
            score: Some(scored.record.normalized),
            confidence: Some(1.0),
            outcome: ResultOutcome::Deterministic(Box::new(scored.feedback)),
            response_ids: vec![attempt.response_id.clone()],
        };
        write!(out, "{}", render_result(&result))?;
        items.push(PlayedItem {
            id,
            activity_id: item.activity.id().to_owned(),
            activity_type: item.activity.activity_type().as_str().to_owned(),
            origin: item.origin,
            score,
        });
    }
    Ok(Played { items, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fallback_reason_has_a_sentence_and_a_name() {
        for reason in [
            FallbackReason::PolicyForbids,
            FallbackReason::SessionLimit,
            FallbackReason::ProviderUnavailable,
            FallbackReason::InvalidOutput,
            FallbackReason::TooFewValid,
        ] {
            assert!(!fallback_text(reason).is_empty());
            assert!(!fallback_name(reason).is_empty());
        }
    }

    #[test]
    fn a_choice_a_gap_and_an_order_map_and_the_rest_is_refused() {
        assert_eq!(
            practice_answer(&Response::Choice(1)).unwrap(),
            PracticeAnswer::Choice(1)
        );
        assert_eq!(
            practice_answer(&Response::Gaps(vec!["am".into()])).unwrap(),
            PracticeAnswer::Gaps(vec!["am".into()])
        );
        assert_eq!(
            practice_answer(&Response::Order(vec!["I".into()])).unwrap(),
            PracticeAnswer::Order(vec!["I".into()])
        );
        assert!(practice_answer(&Response::Done).is_err());
        assert!(practice_answer(&Response::Text("x".into())).is_err());
    }
}
