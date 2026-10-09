//! Choosing and connecting the language model provider.
//!
//! The same sources as the server: the `TUTOR_LLM_*` environment variables or a
//! `.env` file make the read-only profile `env`, and `providers.toml` in the data
//! directory holds the saved ones. A key is held in memory by `llm-client` and
//! never printed: this module prints the host and the model, nothing else about
//! the profile.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use llm_client::{
    ClientOptions, EnvProfileLoader, Limits, LlmClient, ProfileSet, ProviderClient,
    ProviderProfile, RetryPolicy,
};
use tokio_util::sync::CancellationToken;

/// What the capability probe found, for the caller to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The probe ran and at least one structured-output ladder level worked.
    StructuredWorks,
    /// The probe ran but no structured-output ladder level worked: the
    /// streaming calls (T1) still work, the structured ones (T2, rubrics) will
    /// fail.
    NoStructuredLevel,
    /// The provider rejected the key. Nothing will work.
    KeyRejected,
    /// The probe could not finish: a transport failure, a timeout, a rate
    /// limit. The calls themselves may still work.
    Unavailable,
}

impl ProbeOutcome {
    /// Whether the run should go on. Only a rejected key stops it: a provider
    /// that fails the probe's non-streaming step can still stream, and a
    /// missing structured level only affects the structured calls.
    pub fn continues(self) -> bool {
        !matches!(self, Self::KeyRejected)
    }
}

/// A connected provider and what may be said about it.
pub struct Provider {
    client: Arc<ProviderClient>,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub model: String,
}

impl Provider {
    /// The client as the rest of the program takes it.
    pub fn client(&self) -> Arc<dyn LlmClient> {
        self.client.clone()
    }

    /// Runs the capability probe (the connection test) and prints what it found.
    ///
    /// Every live check that makes structured calls runs this first: the probe
    /// caches the ladder level the provider really supports, and without it the
    /// structured calls start at level 1 and fail on a gateway that silently
    /// ignores the native schema (the S4-06 quirk).
    /// `TUTOR_LLM_FORCE_LEVEL=1..4` forces a level after the probe, to compare
    /// what a provider does at each one.
    pub async fn probe(&self) -> ProbeOutcome {
        let caps = match self.client.probe(&CancellationToken::new()).await {
            Ok(caps) => caps,
            Err(error) => {
                eprintln!("probe failed: {error}");
                return match error {
                    llm_client::LlmError::Auth { .. } => ProbeOutcome::KeyRejected,
                    _ => ProbeOutcome::Unavailable,
                };
            }
        };
        println!(
            "probe: auth {}, stream {}, structured level {:?}, first token {:?} ms",
            caps.auth_ok,
            caps.stream_ok,
            caps.structured_level.map(llm_client::LadderLevel::as_u8),
            caps.ttft_ms
        );
        if let Ok(forced) = std::env::var("TUTOR_LLM_FORCE_LEVEL")
            && let Ok(number) = forced.parse::<u8>()
            && let Ok(level) = llm_client::LadderLevel::try_from(number)
        {
            self.client.set_structured_level(level);
            println!("forced structured level {}", level.as_u8());
        }
        if caps.structured_level.is_some() {
            ProbeOutcome::StructuredWorks
        } else {
            eprintln!(
                "warning: no structured-output ladder level worked in the probe; \
                 structured calls will fail"
            );
            ProbeOutcome::NoStructuredLevel
        }
    }
}

/// The limits of one tutor turn from the whole time the provider is given to
/// start a reply. The defaults of `context_pack.md` section 13 (connect 5 s,
/// first token 8 s) are what a budget of 16 s gives.
pub fn client_options(budget: Duration) -> ClientOptions {
    let half = budget / 2;
    let tutor = Limits {
        connect: half.min(Duration::from_secs(5)),
        first_token: Some(half),
        total: Limits::TUTOR_TURN.total.max(budget),
    };
    ClientOptions {
        tutor,
        background: Limits {
            connect: tutor.connect,
            first_token: None,
            total: Limits::BACKGROUND.total,
        },
        retry: RetryPolicy::DEFAULT,
    }
}

fn providers_path(data_dir: Option<&Path>, file: Option<&Path>) -> Option<PathBuf> {
    match (file, data_dir) {
        (Some(file), _) => Some(file.to_owned()),
        (None, Some(dir)) => Some(dir.join("providers.toml")),
        (None, None) => app_core::default_data_dir().map(|dir| dir.join("providers.toml")),
    }
}

