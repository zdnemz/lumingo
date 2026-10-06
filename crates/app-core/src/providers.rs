//! Provider profile management over `llm-client`.
//!
//! Two stores cooperate. `llm_client::ProfileSet` owns the profiles and their
//! keys: the read-only `env` profile from the environment or a `.env` file, and
//! the saved ones in `providers.toml`. The database table `provider_profiles`
//! owns what has no key in it: which profile is active, the last probe result
//! and the qualification mark. [`reconcile`] makes the table follow the set.
//!
//! No function here returns a key. Everything the UI sees is a
//! [`ProviderInfo`], built from `llm_client::ProfileInfo` (`has_key`, last four
//! characters) and the table row.
//!
//! Rules kept here:
//! - While profiles exist, exactly one is active.
//! - A profile saved again with a changed address, model or protocol is a new
//!   row, because its old probe result no longer describes it. It keeps the
//!   active mark.
//! - A `providers.toml` that cannot be read is never overwritten. Saving is
//!   refused until the learner fixes or removes the file.
//! - At most one connection test runs at a time; a second request is refused
//!   with `Busy` instead of queued.

use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Instant;

use llm_client::{
    Capabilities, ClientOptions, LlmClient, LlmError, ProfileError, ProfileInfo, ProfileSet,
    ProfileSource, ProviderClient,
};
use storage::{Database, KeySource, NewProviderProfile, ProviderProfile as ProviderRow, Timestamp};
use tokio::sync::Mutex;

use crate::api::{
    ProbeFailure, ProbeFailureKind, ProbeReport, ProviderCapabilities, ProviderInfo, ProviderList,
    ProviderSource, SaveProviderRequest, ServerEvent,
};
use crate::config::CoreConfig;
use crate::core::AppCore;
use crate::error::{CoreError, CoreResult};

/// A name no `providers.toml` can have, used to build an empty saved set when
/// the real file is unreadable.
const ABSENT_FILE: &str = ".providers-file-is-unreadable";

/// The provider state the core keeps in memory.
pub(crate) struct ProviderHub {
    state: RwLock<State>,
    /// Serialises changes, so a save and a delete cannot interleave their file
    /// and database writes.
    write: Mutex<()>,
    /// One connection test at a time.
    probe_gate: Mutex<()>,
}

struct State {
    set: ProfileSet,
    infos: Vec<ProviderInfo>,
    problems: Vec<String>,
    /// True when `providers.toml` exists but could not be read.
    file_broken: bool,
    client: Option<Arc<ProviderClient>>,
}

impl ProviderHub {
    /// Reads both profile sources and brings the table in line with them.
    pub(crate) async fn load(
        config: &CoreConfig,
        db: &Database,
        now: &Timestamp,
    ) -> CoreResult<Self> {
        let loader = config.env_profiles.clone();
        let lookup = Arc::clone(&config.env_lookup);
        let path = config.providers_path();
        let absent = config.data_dir.join(ABSENT_FILE);
        let (set, problems, file_broken) = tokio::task::spawn_blocking(move || {
            read_sources(&loader, lookup.as_ref(), &path, &absent)
        })
        .await
        .map_err(|_| CoreError::Internal("reading the provider files did not finish".to_owned()))?;
        let infos = reconcile(db, &set, now).await?;
        Ok(Self {
            state: RwLock::new(State {
                set,
                infos,
                problems,
                file_broken,
                client: None,
            }),
            write: Mutex::new(()),
            probe_gate: Mutex::new(()),
        })
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The active profile, for the snapshot.
    pub(crate) fn active(&self) -> Option<ProviderInfo> {
        self.read().infos.iter().find(|p| p.is_active).cloned()
    }

    /// The profiles as an export may show them: name, protocol, address and
    /// model. Not the key, and not whether there is one or how it ends.
    pub(crate) fn exportable(&self) -> Vec<serde_json::Value> {
        self.read()
            .infos
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "protocol": p.protocol,
                    "base_url": p.base_url,
                    "model": p.model,
                    "source": p.source,
                })
            })
            .collect()
    }

    fn list(&self) -> ProviderList {
        let state = self.read();
        ProviderList {
            providers: state.infos.clone(),
            active_id: state.infos.iter().find(|p| p.is_active).map(|p| p.id),
            problems: state.problems.clone(),
        }
    }
}

