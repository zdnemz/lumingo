//! [`AppCore`]: the object the server holds. Its methods are the typed command
//! and query API; the route groups live in the sibling modules, each as an
//! `impl AppCore` block.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use std::time::Instant;

use storage::{Database, L1HelpMode, NewProfile, Profile, Timestamp, UiLanguage};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::api::{Feature, HardwareProfile, ServerEvent, Settings, StateSnapshot};
use crate::clock::Clock;
use crate::config::CoreConfig;
use crate::error::{CoreError, CoreResult};
use crate::events::EventBus;
use crate::providers::ProviderHub;
use crate::session::SessionService;
use crate::units::UnitHub;
use crate::{hardware, settings};

/// The default name of the one learner profile, until the learner sets one.
const DEFAULT_DISPLAY_NAME: &str = "Learner";

/// The running core. Cheap to share: the server holds an `Arc<AppCore>`.
pub struct AppCore {
    pub(crate) config: CoreConfig,
    pub(crate) db: Database,
    pub(crate) bus: EventBus,
    pub(crate) started: Instant,
    /// Id of the one learner profile. It changes when "delete all data"
    /// recreates the profile.
    pub(crate) profile_id: AtomicI64,
    pub(crate) settings: RwLock<Settings>,
    /// Serialises settings writes, so two saves cannot interleave their two
    /// database writes.
    pub(crate) settings_write: Mutex<()>,
    pub(crate) providers: ProviderHub,
    pub(crate) units: UnitHub,
    pub(crate) hardware: HardwareProfile,
    pub(crate) shutdown: CancellationToken,
    pub(crate) sessions: OnceLock<Arc<dyn SessionService>>,
}

