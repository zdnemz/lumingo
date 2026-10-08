//! `tutor-cli chat`: wiring the voice loop to a terminal, a script or recordings.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, anyhow};
use app_core::voice::{
    LatencySummary, Recording, Scenario, SystemLoopClock, VoiceConfig, VoiceError, VoiceHandle,
    VoiceLoop, VoiceParts, VoiceSummary,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tutor_engine::{FeedbackMode, Phase, TurnState};

use crate::args::{ChatArgs, ModeArg};
use crate::engines::{self, Need, Settings};
use crate::exit::Exit;
use crate::observer::Observer;
use crate::provider::{self, Provider};
use crate::results::{self, EngineLine, RunInfo};
use crate::script::{self, ScriptTurn};
use crate::wav;

/// A failure with the exit code it should end the program with.
pub struct Failure {
    pub exit: Exit,
    pub error: anyhow::Error,
}

impl Failure {
    fn new(exit: Exit, error: impl Into<anyhow::Error>) -> Self {
        Self {
            exit,
            error: error.into(),
        }
    }

    fn other(error: impl Into<anyhow::Error>) -> Self {
        Self::new(Exit::Failure, error)
    }
}

/// How the driving of the session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    Completed,
    Interrupted,
    ProviderUnavailable,
    TimedOut,
}

/// The longest one turn may take from the learner's message to its end: the
/// recogniser's 10 s, the tutor turn's 30 s and speech with some room.
const TURN_LIMIT: Duration = Duration::from_secs(120);

/// Samples per paced chunk of a recording: three VAD frames, so a chunk never
/// ends inside a frame.
const FEED_CHUNK: usize = 1_536;

pub async fn run(args: ChatArgs) -> Exit {
    match execute(&args).await {
        Ok(exit) => exit,
        Err(failure) => {
            eprintln!("error: {:#}", failure.error);
            failure.exit
        }
    }
}

fn speech_exit(error: &VoiceError) -> Exit {
    match error {
        VoiceError::EngineUnavailable { .. }
        | VoiceError::EngineLoadTimeout { .. }
        | VoiceError::Audio(_)
        | VoiceError::Device(_)
        | VoiceError::Playback(_)
        | VoiceError::Worker(_) => Exit::SpeechUnavailable,
        _ => Exit::Failure,
    }
}

async fn load_scenario(args: &ChatArgs) -> Result<(Scenario, String), Failure> {
    let bytes = tokio::fs::read(&args.scenario)
        .await
        .with_context(|| format!("cannot read the scenario {}", args.scenario.display()))
        .map_err(Failure::other)?;
    let unit = curriculum::load_unit_bytes(&bytes)
        .map_err(|error| anyhow!("{}: {error}", args.scenario.display()))
        .map_err(Failure::other)?
        .unit;
    let mode = args.mode.map(|m| match m {
        ModeArg::Fluency => FeedbackMode::Fluency,
        ModeArg::Accuracy => FeedbackMode::Accuracy,
    });
    let title = unit.title.en.clone();
    let scenario = Scenario::from_unit(&unit, args.activity.as_deref(), mode, &args.first_language)
        .map_err(Failure::other)?;
    Ok((scenario, title))
}

async fn open_recording(args: &ChatArgs, provider: &Provider) -> Result<Recording, Failure> {
    let dir = args
        .data_dir
        .clone()
        .or_else(app_core::default_data_dir)
        .ok_or_else(|| {
            Failure::other(anyhow!(
                "--analysis needs --data-dir: no home folder is known"
            ))
        })?;
    let db = storage::Database::open(dir.join("lumingo.sqlite"))
        .await
        .map_err(|error| Failure::other(anyhow!("the database could not be opened: {error}")))?;
    let profile = match db
        .profiles()
        .first()
        .await
        .map_err(|e| Failure::other(anyhow!("{e}")))?
    {
        Some(profile) => profile,
        None => db
            .profiles()
            .create(&storage::NewProfile {
                display_name: "Learner".to_owned(),
                ui_language: storage::UiLanguage::Id,
                l1: "id".to_owned(),
                l1_help_mode: storage::L1HelpMode::Auto,
                created_at: storage::Timestamp::now(),
            })
            .await
            .map_err(|e| Failure::other(anyhow!("{e}")))?,
    };
    Ok(Recording {
        db,
        profile_id: profile.id,
        provider_profile_id: None,
        model: provider.model.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        // The example unit is not in this database's curriculum index.
        link_unit: false,
        clock: tutor_engine::system_clock(),
    })
}