/// The profile to use: the named one, the only one, or `env`.
pub fn pick<'a>(set: &'a ProfileSet, wanted: Option<&str>) -> Result<&'a ProviderProfile> {
    let names: Vec<String> = set.list().into_iter().map(|p| p.name).collect();
    let name = match wanted {
        Some(name) => name.to_owned(),
        None => match names.as_slice() {
            [only] => only.clone(),
            _ if names.iter().any(|n| n == llm_client::ENV_PROFILE_NAME) => {
                llm_client::ENV_PROFILE_NAME.to_owned()
            }
            [] => bail!(
                "no provider is configured: set {}, {}, {} and {} (or put them in a .env file), \
                 or save a profile in providers.toml",
                llm_client::ENV_PROTOCOL,
                llm_client::ENV_BASE_URL,
                llm_client::ENV_MODEL,
                llm_client::ENV_API_KEY
            ),
            _ => bail!(
                "several providers are configured ({}); pick one with --provider",
                names.join(", ")
            ),
        },
    };
    set.get(&name).with_context(|| {
        if names.is_empty() {
            format!("the provider profile `{name}` does not exist and none is configured")
        } else {
            format!(
                "the provider profile `{name}` does not exist; known profiles: {}",
                names.join(", ")
            )
        }
    })
}

/// Reads the profiles from `loader` and `providers.toml`, picks one and connects.
pub fn connect(
    loader: &EnvProfileLoader,
    data_dir: Option<&Path>,
    providers_file: Option<&Path>,
    wanted: Option<&str>,
    budget: Duration,
) -> Result<Provider> {
    let env = loader
        .load()
        .map_err(|error| anyhow::anyhow!("the environment profile is not usable: {error}"))?;
    let set = match providers_path(data_dir, providers_file) {
        Some(path) => ProfileSet::load(env, &path)
            .map_err(|error| anyhow::anyhow!("providers.toml is not usable: {error}"))?,
        None => ProfileSet::load(env, Path::new(".no-providers-file"))
            .map_err(|error| anyhow::anyhow!("the environment profile is not usable: {error}"))?,
    };
    let profile = pick(&set, wanted)?;
    let client = ProviderClient::connect(profile, client_options(budget), None)
        .map_err(|error| anyhow::anyhow!("cannot set up the provider client: {error}"))?;
    Ok(Provider {
        client: Arc::new(client),
        name: profile.name.clone(),
        protocol: profile.protocol.to_string(),
        host: profile.base_url.host_str().unwrap_or("?").to_owned(),
        model: profile.model.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_client::{ProfileSource, Protocol};

    fn profile(name: &str) -> ProviderProfile {
        ProviderProfile::new(
            name,
            Protocol::OpenAiChat,
            "https://api.example.test/v1",
            "m",
            Some("sk-test-0123456789abcdef"),
            ProfileSource::File,
        )
        .unwrap()
    }

    fn set_of(names: &[&str]) -> ProfileSet {
        let mut set = ProfileSet::default();
        for name in names {
            set.upsert(profile(name)).unwrap();
        }
        set
    }

    #[test]
    fn the_only_profile_is_used_and_a_name_picks_one_of_several() {
        assert_eq!(pick(&set_of(&["a"]), None).unwrap().name, "a");
        let two = set_of(&["a", "b"]);
        assert_eq!(pick(&two, Some("b")).unwrap().name, "b");
        let error = pick(&two, None).err().unwrap().to_string();
        assert!(
            error.contains("--provider") && error.contains("a, b"),
            "{error}"
        );
        let error = pick(&two, Some("zzz")).err().unwrap().to_string();
        assert!(error.contains("known profiles: a, b"), "{error}");
    }

    #[test]
    fn no_profile_says_what_to_set_and_never_a_key() {
        let error = pick(&ProfileSet::default(), None)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("TUTOR_LLM_BASE_URL"), "{error}");
        assert!(error.contains("providers.toml"), "{error}");
    }

    #[test]
    fn the_budget_sets_the_limits_and_16_seconds_gives_the_section_13_defaults() {
        let options = client_options(Duration::from_secs(16));
        assert_eq!(options.tutor.connect, Duration::from_secs(5));
        assert_eq!(options.tutor.first_token, Some(Duration::from_secs(8)));
        assert_eq!(options.tutor.total, Duration::from_secs(30));
        let short = client_options(Duration::from_millis(1_000));
        assert_eq!(short.tutor.connect, Duration::from_millis(500));
        assert_eq!(short.tutor.first_token, Some(Duration::from_millis(500)));
    }
}
