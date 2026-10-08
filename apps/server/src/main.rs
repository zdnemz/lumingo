use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use app_core::engines::{EngineOptions, Engines};
use app_core::{AppCore, CoreConfig, SessionManager, default_curriculum_dir, default_data_dir};
use clap::Parser;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use tutor_server::security::Guard;
use tutor_server::{AppState, build_router, run_heartbeat};

/// How many consecutive ports are tried when the first one is taken.
const PORT_ATTEMPTS: u16 = 100;

#[derive(Debug, Parser)]
#[command(name = "tutor-server", version, about = "Lumingo local server")]
struct Args {
    /// First port to try. If it is taken, the next free one is used.
    #[arg(long, default_value_t = 8765)]
    port: u16,
    /// Do not open the default browser.
    #[arg(long)]
    no_open: bool,
    /// Development mode: allow exactly one extra origin, the `next dev` address.
    #[arg(long)]
    dev: bool,
    /// The one origin allowed in development mode.
    #[arg(long, default_value = "http://localhost:3000", requires = "dev")]
    dev_origin: String,
    /// Folder for the database, `providers.toml` and recordings. Default: the
    /// per-user data folder (`%LOCALAPPDATA%\Lumingo` on Windows).
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Folder of unit files. Default: `curriculum/units` next to the executable,
    /// otherwise in the working directory.
    #[arg(long)]
    curriculum_dir: Option<PathBuf>,
    /// The engines file that names the speech model files. Default:
    /// `engines.toml` in the data folder. Relative paths in it are read from the
    /// `models` folder of the data folder.
    #[arg(long)]
    engines_file: Option<PathBuf>,
    /// The model manifest. Default: `models/manifest.toml` next to the
    /// executable, otherwise in the working directory.
    #[arg(long)]
    models_manifest: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args = Args::parse();

    let (listener, port) = bind_loopback(args.port).await?;
    let dev_origin = args.dev.then(|| args.dev_origin.clone());
    let guard = Arc::new(
        Guard::new(port, dev_origin.clone()).context("could not create the session secret")?,
    );
    let address = format!("http://127.0.0.1:{port}");
    let data_dir = match args.data_dir {
        Some(dir) => dir,
        None => default_data_dir().context("no per-user data folder was found; pass --data-dir")?,
    };
    let mut config = CoreConfig::new(
        data_dir.clone(),
        args.curriculum_dir.unwrap_or_else(default_curriculum_dir),
    );
    if let Some(manifest) = args.models_manifest {
        config.models_manifest = manifest;
    }
    config.dev_mode = args.dev;
    config.server_address = Some(address.clone());
    let core = AppCore::open(config)
        .await
        .context("the application core could not start")?;
    // What the cargo features and the files on disk allow. A build with every
    // feature off finds no audio and no speech and says why; text chat, lessons,
    // writing and reading work without them.
    let options = EngineOptions {
        data_dir,
        engines_file: args.engines_file,
    };
    let engines = tokio::task::spawn_blocking(move || Engines::detect(&options))
        .await
        .context("looking for the audio and speech engines did not finish")?;
    SessionManager::attach(&core, engines)
        .await
        .context("the session manager could not start")?;
    let state = Arc::new(AppState::new(Arc::clone(&core), guard));

    println!("Lumingo is running at {address}");
    if let Some(origin) = &dev_origin {
        println!("Development mode: requests from {origin} are allowed.");
    }
    if !args.no_open && !args.dev && webbrowser::open(&address).is_err() {
        tracing::warn!("could not open the default browser; open the address above yourself");
    }

    tokio::spawn(run_heartbeat(Arc::clone(&state)));
    let app = build_router(state);
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(core.shutdown_token()))
        .await;
    // Close the database even when the server stopped with an error.
    let closed = core.close().await;
    served.context("the server stopped with an error")?;
    closed.context("the database did not close cleanly")?;
    tracing::info!("server stopped");
    Ok(())
}

/// Binds the first free port at or after `first`, on the loopback address only.
async fn bind_loopback(first: u16) -> Result<(TcpListener, u16)> {
    for offset in 0..PORT_ATTEMPTS {
        let Some(port) = first.checked_add(offset) else {
            break;
        };
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
            Ok(listener) => return Ok((listener, port)),
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {}
            Err(err) => return Err(err).context("could not bind the loopback address"),
        }
    }
    bail!("no free port found from {first} onward")
}

/// Completes on Ctrl+C or, on Windows, when the console window is closed, then
/// tells every task to stop.
async fn shutdown_signal(shutdown: CancellationToken) {
    #[cfg(windows)]
    {
        use tokio::signal::windows;
        match windows::ctrl_close() {
            Ok(mut close) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = close.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    shutdown.cancel();
}