impl std::fmt::Debug for AppCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppCore")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AppCore {
    /// Starts the core: creates the data directory, opens and migrates the
    /// database, makes sure the learner profile exists and measures the machine.
    pub async fn open(config: CoreConfig) -> CoreResult<Arc<Self>> {
        tokio::fs::create_dir_all(&config.data_dir)
            .await
            .map_err(|error| CoreError::io("creating the data directory", &error))?;
        let db = Database::open(config.database_path()).await?;
        let profile = ensure_profile(&db, config.clock.as_ref(), None).await?;
        let loaded = settings::load(&db, &profile).await?;
        let hardware = match config.hardware.clone() {
            Some(profile) => profile,
            None => tokio::task::spawn_blocking(hardware::detect)
                .await
                .map_err(|_| CoreError::Internal("the hardware probe did not finish".to_owned()))?,
        };
        let providers = ProviderHub::load(&config, &db, &config.clock.now()).await?;
        let units = UnitHub::load(&config.curriculum_dir, &db, &config.clock.now()).await?;
        let core = Arc::new(Self {
            config,
            db,
            bus: EventBus::new(),
            started: Instant::now(),
            profile_id: AtomicI64::new(profile.id),
            settings: RwLock::new(loaded),
            settings_write: Mutex::new(()),
            providers,
            units,
            hardware,
            shutdown: CancellationToken::new(),
            sessions: OnceLock::new(),
        });
        core.sync_unlocks().await?;
        Ok(core)
    }

    /// Tells every task that stops with the program to stop: open event
    /// streams close and provider tests are cancelled. New commands that change
    /// data are refused from now on. It does not wait for anything; call
    /// [`AppCore::close`] when the server has finished.
    pub fn request_shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Stops the running session, if a session service is attached, and flushes
    /// and closes the database.
    pub async fn close(&self) -> CoreResult<()> {
        self.request_shutdown();
        if let Some(sessions) = self.sessions.get() {
            sessions.stop_active().await?;
        }
        self.db.close().await?;
        Ok(())
    }

    /// Cancelled when the program is stopping. Long operations take a child.
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub fn config(&self) -> &CoreConfig {
        &self.config
    }

    /// The database, for the crates that write learning data (the tutor engine,
    /// the assessment code). Nothing in the HTTP layer uses it.
    pub fn database(&self) -> &Database {
        &self.db
    }

    pub fn clock(&self) -> &dyn Clock {
        self.config.clock.as_ref()
    }

    pub fn dev_mode(&self) -> bool {
        self.config.dev_mode
    }

    /// The id of the learner profile.
    pub fn profile_id(&self) -> i64 {
        self.profile_id.load(Ordering::SeqCst)
    }

    /// Refuses a command that changes data once shutdown has begun.
    pub(crate) fn ensure_running(&self) -> CoreResult<()> {
        if self.shutdown.is_cancelled() {
            Err(CoreError::ShuttingDown)
        } else {
            Ok(())
        }
    }

    // ---- state and events ----------------------------------------------

    /// The full state. Built from memory only, so it is cheap and cannot fail.
    pub fn snapshot(&self) -> StateSnapshot {
        let settings = self
            .settings
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let attached = self.sessions.get().is_some();
        StateSnapshot {
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            dev_mode: self.config.dev_mode,
            uptime_ms: self.uptime_ms(),
            settings,
            provider: self.providers.active(),
            hardware: self.hardware.clone(),
            unavailable: Feature::ALL
                .into_iter()
                .filter(|feature| !(attached && *feature == Feature::Sessions))
                .collect(),
        }
    }

    /// A `Snapshot` event stamped with the current sequence number.
    pub fn snapshot_event(&self) -> ServerEvent {
        self.bus.snapshot_event(|| self.snapshot())
    }

    /// The event bus. Subscribe before calling [`AppCore::snapshot_event`].
    pub fn events(&self) -> &EventBus {
        &self.bus
    }

    /// Publishes a fresh `Snapshot` to every client, for changes that replace
    /// state wholesale: new settings, or all data deleted.
    pub fn publish_snapshot(&self) {
        self.bus.publish(|seq| ServerEvent::Snapshot {
            seq,
            state: self.snapshot(),
        });
    }

    /// Publishes a heartbeat.
    pub fn publish_heartbeat(&self) {
        self.bus.publish(|seq| ServerEvent::Heartbeat {
            seq,
            uptime_ms: self.uptime_ms(),
        });
    }

    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    // ---- sessions boundary ---------------------------------------------

    /// Attaches the session orchestration. It can be attached once.
    pub fn attach_sessions(&self, service: Arc<dyn SessionService>) -> CoreResult<()> {
        self.sessions
            .set(service)
            .map_err(|_| CoreError::Conflict("a session service is already attached".to_owned()))
    }

    /// The attached session orchestration, or `NotAvailable`.
    pub fn session_service(&self) -> CoreResult<Arc<dyn SessionService>> {
        self.sessions
            .get()
            .cloned()
            .ok_or(CoreError::NotAvailable(Feature::Sessions))
    }

    // ---- settings --------------------------------------------------------

    /// The current settings.
    pub fn settings(&self) -> Settings {
        self.snapshot().settings
    }

    /// Replaces all settings after checking them, and returns what is stored.
    pub async fn update_settings(&self, new: Settings) -> CoreResult<Settings> {
        self.ensure_running()?;
        let new = new.validated()?;
        let _write = self.settings_write.lock().await;
        let profile = self
            .db
            .profiles()
            .get(self.profile_id())
            .await?
            .ok_or(CoreError::NotFound { what: "profile" })?;
        settings::save(&self.db, &profile, &new, &self.clock().now()).await?;
        *self
            .settings
            .write()
            .unwrap_or_else(PoisonError::into_inner) = new.clone();
        self.publish_snapshot();
        Ok(new)
    }
}

/// The first profile, created with the schema defaults when there is none.
/// `ui_language` seeds a new profile; an existing one is returned untouched.
pub(crate) async fn ensure_profile(
    db: &Database,
    clock: &dyn Clock,
    ui_language: Option<UiLanguage>,
) -> CoreResult<Profile> {
    if let Some(profile) = db.profiles().first().await? {
        return Ok(profile);
    }
    let created_at: Timestamp = clock.now();
    let profile = db
        .profiles()
        .create(&NewProfile {
            display_name: DEFAULT_DISPLAY_NAME.to_owned(),
            ui_language: ui_language.unwrap_or(UiLanguage::Id),
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            created_at,
        })
        .await?;
    Ok(profile)
}
