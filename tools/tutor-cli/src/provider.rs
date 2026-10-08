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

/// A connected provider and what may be said about it.
pub struct Provider {
    pub client: Arc<dyn LlmClient>,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub model: String,
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
