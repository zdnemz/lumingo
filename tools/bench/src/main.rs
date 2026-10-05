use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use bench::machine::MachineInfo;
use bench::profile::{Profile, check_profile};
use bench::result::BenchResult;
use bench::selftest;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProfileArg {
    Dev,
    Floor,
}

impl From<ProfileArg> for Profile {
    fn from(value: ProfileArg) -> Self {
        match value {
            ProfileArg::Dev => Self::Dev,
            ProfileArg::Floor => Self::Floor,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "bench", about = "Lumingo benchmark harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print what the harness detects about this machine, and whether it may tag a run `floor`.
    Machine,
    /// Measure the harness itself and write cold and warm result files.
    Selftest {
        #[arg(long, value_enum, default_value = "dev")]
        profile: ProfileArg,
        #[arg(long, default_value_t = 120)]
        samples: usize,
        #[arg(long, default_value = "benchmarks/results")]
        out: PathBuf,
    },
    /// Validate result files, or every `.json` file in a folder.
    Check { paths: Vec<PathBuf> },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Machine => {
            let machine = MachineInfo::detect();
            println!("{}", serde_json::to_string_pretty(&machine)?);
            match check_profile(Profile::Floor, &machine) {
                Ok(()) => println!("This machine may be tagged `floor`."),
                Err(error) => println!("{error}"),
            }
            Ok(())
        }
        Command::Selftest {
            profile,
            samples,
            out,
        } => {
            for path in selftest::run(profile.into(), samples, &out)? {
                println!("wrote {}", path.display());
            }
            Ok(())
        }
        Command::Check { paths } => {
            let mut files = Vec::new();
            for path in paths {
                if path.is_dir() {
                    for entry in std::fs::read_dir(&path)
                        .with_context(|| format!("reading {}", path.display()))?
                    {
                        let entry_path = entry?.path();
                        if entry_path.extension().is_some_and(|ext| ext == "json") {
                            files.push(entry_path);
                        }
                    }
                } else {
                    files.push(path);
                }
            }
            if files.is_empty() {
                bail!("no result files to check");
            }
            let mut failed = false;
            for file in &files {
                match BenchResult::read_from(file) {
                    Ok(_) => println!("ok      {}", file.display()),
                    Err(error) => {
                        failed = true;
                        println!("INVALID {}: {error}", file.display());
                    }
                }
            }
            if failed {
                bail!("at least one result file is invalid");
            }
            Ok(())
        }
    }
}
