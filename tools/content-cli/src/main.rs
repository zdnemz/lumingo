#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use content_cli::discover::{parse_kind, validate};
use content_cli::{Format, ValidateOptions, render};

/// Validates Lumingo curriculum units and catalogs.
#[derive(Debug, Parser)]
#[command(name = "content-cli", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check units and catalogs against the schema and the rules of docs/CURRICULUM_SPEC.md
    /// section 6. Exits with 0 only when there is no error. Warnings are listed and do not fail.
    Validate {
        /// Files or folders. A folder is searched for .json files, recursively, and enables the
        /// rules across units (X01, X02, X04, X05).
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,

        /// Require a finished curriculum: every level that has a unit must have all 30 (X01),
        /// every syllabus grammar id must be covered (X05) and catalogs must be complete (K03).
        #[arg(long)]
        complete: bool,

        /// A word list with one `word,LEVEL` per line, for the warnings W01 and W02.
        #[arg(long, value_name = "FILE")]
        word_list: Option<PathBuf>,

        /// The curriculum folder that holds syllabus/ and catalogs/. Found from the first path
        /// when not given.
        #[arg(long, value_name = "DIR")]
        curriculum_root: Option<PathBuf>,

        /// Treat every file as this kind (unit, syllabus, rubric, anchors, placement, errors,
        /// pron_tips, topics) instead of guessing from where it lives.
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate {
            paths,
            format,
            complete,
            word_list,
            curriculum_root,
            kind,
        } => {
            let kind = kind.as_deref().map(parse_kind).transpose()?;
            let options = ValidateOptions {
                paths,
                kind,
                complete,
                word_list,
                curriculum_root,
            };
            let outcome = validate(&options)?;
            let format = match format {
                OutputFormat::Text => Format::Text,
                OutputFormat::Json => Format::Json,
            };
            match format {
                Format::Text => print!("{}", render::text(&outcome)),
                Format::Json => println!("{}", render::json(&outcome)?),
            }
            Ok(outcome.exit_code())
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(err) => {
            // Exit code 2 means the command could not run, as opposed to 1 for validation errors.
            eprintln!("content-cli: {err:#}");
            ExitCode::from(2)
        }
    }
}