async fn execute(args: &ChatArgs) -> Result<Exit, Failure> {
    let (scenario, unit_title) = load_scenario(args).await?;
    let planned: Option<Vec<ScriptTurn>> = match &args.script {
        Some(path) => {
            let all = script::load(path).map_err(Failure::other)?;
            Some(script::plan(&all, args.turns))
        }
        None => None,
    };
    let has_audio = planned
        .iter()
        .flatten()
        .any(|t| matches!(t, ScriptTurn::Audio(_)));
    if args.text && has_audio {
        return Err(Failure::other(anyhow!(
            "the script has wav: lines, which need the VAD and the recogniser: drop --text"
        )));
    }
    // Typed input means no microphone and no recogniser. A script of recordings
    // needs the recogniser but not the microphone.
    let typed = args.text || (planned.is_some() && !has_audio);
    let need = Need {
        microphone: !typed && planned.is_none(),
        listen: !typed,
        speak: if typed { args.speak } else { !args.no_speak },
    };

    let provider = provider::connect(
        &llm_client::EnvProfileLoader::with_default_paths(),
        args.data_dir.as_deref(),
        args.providers_file.as_deref(),
        args.provider.as_deref(),
        Duration::from_millis(args.provider_timeout_ms),
    )
    .map_err(Failure::other)?;

    let mut config = VoiceConfig {
        provider_timeout: Duration::from_millis(args.provider_timeout_ms),
        ..VoiceConfig::default()
    };
    config.endpoint = config
        .endpoint
        .with_end_silence_ms(args.end_silence_ms)
        .map_err(|error| Failure::other(anyhow!("{error}")))?;

    let fake_transcripts = planned.as_ref().map_or(1_000, Vec::len) + 1;
    let (backend, end_silence, engines_file, data_dir) = (
        args.backend,
        args.end_silence_ms,
        args.engines.clone(),
        args.data_dir.clone(),
    );
    let built = tokio::task::spawn_blocking(move || {
        engines::build(
            need,
            &Settings {
                backend,
                engines_file: engines_file.as_deref(),
                data_dir: data_dir.as_deref(),
                end_silence_ms: end_silence,
                fake_transcripts,
            },
        )
    })
    .await
    .map_err(|_| Failure::other(anyhow!("loading the speech side did not finish")))?
    .map_err(|problem| Failure::new(Exit::SpeechUnavailable, anyhow!("{problem}")))?;

    let recording = if args.analysis {
        Some(open_recording(args, &provider).await?)
    } else {
        None
    };
    let unit_id = scenario.unit_id.clone();
    let activity_id = scenario.activity_id.clone();
    let feedback = scenario.context.mode;
    let backend_name = built.backend;
    let warning = built.warning.clone();
    let voice = VoiceLoop::start(VoiceParts {
        llm: provider.client.clone(),
        clock: std::sync::Arc::new(SystemLoopClock::new()),
        scenario,
        config,
        audio: built.audio,
        listen: built.listen,
        tts: built.tts,
        recording,
    })
    .await
    .map_err(|error| Failure::new(speech_exit(&error), anyhow!("{error}")))?;
    let handle = voice.handle();
    let observer = Observer::start(handle.subscribe(), args.show_states);

    eprintln!(
        "provider {} ({}, model {}); scenario \"{unit_title}\"; input {}; speech output {}",
        provider.host,
        provider.protocol,
        provider.model,
        if typed {
            "typed"
        } else if planned.is_some() {
            "recorded utterances"
        } else {
            "microphone"
        },
        if need.speak { "on" } else { "off" },
    );
    if let Some(warning) = &warning {
        eprintln!("warning: {warning}");
    }

    let ended = match &planned {
        Some(turns) => drive_script(&handle, &observer, turns, args).await,
        None => drive_interactive(&handle, &observer, args, typed).await,
    };
    let ended = match ended {
        Ok(ended) => ended,
        Err(failure) => {
            let _ = voice.finish(true).await;
            return Err(failure);
        }
    };

    let engines = handle.engines();
    let summary = voice.finish(ended == Ended::Interrupted).await;
    // The observer prints the last events on its own task; give it a moment.
    tokio::time::sleep(Duration::from_millis(50)).await;

    if let Some(session) = &summary.recording {
        eprintln!(
            "stored session {} with {} turns ({} without an analysis)",
            session.session_id, session.turns_stored, session.unanalysed_turns
        );
    }
    print_summary(&summary.latency);

    if let Some(turns) = &planned {
        let run = RunInfo {
            tool: "tutor-cli",
            version: env!("CARGO_PKG_VERSION"),
            input: input_kind(turns),
            backend: backend_name.to_owned(),
            scenario_unit: unit_id,
            scenario_activity: activity_id,
            feedback_mode: match feedback {
                FeedbackMode::Fluency => "fluency",
                FeedbackMode::Accuracy => "accuracy",
            }
            .to_owned(),
            provider_host: provider.host.clone(),
            provider_model: provider.model.clone(),
            provider_protocol: provider.protocol.clone(),
            turns_planned: turns.len(),
            end_silence_ms: args.end_silence_ms,
            stt: engines.stt.map(EngineLine::from),
            tts: engines.tts.map(EngineLine::from),
            warning,
        };
        write_results(args.out.clone(), &run, &observer, ended, &summary).await?;
    }

    Ok(match ended {
        Ended::Completed => Exit::Done,
        Ended::Interrupted => Exit::Interrupted,
        Ended::ProviderUnavailable => {
            eprintln!(
                "The provider is not reachable. The session was stopped cleanly. \
                 Run again when the connection is back (exit code {}).",
                Exit::ProviderUnavailable.code()
            );
            Exit::ProviderUnavailable
        }
        Ended::TimedOut => {
            return Err(Failure::other(anyhow!(
                "a turn did not end within {} s; the session was stopped",
                TURN_LIMIT.as_secs()
            )));
        }
    })
}

