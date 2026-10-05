//! Command line for the model manager.
//!
//! ```text
//! model-manager verify  [--manifest PATH]
//! model-manager licence <id> [--manifest PATH]
//! model-manager install <id> --models-dir DIR --accept-licence [--manifest PATH]
//! model-manager check   <id> --models-dir DIR [--manifest PATH]
//! ```
//!
//! `verify` reads the manifest and exits with status 1 when any entry cannot be
//! downloaded, for example because a checksum is empty. It never touches the
//! network.

use std::path::PathBuf;
use std::process::ExitCode;

use model_manager::{DownloadProgress, ModelError, ModelManager};
use speech::CancelFlag;

const DEFAULT_MANIFEST: &str = "models/manifest.toml";

struct Args {
    command: String,
    id: Option<String>,
    manifest: PathBuf,
    models_dir: Option<PathBuf>,
    accept_licence: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    let mut parsed = Args {
        command,
        id: None,
        manifest: PathBuf::from(DEFAULT_MANIFEST),
        models_dir: None,
        accept_licence: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => {
                parsed.manifest = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--manifest needs a path")?;
            }
            "--models-dir" => {
                parsed.models_dir = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("--models-dir needs a path")?,
                );
            }
            "--accept-licence" => parsed.accept_licence = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => {
                if parsed.id.replace(other.to_owned()).is_some() {
                    return Err("only one model id is accepted".to_owned());
                }
            }
        }
    }
    Ok(parsed)
}

fn usage() -> String {
    "usage: model-manager verify | licence <id> | install <id> --models-dir DIR --accept-licence | check <id> --models-dir DIR  [--manifest PATH]".to_owned()
}

fn run(args: &Args) -> Result<ExitCode, ModelError> {
    // The manager needs a folder for its record. `verify` and `licence` do not
    // read or write it, so a missing --models-dir falls back to a path that is
    // never created.
    let root = args
        .models_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("models/installed"));
    let mut manager = ModelManager::open_manifest_file(&args.manifest, root)?;
    match args.command.as_str() {
        "verify" => {
            let refused = manager.undownloadable();
            for entry in &manager.manifest().models {
                match refused.iter().find(|(e, _)| e.id == entry.id) {
                    Some((_, why)) => println!("refused    {}: {why}", entry.id),
                    None => println!("ok         {}", entry.id),
                }
            }
            Ok(if refused.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        "licence" => {
            let id = need_id(args)?;
            let notice = manager.licence_notice(id)?;
            println!(
                "{}: {} ({})",
                notice.model_id, notice.license, notice.license_url
            );
            if notice.needs_review {
                println!("This licence has use restrictions. Read them in full before you accept.");
            }
            match notice.text {
                Some(text) => println!("\n{text}"),
                None => {
                    println!("\nThe manifest carries no licence text. Read it at the URL above.")
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        "install" => {
            let id = need_id(args)?.to_owned();
            if args.models_dir.is_none() {
                eprintln!("install needs --models-dir");
                return Ok(ExitCode::from(2));
            }
            let notice = manager.licence_notice(&id)?;
            println!(
                "{}: {} ({})",
                notice.model_id, notice.license, notice.license_url
            );
            if !args.accept_licence {
                eprintln!(
                    "Read the licence (model-manager licence {id}) and run again with --accept-licence."
                );
                return Ok(ExitCode::from(2));
            }
            let cancel = CancelFlag::new();
            let mut report = |p: &DownloadProgress| {
                println!(
                    "{} [{}/{}] {} bytes{}",
                    p.file,
                    p.file_index + 1,
                    p.file_count,
                    p.bytes_done,
                    p.bytes_total
                        .map_or_else(String::new, |t| format!(" of {t}"))
                );
            };
            let record = manager.install(&id, &notice.accept(), &cancel, &mut report)?;
            println!("installed {} ({} files)", record.id, record.files.len());
            Ok(ExitCode::SUCCESS)
        }
        "check" => {
            let id = need_id(args)?;
            manager.verify_installed(id, &CancelFlag::new())?;
            println!("{id}: every installed file matches its checksum");
            Ok(ExitCode::SUCCESS)
        }
        _ => {
            eprintln!("{}", usage());
            Ok(ExitCode::from(2))
        }
    }
}

fn need_id(args: &Args) -> Result<&str, ModelError> {
    args.id
        .as_deref()
        .ok_or_else(|| ModelError::UnknownModel("(no id given)".to_owned()))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