/// Reads the `env` profile and `providers.toml`. Problems become sentences for
/// the UI; none of them contains a key.
fn read_sources(
    loader: &llm_client::EnvProfileLoader,
    lookup: &(dyn Fn(&str) -> Option<String> + Send + Sync),
    path: &Path,
    absent: &Path,
) -> (ProfileSet, Vec<String>, bool) {
    let mut problems = Vec::new();
    let env = match loader.load_with(lookup) {
        Ok(profile) => profile,
        Err(error) => {
            problems.push(format!("environment profile: {error}"));
            None
        }
    };
    match ProfileSet::load(env.clone(), path) {
        Ok(set) => (set, problems, false),
        Err(error) => {
            problems.push(format!("providers file: {error}"));
            // The environment profile still works; the saved ones do not exist
            // as far as this run is concerned.
            let set = ProfileSet::load(env, absent).unwrap_or_default();
            (set, problems, true)
        }
    }
}

fn same_profile(row: &ProviderRow, wanted: &ProfileInfo) -> bool {
    row.protocol.as_str() == wanted.protocol.as_str()
        && row.base_url == wanted.base_url
        && row.model == wanted.model
        && row.key_source
            == match wanted.source {
                ProfileSource::Env => KeySource::Env,
                ProfileSource::File => KeySource::File,
            }
}

fn storage_protocol(protocol: llm_client::Protocol) -> storage::ProviderProtocol {
    match protocol {
        llm_client::Protocol::OpenAiChat => storage::ProviderProtocol::OpenaiChat,
        llm_client::Protocol::AnthropicMessages => storage::ProviderProtocol::AnthropicMessages,
    }
}

/// Makes `provider_profiles` follow `set`, and returns the merged view.
async fn reconcile(
    db: &Database,
    set: &ProfileSet,
    now: &Timestamp,
) -> CoreResult<Vec<ProviderInfo>> {
    let wanted = set.list();
    let rows = db.providers().list().await?;
    let mut active_name = rows.iter().find(|r| r.is_active).map(|r| r.name.clone());
    for row in &rows {
        match wanted.iter().find(|w| w.name == row.name) {
            Some(w) if same_profile(row, w) => {}
            Some(_) => db.providers().delete(row.id).await?,
            None => {
                db.providers().delete(row.id).await?;
                if active_name.as_deref() == Some(row.name.as_str()) {
                    active_name = None;
                }
            }
        }
    }

    let mut rows = db.providers().list().await?;
    for w in &wanted {
        if rows.iter().any(|r| r.name == w.name) {
            continue;
        }
        let created = db
            .providers()
            .create(&NewProviderProfile {
                name: w.name.clone(),
                protocol: storage_protocol(w.protocol),
                base_url: w.base_url.clone(),
                model: w.model.clone(),
                key_source: match w.source {
                    ProfileSource::Env => KeySource::Env,
                    ProfileSource::File => KeySource::File,
                },
                created_at: *now,
            })
            .await?;
        rows.push(created);
    }

    // A recreated row takes over the active mark of the one it replaces. When
    // nothing was active and profiles exist, the first one (the `env` profile
    // when there is one) becomes active.
    if !rows.iter().any(|r| r.is_active) {
        let pick = active_name
            .as_deref()
            .and_then(|name| rows.iter().find(|r| r.name == name))
            .or_else(|| {
                wanted
                    .first()
                    .and_then(|w| rows.iter().find(|r| r.name == w.name))
            });
        if let Some(row) = pick {
            db.providers().set_active(row.id).await?;
        }
    }

    let rows = db.providers().list().await?;
    Ok(wanted
        .iter()
        .filter_map(|w| {
            let row = rows.iter().find(|r| r.name == w.name)?;
            Some(info_from(w, row))
        })
        .collect())
}

