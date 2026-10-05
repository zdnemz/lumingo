//! `pron`: score a reference text against phoneme posteriors, and show the
//! timing mode a stored speed profile leads to.
//!
//! `pron score` reads the posterior matrix either from a JSON file
//! (`--posteriors`, works in every build with the `cli` feature) or from a WAV
//! file through the ONNX model (`--model`, needs the `ort-backend` feature and
//! is UNVERIFIED: it has never run against a real model).
//!
//! The output is labelled experimental, and says plainly when no calibration is
//! loaded: GOP values are measurements, 0..1 scores and flags exist only with a
//! validated configuration.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pron_engine::{
    Arpabet, Calibration, Lexicon, LogPosteriors, Mode, ModeDecision, ModeSetting, ModelVocab,
    Outcome, PhoneMap, PolicyConfig, Reference, ScoreRequest, SpeedProfile, UtteranceReport,
    WordResult, decide, score_utterance,
};

#[derive(Parser)]
#[command(name = "pron", about = "Experimental pronunciation scoring")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Score a reference text against posteriors.
    Score(Box<ScoreArgs>),
    /// Show the blocking or deferred decision for a stored speed profile.
    Mode(ModeArgs),
}

#[derive(Args)]
struct ScoreArgs {
    /// The text the learner was meant to say (or the transcript, with --free-speech).
    #[arg(long)]
    text: String,
    /// A cmudict.dict file you supply. The program ships no dictionary.
    #[arg(long)]
    lexicon: PathBuf,
    /// The model's vocab.json (label to column).
    #[arg(long)]
    vocab: PathBuf,
    /// The vocabulary label that is the CTC blank.
    #[arg(long, default_value = "<pad>")]
    blank: String,
    /// A phone map to use instead of the bundled candidate.
    #[arg(long)]
    phone_map: Option<PathBuf>,
    /// Thresholds and curve. Without it nothing is scored or flagged.
    #[arg(long)]
    calibration: Option<PathBuf>,
    /// Treat the text as an STT transcript (free speech) rather than a drill.
    #[arg(long)]
    free_speech: bool,
    /// With --free-speech: the learner rejected the transcript.
    #[arg(long, requires = "free_speech")]
    transcript_rejected: bool,
    /// Focus phonemes of the drill, comma separated ARPAbet symbols.
    #[arg(long, value_delimiter = ',')]
    focus: Vec<String>,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// Posteriors as JSON: {"frames": n, "labels": l, "log_posteriors": [...]}.
    #[arg(long, conflicts_with_all = ["wav", "model"])]
    posteriors: Option<PathBuf>,
    /// A 16 kHz mono WAV file (needs the ort-backend feature and --model).
    wav: Option<PathBuf>,
    /// The phoneme model, ONNX (needs the ort-backend feature).
    #[arg(long)]
    model: Option<PathBuf>,
    /// The ONNX Runtime library to load (default: ORT_DYLIB_PATH).
    #[arg(long)]
    runtime: Option<PathBuf>,
    /// CPU threads for inference.
    #[arg(long, default_value_t = 2)]
    threads: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum SettingArg {
    Automatic,
    Blocking,
    Deferred,
}

#[derive(Args)]
struct ModeArgs {
    /// A speed profile written by the measurement harness.
    #[arg(long)]
    profile: Option<PathBuf>,
    /// Extra wait before the LLM request that is acceptable, in milliseconds.
    /// Take it from the latency budget; there is no default.
    #[arg(long)]
    max_added_wait_ms: f64,
    /// Utterance length the prediction is made for.
    #[arg(long, default_value_t = pron_engine::perf::LONGEST_TARGET_UTTERANCE_SECONDS)]
    utterance_seconds: f64,
    #[arg(long, value_enum, default_value = "automatic")]
    setting: SettingArg,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Score(args) => score(&args),
        Command::Mode(args) => mode(&args),
    }
}

fn mode(args: &ModeArgs) -> Result<()> {
    let config = PolicyConfig::new(args.utterance_seconds, args.max_added_wait_ms)?;
    let profile = match &args.profile {
        Some(path) => Some(SpeedProfile::load(path)?),
        None => None,
    };
    let setting = match args.setting {
        SettingArg::Automatic => ModeSetting::Automatic,
        SettingArg::Blocking => ModeSetting::Blocking,
        SettingArg::Deferred => ModeSetting::Deferred,
    };
    let decision: ModeDecision =
        decide(profile.as_ref().map(|p| &p.measurement), &config, setting)?;
    println!("pronunciation timing: {decision}");
    if let Some(p) = &profile {
        println!(
            "speed profile {:?}: {} timed runs after {} warm-up, fixture {:.1} s",
            p.label, p.timed_runs, p.warmup_runs, p.fixture_audio_seconds
        );
    }
    Ok(())
}

fn read_text(path: &Path, what: &str) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the {what} {}", path.display()))
}

fn load_posteriors(args: &ScoreArgs, vocab: &ModelVocab) -> Result<LogPosteriors> {
    if let Some(path) = &args.posteriors {
        return Ok(LogPosteriors::from_json(&read_text(
            path,
            "posteriors file",
        )?)?);
    }
    wav_posteriors(args, vocab)
}

