//! Command-line arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "tutor-cli",
    version,
    about = "The Lumingo voice loop on the command line: listen, transcribe, think, speak",
    long_about = "Runs the tutor's voice loop with the T1 prompt and a fixed scenario from a \
                  unit file, before the UI exists. Real audio and real speech models need the \
                  cpal-backend and sherpa cargo features and have not been run in the build \
                  container: see tools/tutor-cli/README.md."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Hold a conversation with the tutor, or run a script of turns.
    Chat(ChatArgs),
    /// Play a unit from its first activity to its checkpoint, or score the
    /// responses that waited for a provider.
    Unit(UnitArgs),
}

#[derive(Debug, Args)]
pub struct UnitArgs {
    #[command(subcommand)]
    pub command: UnitCommand,
}

#[derive(Debug, Subcommand)]
pub enum UnitCommand {
    /// Play a unit with scripted or typed responses and print the scores.
    Run(UnitRunArgs),
    /// Score the responses of earlier runs that were stored as pending because
    /// no provider could be reached.
    ScorePending(ScorePendingArgs),
}

#[derive(Debug, Clone, Args)]
#[command(group(clap::ArgGroup::new("responses").required(true).args(["script", "interactive"])))]
pub struct UnitRunArgs {
    /// The unit file.
    pub unit: PathBuf,
    /// A JSON file of responses, one entry per activity id (see
    /// tools/tutor-cli/README.md).
    #[arg(long)]
    pub script: Option<PathBuf>,
    /// Type the responses at the terminal instead of reading a script.
    #[arg(long)]
    pub interactive: bool,
    /// The folder of rubric files. Defaults to `catalogs/rubrics` next to the
    /// unit's folder.
    #[arg(long)]
    pub rubrics: Option<PathBuf>,
    /// A word list, one `word,LEVEL` per line, for the vocabulary profile and the
    /// range cross-check.
    #[arg(long)]
    pub word_list: Option<PathBuf>,
    /// The database file the attempts are written to. A new file in the
    /// temporary folder when not given; the run prints its path.
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Write the scores, the statuses and the checkpoint as one JSON file.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// The learner's first language, written in English.
    #[arg(long, default_value = "Indonesian")]
    pub first_language: String,
    /// Do not print the text of audio items. By default it is printed, because no
    /// speaker is attached to this tool.
    #[arg(long)]
    pub hide_audio_text: bool,
    /// Do not look for a provider: productive responses are stored as pending.
    #[arg(long)]
    pub offline: bool,
    /// The provider profile: `env` for the TUTOR_LLM_* variables or a `.env` file,
    /// or the name of a profile in providers.toml.
    #[arg(long)]
    pub provider: Option<String>,
    /// providers.toml. Defaults to the one in the data directory.
    #[arg(long)]
    pub providers_file: Option<PathBuf>,
    /// The data directory (providers.toml).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// The whole time, in milliseconds, the provider gets to start a reply.
    #[arg(long, default_value_t = 16_000)]
    pub provider_timeout_ms: u64,
}

#[derive(Debug, Clone, Args)]
pub struct ScorePendingArgs {
    /// The database of the earlier run.
    #[arg(long)]
    pub db: PathBuf,
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long)]
    pub providers_file: Option<PathBuf>,
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 16_000)]
    pub provider_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ModeArg {
    Fluency,
    Accuracy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendArg {
    /// Microphone and speakers through cpal (needs the `cpal-backend` feature).
    Cpal,
    /// Fake devices and fake engines (needs the `test-support` feature; for tests).
    Fake,
}

#[derive(Debug, Clone, Args)]
pub struct ChatArgs {
    /// The unit file whose roleplay is the scenario.
    #[arg(long, default_value = "curriculum/examples/a1-u01.example.json")]
    pub scenario: PathBuf,
    /// The roleplay activity of the unit. The first one when not given.
    #[arg(long)]
    pub activity: Option<String>,
    /// Feedback mode. The activity's own mode when not given.
    #[arg(long, value_enum)]
    pub mode: Option<ModeArg>,
    /// The learner's first language, written in English.
    #[arg(long, default_value = "Indonesian")]
    pub first_language: String,
    /// The provider profile: `env` for the TUTOR_LLM_* variables or a `.env` file,
    /// or the name of a profile in providers.toml. With one profile, that one.
    #[arg(long)]
    pub provider: Option<String>,
    /// providers.toml. Defaults to the one in the data directory.
    #[arg(long)]
    pub providers_file: Option<PathBuf>,
    /// The data directory (database, providers.toml).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Audio backend for voice mode.
    #[arg(long, value_enum, default_value = "cpal")]
    pub backend: BackendArg,
    /// Typed input: no microphone and no speech recognition. Replies are printed;
    /// add --speak to hear them.
    #[arg(long)]
    pub text: bool,
    /// Speak the replies even though the input is typed (needs a speech engine and
    /// an output device). Voice input speaks them by default.
    #[arg(long)]
    pub speak: bool,
    /// Do not speak the replies, for example in a script run of recorded utterances.
    #[arg(long, conflicts_with = "speak")]
    pub no_speak: bool,
    /// A script of turns, one per line: text to type, or `wav: path` for a recorded
    /// utterance fed through the VAD and the recogniser. Lines starting with # are comments.
    #[arg(long)]
    pub script: Option<PathBuf>,
    /// The number of turns to run. With a script shorter than this, it is repeated.
    #[arg(long)]
    pub turns: Option<u32>,
    /// Where a script run writes its result file (JSON lines).
    #[arg(long, default_value = "tutor-cli-result.jsonl")]
    pub out: PathBuf,
    /// The learner speaks first; the tutor waits instead of opening the conversation.
    #[arg(long)]
    pub learner_first: bool,
    /// The whole time, in milliseconds, the provider gets to start a reply for a
    /// turn, both attempts together.
    #[arg(long, default_value_t = 16_000)]
    pub provider_timeout_ms: u64,
    /// Silence that ends an utterance, 400 to 900 ms.
    #[arg(long, default_value_t = 600)]
    pub end_silence_ms: u32,
    /// Store the session and run the background analysis (T2) through the provider.
    #[arg(long)]
    pub analysis: bool,
    /// Print every phase of the session as it changes.
    #[arg(long)]
    pub show_states: bool,
    /// A TOML file naming the sherpa models (needs the `sherpa` feature).
    #[arg(long)]
    pub engines: Option<PathBuf>,
}

pub fn parse() -> Cli {
    Cli::parse()
}