fn info_from(profile: &ProfileInfo, row: &ProviderRow) -> ProviderInfo {
    ProviderInfo {
        id: row.id,
        name: profile.name.clone(),
        protocol: profile.protocol.into(),
        base_url: profile.base_url.clone(),
        model: profile.model.clone(),
        has_key: profile.has_key,
        key_last4: profile.key_last4.clone(),
        source: profile.source.into(),
        is_active: row.is_active,
        capabilities: stored_capabilities(row).map(|caps| capabilities_view(&caps)),
        probed_at: row.probed_at.map(|t| t.to_string()),
        qualified_at: row.qualified_at.map(|t| t.to_string()),
    }
}

fn stored_capabilities(row: &ProviderRow) -> Option<Capabilities> {
    row.capabilities
        .as_ref()
        .and_then(|json| serde_json::from_value(json.clone()).ok())
}

fn capabilities_view(caps: &Capabilities) -> ProviderCapabilities {
    ProviderCapabilities {
        probe_version: caps.probe_version,
        auth_ok: caps.auth_ok,
        stream_ok: caps.stream_ok,
        ttft_ms: caps.ttft_ms,
        tokens_per_second: caps.tokens_per_second,
        structured_level: caps.structured_level.map(llm_client::LadderLevel::as_u8),
        contracts_ok: caps.contracts_ok.clone(),
        rate_limit_rpm: caps.rate_limit.rpm,
        rate_limit_rpd: caps.rate_limit.rpd,
    }
}

fn failure_from(error: &LlmError) -> ProbeFailure {
    let kind = match error {
        LlmError::Cancelled => ProbeFailureKind::Cancelled,
        LlmError::Timeout(_) => ProbeFailureKind::Timeout,
        LlmError::Transport(_) => ProbeFailureKind::Network,
        LlmError::Auth { .. } => ProbeFailureKind::Auth,
        LlmError::RateLimited { .. } => ProbeFailureKind::RateLimited,
        LlmError::Server { .. } => ProbeFailureKind::ProviderError,
        LlmError::Rejected { .. }
        | LlmError::Protocol(_)
        | LlmError::Refusal
        | LlmError::InvalidOutput(_)
        | LlmError::Stream { .. } => ProbeFailureKind::UnexpectedReply,
        LlmError::HostNotAllowed { .. }
        | LlmError::InsecureScheme { .. }
        | LlmError::InvalidRequest(_) => ProbeFailureKind::Configuration,
    };
    ProbeFailure {
        kind,
        // `LlmError` messages carry fixed categories or sanitised provider text.
        message: error.to_string(),
    }
}

fn profile_error(error: &ProfileError) -> CoreError {
    match error {
        ProfileError::ReservedName => {
            CoreError::ReadOnly("the name `env` belongs to the read-only environment profile")
        }
        other => CoreError::InvalidInput(other.to_string()),
    }
}

impl AppCore {
    /// Every provider profile and which one is active.
    pub fn list_providers(&self) -> ProviderList {
        self.providers.list()
    }

