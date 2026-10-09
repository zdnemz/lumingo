//! `tutor-cli probe`: the capability probe (the connection test) on the command
//! line. It is the live check for the two gateway quirks found on 2026-10-07
//! (S4-06): a gateway that answers `anthropic_messages` with an OpenAI-shaped
//! body, and one that silently ignores the native JSON schema (the probe's
//! canary catches that one and walks to a level that works).
//!
//! It makes a handful of small requests and prints what each step found. The
//! protocol and the model name are printed; the key never is.

use std::time::Duration;

use anyhow::anyhow;

use crate::args::ProbeArgs;
use crate::chat::Failure;
use crate::exit::Exit;
use crate::provider;

pub async fn run(args: ProbeArgs) -> Exit {
    match execute(&args).await {
        Ok(exit) => exit,
        Err(failure) => {
            eprintln!("error: {:#}", failure.error);
            failure.exit
        }
    }
}

async fn execute(args: &ProbeArgs) -> Result<Exit, Failure> {
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
    println!(
        "provider {} ({}, model {})",
        connected.host, connected.protocol, connected.model
    );
    match connected.probe().await {
        provider::ProbeOutcome::StructuredWorks => Ok(Exit::Done),
        provider::ProbeOutcome::NoStructuredLevel => {
            eprintln!("no structured-output ladder level worked");
            Ok(Exit::Failure)
        }
        provider::ProbeOutcome::KeyRejected => Err(Failure {
            exit: Exit::ProviderUnavailable,
            error: anyhow!("the provider did not accept the key"),
        }),
        provider::ProbeOutcome::Unavailable => Err(Failure {
            exit: Exit::ProviderUnavailable,
            error: anyhow!("the capability probe did not finish"),
        }),
    }
}