#[cfg(feature = "ort-backend")]
fn wav_posteriors(args: &ScoreArgs, vocab: &ModelVocab) -> Result<LogPosteriors> {
    use pron_engine::PosteriorModel;
    use pron_engine::ort_backend::OrtPosteriorModel;

    let (Some(wav), Some(model)) = (&args.wav, &args.model) else {
        bail!("give either --posteriors FILE, or a WAV file together with --model");
    };
    let mut reader = hound::WavReader::open(wav)
        .with_context(|| format!("cannot open the WAV file {}", wav.display()))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != speech::SPEECH_SAMPLE_RATE {
        bail!(
            "the WAV file must be mono at {} Hz (it is {} channel(s) at {} Hz); the program does not resample",
            speech::SPEECH_SAMPLE_RATE,
            spec.channels,
            spec.sample_rate
        );
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mut model =
        OrtPosteriorModel::load(args.runtime.as_deref(), model, vocab.clone(), args.threads)?;
    Ok(model.log_posteriors(&samples, &speech::CancelFlag::new())?)
}

#[cfg(not(feature = "ort-backend"))]
fn wav_posteriors(_args: &ScoreArgs, _vocab: &ModelVocab) -> Result<LogPosteriors> {
    bail!(
        "reading a WAV file needs the ONNX phoneme model, and this build has no `ort-backend` feature; \
         pass --posteriors FILE or rebuild with --features ort-backend"
    )
}

fn score(args: &ScoreArgs) -> Result<()> {
    let lexicon = Lexicon::from_path(&args.lexicon)?;
    let vocab = ModelVocab::from_vocab_json(&read_text(&args.vocab, "vocabulary")?, &args.blank)?;
    let phone_map = match &args.phone_map {
        Some(path) => PhoneMap::from_toml(&read_text(path, "phone map")?)?,
        None => PhoneMap::bundled_candidate()?,
    };
    let bound = phone_map.bind(&vocab)?;
    let calibration = match &args.calibration {
        Some(path) => Calibration::from_toml(&read_text(path, "calibration")?)?,
        None => Calibration::unvalidated(),
    };
    let focus = args
        .focus
        .iter()
        .map(|s| s.parse::<Arpabet>().map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;
    let mode = if args.free_speech {
        Mode::FreeSpeech {
            transcript_rejected: args.transcript_rejected,
        }
    } else {
        Mode::Drill
    };

    let reference = Reference::from_text(&args.text, &lexicon);
    let posteriors = load_posteriors(args, &vocab)?;
    let report = score_utterance(&ScoreRequest {
        reference: &reference,
        posteriors: &posteriors,
        map: &bound,
        vocab: &vocab,
        calibration: &calibration,
        mode,
        focus: &focus,
    })?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report, &phone_map.status);
    }
    Ok(())
}

fn opt(v: Option<f32>) -> String {
    v.map_or_else(|| "-".to_owned(), |x| format!("{x:.2}"))
}

fn print_report(report: &UtteranceReport, phone_map_status: &str) {
    println!("Pronunciation feedback is experimental.");
    if report.scores_calibrated {
        println!(
            "Calibration: loaded, {}.",
            if report.calibration_validated {
                "validated"
            } else {
                "NOT validated"
            }
        );
    } else {
        println!(
            "Calibration: none. GOP is measured; scores and flags are not available until thresholds are validated."
        );
    }
    println!("Phone map status: {phone_map_status}.");
    if let Outcome::NotScored(reason) = &report.outcome {
        println!("NOT SCORED: {reason:?}");
    }
    for word in &report.words {
        match word {
            WordResult::NotChecked { text, reason } => {
                println!("{text}: not checked ({reason:?})");
            }
            WordResult::NotScored { text } => println!("{text}: not scored"),
            WordResult::Scored {
                text,
                pronunciation,
                start_frame,
                end_frame,
                phonemes,
                score,
                flagged_phonemes,
                adjacent_to_unchecked,
            } => {
                let pron: Vec<&str> = pronunciation.iter().map(|p| p.as_str()).collect();
                println!(
                    "{text}  [{}]  frames {start_frame}..{end_frame}  score {}  flagged {flagged_phonemes}{}",
                    pron.join(" "),
                    opt(*score),
                    if *adjacent_to_unchecked {
                        "  (next to a word that was not checked)"
                    } else {
                        ""
                    }
                );
                for p in phonemes {
                    let flag = match p.flagged {
                        Some(true) => "FLAG",
                        Some(false) => "ok",
                        None => "-",
                    };
                    let heard = p
                        .heard
                        .as_ref()
                        .map_or_else(String::new, |h| match h.arpabet {
                            Some(a) => format!("  heard {} ({a})", h.label),
                            None => format!("  heard {}", h.label),
                        });
                    println!(
                        "    {:<3} {:<9} frames {}..{}  GOP {:>6.2}  score {}  {flag}{heard}",
                        p.symbol.as_str(),
                        format!("{:?}", p.class).to_lowercase(),
                        p.start_frame,
                        p.end_frame,
                        p.gop,
                        opt(p.score),
                    );
                }
            }
        }
    }
    for f in &report.focus {
        println!(
            "focus {}: {} occurrence(s), mean GOP {}, mean score {}, {} flagged",
            f.symbol,
            f.occurrences,
            opt(f.mean_gop),
            opt(f.mean_score),
            f.flagged
        );
    }
    println!(
        "utterance score {}  ({} word(s) scored, {} not checked, {} frames)",
        opt(report.utterance_score),
        report.words_scored,
        report.words_not_checked,
        report.frames
    );
    if !report.highlighted_words.is_empty() {
        println!(
            "highlighted words (free speech): {:?}",
            report.highlighted_words
        );
    }
}