fn input_kind(turns: &[ScriptTurn]) -> String {
    let audio = turns.iter().filter(|t| t.kind() == "audio").count();
    match audio {
        0 => "text",
        n if n == turns.len() => "audio",
        _ => "mixed",
    }
    .to_owned()
}

fn print_summary(summary: &LatencySummary) {
    if summary.turns == 0 {
        return;
    }
    let figure = |name: &str, p50: Option<f64>, p95: Option<f64>, count: usize| {
        if let (Some(p50), Some(p95)) = (p50, p95) {
            println!("summary  {name}: p50 {p50:.0} ms, p95 {p95:.0} ms over {count} turns");
        }
    };
    figure(
        "end to end",
        summary.sum.p50_ms,
        summary.sum.p95_ms,
        summary.sum.count,
    );
    figure(
        "llm first sentence",
        summary.llm_first_sentence.p50_ms,
        summary.llm_first_sentence.p95_ms,
        summary.llm_first_sentence.count,
    );
}

async fn write_results(
    path: PathBuf,
    run: &RunInfo,
    observer: &Observer,
    ended: Ended,
    summary: &VoiceSummary,
) -> Result<(), Failure> {
    let ended_name = match ended {
        Ended::Completed => "completed",
        Ended::Interrupted => "interrupted",
        Ended::ProviderUnavailable => "provider_unavailable",
        Ended::TimedOut => "timed_out",
    };
    let text = results::render(run, &observer.records(), ended_name, &summary.stats)
        .map_err(Failure::other)?;
    results::write(&path, text).await.map_err(Failure::other)?;
    eprintln!("result file written to {}", path.display());
    Ok(())
}

