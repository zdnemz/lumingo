use std::process::ExitCode;

use tutor_cli::args::{self, Command};
use tutor_cli::chat;

#[tokio::main]
async fn main() -> ExitCode {
    // Logs go to stderr and stay at warnings unless RUST_LOG says more. No log
    // line at info or above carries learner text, a prompt, a reply or a key.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    match args::parse().command {
        Command::Chat(args) => chat::run(args).await.into(),
    }
}
