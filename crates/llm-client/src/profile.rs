//! Provider profiles and where their keys come from (`context_pack.md` section 10).
//!
//! Source one is the process environment or a `.env` file; the four variables form
//! a read-only profile named `env`. Source two is `providers.toml` in the app data
//! directory, written by the settings screen. A key lives in memory in `ApiKey`; the
//! only types that leave this crate for the UI are `ProfileInfo` (with `has_key` and
//! the last four characters) and plain strings.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::http::is_loopback_url;
use crate::key::{ApiKey, KeyError};
use crate::redact::sanitize_message;

pub const ENV_PROTOCOL: &str = "TUTOR_LLM_PROTOCOL";
pub const ENV_BASE_URL: &str = "TUTOR_LLM_BASE_URL";
pub const ENV_MODEL: &str = "TUTOR_LLM_MODEL";
pub const ENV_API_KEY: &str = "TUTOR_LLM_API_KEY";
/// The name of the read-only profile built from the environment.
pub const ENV_PROFILE_NAME: &str = "env";

const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    // `snake_case` would give `open_ai_chat`; the wire name is fixed by the spec.
    #[serde(rename = "openai_chat")]
    OpenAiChat,
    #[serde(rename = "anthropic_messages")]
    AnthropicMessages,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned for an unknown protocol name. Does not echo the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the protocol must be openai_chat or anthropic_messages")]
pub struct UnknownProtocol;

impl FromStr for Protocol {
    type Err = UnknownProtocol;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "openai_chat" => Ok(Self::OpenAiChat),
            "anthropic_messages" => Ok(Self::AnthropicMessages),
            _ => Err(UnknownProtocol),
        }
    }
}

/// Where a profile came from. `Env` profiles are read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    Env,
    File,
}

/// What the UI may know about a profile. There is no field for the key itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
    /// The last four characters of the key, when the key is long enough to show them safely.
    pub key_last4: Option<String>,
    pub source: ProfileSource,
}

/// Why a profile or a profile file was refused. No variant carries a key or a URL,
/// because both can hold one (`https://user:key@host`, `?key=...`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("the environment sets some of the provider variables but not {}", .missing.join(", "))]
    MissingVariables { missing: Vec<&'static str> },
    #[error(transparent)]
    UnknownProtocol(#[from] UnknownProtocol),
    #[error("the base URL is not usable: {0}")]
    InvalidBaseUrl(&'static str),
    #[error("the profile name must have 1 to 64 characters and no control characters")]
    InvalidName,
    #[error("the model name is empty")]
    EmptyModel,
    #[error("the profile name `env` is reserved for the read-only environment profile")]
    ReservedName,
    #[error("a profile with this name exists twice")]
    DuplicateName,
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error("could not {what} the providers file ({kind:?})")]
    Io {
        what: &'static str,
        kind: std::io::ErrorKind,
    },
    #[error("the providers file is not valid: {0}")]
    Toml(String),
}

/// A configured provider. `Debug` hides the key.
#[derive(Clone)]
pub struct ProviderProfile {
    pub name: String,
    pub protocol: Protocol,
    pub base_url: Url,
    pub model: String,
    pub key: Option<ApiKey>,
    pub source: ProfileSource,
}

impl fmt::Debug for ProviderProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderProfile")
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("host", &self.base_url.host_str())
            .field("model", &self.model)
            .field("has_key", &self.key.is_some())
            .field("source", &self.source)
            .finish()
    }
}