/// Waits until more turns have ended than `before`.
async fn wait_ended(observer: &Observer, before: usize) -> Ended {
    let started = tokio::time::Instant::now();
    loop {
        if observer.ended() > before {
            return if observer.provider_unavailable().is_some() {
                Ended::ProviderUnavailable
            } else {
                Ended::Completed
            };
        }
        if started.elapsed() > TURN_LIMIT {
            return Ended::TimedOut;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn drive_script(
    handle: &VoiceHandle,
    observer: &Observer,
    turns: &[ScriptTurn],
    args: &ChatArgs,
) -> Result<Ended, Failure> {
    let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
    // Recordings are read up front, so a bad file stops the run before it starts.
    let mut clips: Vec<Option<Vec<f32>>> = Vec::with_capacity(turns.len());
    for turn in turns {
        clips.push(match turn {
            ScriptTurn::Text(_) => None,
            ScriptTurn::Audio(path) => {
                let bytes = tokio::fs::read(path)
                    .await
                    .with_context(|| format!("cannot read {}", path.display()))
                    .map_err(Failure::other)?;
                Some(
                    tokio::task::spawn_blocking(move || wav::parse(&bytes))
                        .await
                        .map_err(|_| Failure::other(anyhow!("reading a recording did not finish")))?
                        .with_context(|| format!("{}", path.display()))
                        .map_err(Failure::other)?,
                )
            }
        });
    }

    if !args.learner_first {
        handle
            .open()
            .await
            .map_err(|e| Failure::other(anyhow!("{e}")))?;
        let ended = tokio::select! {
            ended = wait_ended(observer, 0) => ended,
            _ = &mut ctrl_c => Ended::Interrupted,
        };
        if ended != Ended::Completed {
            return Ok(ended);
        }
    }
    for (turn, clip) in turns.iter().zip(clips) {
        let before = observer.ended();
        observer.set_next_input(turn.kind());
        let step = async {
            match (turn, clip) {
                (ScriptTurn::Text(text), _) => handle
                    .send_text(text.clone())
                    .await
                    .map_err(|e| Failure::other(anyhow!("{e}")))?,
                (ScriptTurn::Audio(_), Some(samples)) => {
                    feed_paced(handle, &samples, args.end_silence_ms)
                        .await
                        .map_err(|e| Failure::other(anyhow!("{e}")))?;
                }
                (ScriptTurn::Audio(_), None) => {}
            }
            Ok::<_, Failure>(wait_ended(observer, before).await)
        };
        let ended = tokio::select! {
            ended = step => ended?,
            _ = &mut ctrl_c => Ended::Interrupted,
        };
        if ended != Ended::Completed {
            return Ok(ended);
        }
    }
    Ok(Ended::Completed)
}

/// Feeds a recording at the speed it was spoken, then enough silence for the
/// endpointer to end the utterance. The pace matters: the endpointing wait is
/// measured against the clock, so silence fed faster than real time would make
/// it look shorter than it is.
async fn feed_paced(
    handle: &VoiceHandle,
    samples: &[f32],
    end_silence_ms: u32,
) -> Result<(), VoiceError> {
    let pace = Duration::from_micros(FEED_CHUNK as u64 * 1_000_000 / 16_000);
    for chunk in samples.chunks(FEED_CHUNK) {
        handle.feed_audio(chunk).await?;
        tokio::time::sleep(pace).await;
    }
    let silence = vec![0.0_f32; FEED_CHUNK];
    let chunks = (u64::from(end_silence_ms) + 800) * 16 / FEED_CHUNK as u64 + 1;
    for _ in 0..chunks {
        handle.feed_audio(&silence).await?;
        tokio::time::sleep(pace).await;
    }
    Ok(())
}

enum Line {
    Quit,
    Stop,
    Pause,
    Resume,
    Text(String),
    Nothing,
}

fn command(line: &str) -> Line {
    match line.trim() {
        "" => Line::Nothing,
        "/quit" | "/exit" => Line::Quit,
        "/stop" => Line::Stop,
        "/pause" => Line::Pause,
        "/resume" => Line::Resume,
        text => Line::Text(text.to_owned()),
    }
}

async fn drive_interactive(
    handle: &VoiceHandle,
    observer: &Observer,
    args: &ChatArgs,
    typed: bool,
) -> Result<Ended, Failure> {
    let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
    let limit = args.turns.map(|n| n as usize);
    let mut sent = 0_usize;
    if !args.learner_first {
        handle
            .open()
            .await
            .map_err(|e| Failure::other(anyhow!("{e}")))?;
        sent += 1;
    }
    if typed {
        eprintln!("Type a message and press Enter. /quit ends, /stop silences the tutor.");
    } else {
        eprintln!("Speak when you see the tutor listening. Ctrl-C ends. You can also type.");
    }
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = true;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = &mut ctrl_c => return Ok(Ended::Interrupted),
            line = lines.next_line(), if stdin_open => match line {
                Ok(Some(line)) => match command(&line) {
                    Line::Quit => return Ok(Ended::Completed),
                    Line::Stop => handle.stop_speaking().await.map_err(|e| Failure::other(anyhow!("{e}")))?,
                    Line::Pause => handle.pause().await.map_err(|e| Failure::other(anyhow!("{e}")))?,
                    Line::Resume => handle.resume().await.map_err(|e| Failure::other(anyhow!("{e}")))?,
                    Line::Text(text) => {
                        observer.set_next_input("text");
                        handle.send_text(text).await.map_err(|e| Failure::other(anyhow!("{e}")))?;
                        sent += 1;
                    }
                    Line::Nothing => {}
                },
                Ok(None) | Err(_) => stdin_open = false,
            },
            _ = tick.tick() => {
                if observer.provider_unavailable().is_some()
                    && handle.phase() == Phase::ProviderUnavailable
                {
                    return Ok(Ended::ProviderUnavailable);
                }
                if limit.is_some_and(|n| observer.learner_turns_ended() >= n) {
                    return Ok(Ended::Completed);
                }
                // Typed input that has run out ends the session once every
                // message has been answered. With a microphone, there is more to come.
                let idle = matches!(handle.phase(), Phase::Active { turn: TurnState::Listening });
                if !stdin_open && typed && idle && observer.ended() >= sent {
                    return Ok(Ended::Completed);
                }
            }
        }
    }
}