    /// Creates a saved profile or replaces the one with the same name, writes
    /// `providers.toml`, and returns the profile as the UI may see it.
    pub async fn save_provider(&self, request: SaveProviderRequest) -> CoreResult<ProviderInfo> {
        self.ensure_running()?;
        let _write = self.providers.write.lock().await;
        let (mut set, file_broken) = {
            let state = self.providers.read();
            (state.set.clone(), state.file_broken)
        };
        if file_broken {
            return Err(CoreError::Conflict(
                "providers.toml cannot be read, so it is not overwritten; fix or remove the file first"
                    .to_owned(),
            ));
        }

        let existing_key = set
            .get(request.name.trim())
            .filter(|p| p.source == ProfileSource::File)
            .and_then(|p| p.key.clone());
        let new_key = request.api_key.as_ref().map(|k| k.expose().to_owned());
        let mut profile = llm_client::ProviderProfile::new(
            &request.name,
            request.protocol.into(),
            &request.base_url,
            &request.model,
            new_key.as_deref(),
            ProfileSource::File,
        )
        .map_err(|error| profile_error(&error))?;
        if new_key.is_none() && request.clear_key != Some(true) {
            profile.key = existing_key;
        }
        let name = profile.name.clone();
        set.upsert(profile).map_err(|error| profile_error(&error))?;
        self.persist_providers(set.clone()).await?;

        let infos = reconcile(&self.db, &set, &self.clock().now()).await?;
        if request.make_active == Some(true) {
            let id = infos
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.id)
                .ok_or(CoreError::NotFound { what: "provider" })?;
            self.db.providers().set_active(id).await?;
        }
        self.apply_provider_set(set).await?;
        self.providers
            .read()
            .infos
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .ok_or(CoreError::NotFound { what: "provider" })
    }

    /// Removes a saved profile. The `env` profile cannot be removed.
    pub async fn delete_provider(&self, id: i64) -> CoreResult<ProviderList> {
        self.ensure_running()?;
        let _write = self.providers.write.lock().await;
        let (mut set, info, file_broken) = {
            let state = self.providers.read();
            let info = state.infos.iter().find(|p| p.id == id).cloned();
            (state.set.clone(), info, state.file_broken)
        };
        let info = info.ok_or(CoreError::NotFound { what: "provider" })?;
        if info.source == ProviderSource::Env {
            return Err(CoreError::ReadOnly(
                "the environment profile is read-only; change the environment or .env instead",
            ));
        }
        if file_broken {
            return Err(CoreError::Conflict(
                "providers.toml cannot be read, so it is not overwritten; fix or remove the file first"
                    .to_owned(),
            ));
        }
        set.remove(&info.name)
            .map_err(|error| profile_error(&error))?;
        self.persist_providers(set.clone()).await?;
        self.apply_provider_set(set).await?;
        Ok(self.providers.list())
    }

    /// Makes one profile the active one.
    pub async fn activate_provider(&self, id: i64) -> CoreResult<ProviderInfo> {
        self.ensure_running()?;
        let _write = self.providers.write.lock().await;
        let (set, exists) = {
            let state = self.providers.read();
            (state.set.clone(), state.infos.iter().any(|p| p.id == id))
        };
        if !exists {
            return Err(CoreError::NotFound { what: "provider" });
        }
        self.db.providers().set_active(id).await?;
        self.apply_provider_set(set).await?;
        self.providers
            .read()
            .infos
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or(CoreError::NotFound { what: "provider" })
    }

    /// Runs the capability probe against one profile (any profile, not only the
    /// active one) and stores what it found. The test uses its own HTTP client,
    /// which allows only that profile's host, so testing a profile never changes
    /// where the running tutor may connect.
    ///
    /// Dropping the returned future, for example when the browser gives up on
    /// the request, cancels the probe. A second test while one runs is refused
    /// with `Busy`.
    pub async fn test_provider(&self, id: i64) -> CoreResult<ProbeReport> {
        self.ensure_running()?;
        let _gate = self
            .providers
            .probe_gate
            .try_lock()
            .map_err(|_| CoreError::Busy)?;
        let (profile, name) = {
            let state = self.providers.read();
            let info = state
                .infos
                .iter()
                .find(|p| p.id == id)
                .ok_or(CoreError::NotFound { what: "provider" })?;
            let profile = state
                .set
                .get(&info.name)
                .cloned()
                .ok_or(CoreError::NotFound { what: "provider" })?;
            (profile, info.name.clone())
        };

        let cancel = self.shutdown.child_token();
        let _stop_on_drop = cancel.clone().drop_guard();
        let started = Instant::now();
        let outcome = match ProviderClient::connect_logged(
            &profile,
            ClientOptions::default(),
            None,
            self.payload_log.clone(),
        ) {
            Ok(client) => client.probe(&cancel).await,
            Err(error) => Err(error),
        };
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        match outcome {
            Ok(capabilities) => {
                let json = serde_json::to_value(&capabilities).map_err(|_| {
                    CoreError::Internal("the probe result could not be stored".to_owned())
                })?;
                // The row may have been replaced while the test ran; then the
                // result belongs to a profile that no longer exists.
                match self
                    .db
                    .providers()
                    .record_probe(id, &json, &self.clock().now())
                    .await
                {
                    Ok(()) => {}
                    Err(storage::StorageError::NotFound { .. }) => {
                        tracing::info!(
                            "a provider changed during its test; the result was dropped"
                        );
                    }
                    Err(other) => return Err(other.into()),
                }
                let set = self.providers.read().set.clone();
                self.apply_provider_set(set).await?;
                tracing::info!(provider = %name, "provider test finished");
                Ok(ProbeReport {
                    provider_id: id,
                    ok: true,
                    capabilities: Some(capabilities_view(&capabilities)),
                    failure: None,
                    duration_ms,
                })
            }
            Err(error) => Ok(ProbeReport {
                provider_id: id,
                ok: false,
                capabilities: None,
                failure: Some(failure_from(&error)),
                duration_ms,
            }),
        }
    }

    /// The client for the active provider, with the capabilities of its last
    /// test. The tutor engine calls this for every session. It fails with
    /// `ProviderNotConfigured` when there is no active profile.
    pub async fn llm_client(&self) -> CoreResult<Arc<dyn LlmClient>> {
        if let Some(client) = self.providers.read().client.clone() {
            return Ok(client);
        }
        let _write = self.providers.write.lock().await;
        let (profile, active_id) = {
            let state = self.providers.read();
            let Some(active) = state.infos.iter().find(|p| p.is_active) else {
                return Err(CoreError::ProviderNotConfigured);
            };
            let profile = state
                .set
                .get(&active.name)
                .cloned()
                .ok_or(CoreError::ProviderNotConfigured)?;
            (profile, active.id)
        };
        let stored = self
            .db
            .providers()
            .get(active_id)
            .await?
            .as_ref()
            .and_then(stored_capabilities);
        let client = Arc::new(
            ProviderClient::connect_logged(
                &profile,
                ClientOptions::default(),
                stored,
                self.payload_log.clone(),
            )
            .map_err(|error| CoreError::Conflict(error.to_string()))?,
        );
        self.providers.write_state().client = Some(Arc::clone(&client));
        Ok(client)
    }

    /// Writes `providers.toml` off the async runtime.
    async fn persist_providers(&self, set: ProfileSet) -> CoreResult<()> {
        let path = self.config.providers_path();
        tokio::task::spawn_blocking(move || set.save(&path))
            .await
            .map_err(|_| {
                CoreError::Internal("saving the providers file did not finish".to_owned())
            })?
            // `ProfileError` names an io failure by its kind only, never a path or a key.
            .map_err(|error| CoreError::Internal(error.to_string()))
    }

    /// Adopts `set` as the current profiles: reconciles the table, rebuilds the
    /// views, drops the cached client and tells the stream.
    async fn apply_provider_set(&self, set: ProfileSet) -> CoreResult<()> {
        let infos = reconcile(&self.db, &set, &self.clock().now()).await?;
        let active = {
            let mut state = self.providers.write_state();
            state.set = set;
            state.infos = infos;
            state.client = None;
            state.infos.iter().find(|p| p.is_active).cloned()
        };
        self.bus.publish(|seq| ServerEvent::ProviderStatus {
            seq,
            provider: active,
        });
        Ok(())
    }
}