fn validate_base_url(raw: &str) -> Result<Url, ProfileError> {
    let url =
        Url::parse(raw.trim()).map_err(|_| ProfileError::InvalidBaseUrl("it is not a URL"))?;
    if url.host().is_none() {
        return Err(ProfileError::InvalidBaseUrl("it has no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProfileError::InvalidBaseUrl(
            "it must not contain a user name or password",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProfileError::InvalidBaseUrl(
            "it must not contain a query or a fragment",
        ));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback_url(&url) => Ok(url),
        "http" => Err(ProfileError::InvalidBaseUrl(
            "plain HTTP is only allowed for localhost",
        )),
        _ => Err(ProfileError::InvalidBaseUrl("it must start with https://")),
    }
}

impl ProviderProfile {
    /// Validates every field. `key` may be absent for a local server that needs none.
    pub fn new(
        name: &str,
        protocol: Protocol,
        base_url: &str,
        model: &str,
        key: Option<&str>,
        source: ProfileSource,
    ) -> Result<Self, ProfileError> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(ProfileError::InvalidName);
        }
        let model = model.trim();
        if model.is_empty() {
            return Err(ProfileError::EmptyModel);
        }
        let key = match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(raw) => Some(ApiKey::new(raw)?),
            None => None,
        };
        Ok(Self {
            name: name.to_owned(),
            protocol,
            base_url: validate_base_url(base_url)?,
            model: model.to_owned(),
            key,
            source,
        })
    }

    pub fn info(&self) -> ProfileInfo {
        ProfileInfo {
            name: self.name.clone(),
            protocol: self.protocol,
            base_url: self.base_url.as_str().to_owned(),
            model: self.model.clone(),
            has_key: self.key.is_some(),
            key_last4: self.key.as_ref().and_then(ApiKey::last4),
            source: self.source,
        }
    }
}

/// Parses the `KEY=value` lines of a `.env` file. Supports comments, an optional
/// `export `, and single or double quotes. Nothing is expanded or unescaped.
pub fn parse_dotenv(text: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim();
        let value = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => match value[1..].find(quote) {
                Some(end) => &value[1..=end],
                None => &value[1..],
            },
            // An unquoted value ends at a ` #` comment.
            _ => value.split(" #").next().unwrap_or(value).trim_end(),
        };
        values.insert(key.to_owned(), value.to_owned());
    }
    values
}

/// Reads the `env` profile from the process environment and `.env` files.
#[derive(Debug, Clone, Default)]
pub struct EnvProfileLoader {
    dotenv_paths: Vec<PathBuf>,
}

impl EnvProfileLoader {
    pub fn new(dotenv_paths: Vec<PathBuf>) -> Self {
        Self { dotenv_paths }
    }

    /// `.env` in the working directory, then `.env` next to the executable.
    pub fn with_default_paths() -> Self {
        let mut paths = Vec::new();
        if let Ok(dir) = std::env::current_dir() {
            paths.push(dir.join(".env"));
        }
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
        {
            let candidate = dir.join(".env");
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
        Self {
            dotenv_paths: paths,
        }
    }

    /// Loads the profile using the real process environment.
    pub fn load(&self) -> Result<Option<ProviderProfile>, ProfileError> {
        self.load_with(|name| std::env::var(name).ok())
    }

    /// `process_env` stands in for the process environment so tests need not change it.
    /// Precedence per variable: process environment, then each `.env` file in order.
    /// Empty values count as unset. When none of the four variables is set the result
    /// is `Ok(None)`; when some are set, protocol, base URL and model are required and
    /// the key is optional.
    pub fn load_with(
        &self,
        process_env: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<ProviderProfile>, ProfileError> {
        let files: Vec<HashMap<String, String>> = self
            .dotenv_paths
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse_dotenv(&text))
            .collect();
        let lookup = |name: &str| -> Option<String> {
            process_env(name)
                .into_iter()
                .chain(files.iter().filter_map(|file| file.get(name).cloned()))
                .map(|v| v.trim().to_owned())
                .find(|v| !v.is_empty())
        };

        let protocol = lookup(ENV_PROTOCOL);
        let base_url = lookup(ENV_BASE_URL);
        let model = lookup(ENV_MODEL);
        let key = lookup(ENV_API_KEY);
        if protocol.is_none() && base_url.is_none() && model.is_none() && key.is_none() {
            return Ok(None);
        }
        let missing: Vec<&'static str> = [
            (ENV_PROTOCOL, &protocol),
            (ENV_BASE_URL, &base_url),
            (ENV_MODEL, &model),
        ]
        .into_iter()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name)
        .collect();
        let (Some(protocol), Some(base_url), Some(model)) = (protocol, base_url, model) else {
            return Err(ProfileError::MissingVariables { missing });
        };
        ProviderProfile::new(
            ENV_PROFILE_NAME,
            protocol.parse()?,
            &base_url,
            &model,
            key.as_deref(),
            ProfileSource::Env,
        )
        .map(Some)
    }
}

/// One `[[profile]]` table of `providers.toml`. The key is written in plain text, as
/// the settings screen promises; the file gets user-only permissions where the OS allows.
#[derive(Serialize, Deserialize)]
struct FileProfile {
    name: String,
    protocol: Protocol,
    base_url: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct ProfileFile {
    #[serde(default, rename = "profile")]
    profiles: Vec<FileProfile>,
}

/// The `env` profile plus the profiles saved in `providers.toml`.
#[derive(Debug, Clone, Default)]
pub struct ProfileSet {
    env: Option<ProviderProfile>,
    file: Vec<ProviderProfile>,
}

impl ProfileSet {
    /// `providers_path` may not exist yet; that is an empty set of saved profiles.
    pub fn load(env: Option<ProviderProfile>, providers_path: &Path) -> Result<Self, ProfileError> {
        let text = match std::fs::read_to_string(providers_path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(ProfileError::Io {
                    what: "read",
                    kind: error.kind(),
                });
            }
        };
        // The parser's message can quote the offending value, so it goes through the
        // same redaction as provider error text. The source snippet is never used.
        let parsed: ProfileFile = toml::from_str(&text)
            .map_err(|error| ProfileError::Toml(sanitize_message(error.message(), None)))?;
        let mut file: Vec<ProviderProfile> = Vec::with_capacity(parsed.profiles.len());
        for entry in parsed.profiles {
            if entry.name.trim() == ENV_PROFILE_NAME {
                return Err(ProfileError::ReservedName);
            }
            let profile = ProviderProfile::new(
                &entry.name,
                entry.protocol,
                &entry.base_url,
                &entry.model,
                entry.api_key.as_deref(),
                ProfileSource::File,
            )?;
            if file.iter().any(|p| p.name == profile.name) {
                return Err(ProfileError::DuplicateName);
            }
            file.push(profile);
        }
        Ok(Self { env, file })
    }

    /// The environment profile first, then the saved ones.
    pub fn list(&self) -> Vec<ProfileInfo> {
        self.env
            .iter()
            .chain(self.file.iter())
            .map(ProviderProfile::info)
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&ProviderProfile> {
        self.env
            .iter()
            .chain(self.file.iter())
            .find(|p| p.name == name)
    }

    /// Adds or replaces a saved profile. The `env` profile cannot be replaced.
    pub fn upsert(&mut self, mut profile: ProviderProfile) -> Result<(), ProfileError> {
        if profile.name == ENV_PROFILE_NAME {
            return Err(ProfileError::ReservedName);
        }
        profile.source = ProfileSource::File;
        match self.file.iter_mut().find(|p| p.name == profile.name) {
            Some(existing) => *existing = profile,
            None => self.file.push(profile),
        }
        Ok(())
    }

    /// Removes a saved profile and says whether it existed. The `env` profile cannot be removed.
    pub fn remove(&mut self, name: &str) -> Result<bool, ProfileError> {
        if name == ENV_PROFILE_NAME {
            return Err(ProfileError::ReservedName);
        }
        let before = self.file.len();
        self.file.retain(|p| p.name != name);
        Ok(self.file.len() != before)
    }

    /// Writes the saved profiles (not the `env` one) to `providers_path`.
    pub fn save(&self, providers_path: &Path) -> Result<(), ProfileError> {
        let document = ProfileFile {
            profiles: self
                .file
                .iter()
                .map(|p| FileProfile {
                    name: p.name.clone(),
                    protocol: p.protocol,
                    base_url: p.base_url.as_str().to_owned(),
                    model: p.model.clone(),
                    api_key: p.key.as_ref().map(|k| k.expose().to_owned()),
                })
                .collect(),
        };
        let text = toml::to_string(&document)
            .map_err(|_| ProfileError::Toml("the profiles could not be written".to_owned()))?;
        write_private(providers_path, text.as_bytes()).map_err(|error| ProfileError::Io {
            what: "write",
            kind: error.kind(),
        })
    }
}

/// Writes through a temporary file in the same directory and renames it, so a crash
/// never leaves a half-written file. On Unix the file is created with mode 0600. On
/// Windows no permission call is made: the file inherits the access rules of the
/// per-user app data directory it is written into.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    // A leftover from a crashed write may have been created with looser permissions.
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests;
