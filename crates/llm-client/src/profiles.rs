//! `providers.toml` (context_pack.md section 10, "Keys", source two): profiles
//! created in the settings screen, saved in plain text in the app data directory
//! with user-only permissions where the OS allows. The key lives in memory in
//! `ApiKey` and is never returned by any summary.

use crate::{ApiKey, Protocol, ProviderConfig, policy};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Write, path::Path};

/// The profile built from `.env` or the environment is called this and cannot be saved over.
pub const ENV_PROFILE_NAME: &str = "env";

#[derive(Debug, thiserror::Error)]
pub enum ProfilesError {
    #[error("could not read or write the providers file: {0}")]
    Io(String),
    /// Deliberately carries no text from the file: the offending line may hold a key.
    #[error("the providers file is not valid TOML (near line {0})")]
    Syntax(usize),
    #[error("profile `{0}`: {1}")]
    Invalid(String, &'static str),
}

#[derive(Debug)]
pub struct Profile {
    pub name: String,
    pub config: ProviderConfig,
}

/// What the settings screen may see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSummary {
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
    pub key_hint: String,
}

impl Profile {
    pub fn summary(&self) -> ProfileSummary {
        ProfileSummary {
            name: self.name.clone(),
            protocol: self.config.protocol,
            base_url: self.config.base_url.clone(),
            model: self.config.model.clone(),
            has_key: !self.config.api_key.is_empty(),
            key_hint: self.config.api_key.last_four(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct RawProfile {
    name: String,
    protocol: String,
    base_url: String,
    model: String,
    api_key: String,
}

#[derive(Serialize, Deserialize, Default)]
struct RawFile {
    #[serde(default, rename = "profile")]
    profiles: Vec<RawProfile>,
}

fn check(profile: &Profile) -> Result<(), ProfilesError> {
    let invalid = |why| ProfilesError::Invalid(profile.name.clone(), why);
    if profile.name.trim().is_empty() {
        return Err(invalid("the name is empty"));
    }
    if profile.name == ENV_PROFILE_NAME {
        return Err(invalid("`env` is reserved for the environment profile"));
    }
    if profile.config.model.trim().is_empty() {
        return Err(invalid("the model is empty"));
    }
    policy::parse_base_url(&profile.config.base_url)
        .map_err(|_| invalid("the base URL must be https, or http on a loopback host"))?;
    Ok(())
}

/// Reads the profiles. A missing file is no profiles. Invalid entries fail the
/// whole read, so a half-understood file is never used.
pub fn read_profiles(path: &Path) -> Result<Vec<Profile>, ProfilesError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ProfilesError::Io(e.kind().to_string())),
    };
    let raw: RawFile = toml::from_str(&text).map_err(|e| {
        let line = e.span().map_or(1, |s| {
            text[..s.start.min(text.len())].matches('\n').count() + 1
        });
        ProfilesError::Syntax(line)
    })?;
    let mut seen = HashSet::new();
    raw.profiles
        .into_iter()
        .map(|r| {
            let protocol = Protocol::from_name(&r.protocol)
                .ok_or_else(|| ProfilesError::Invalid(r.name.clone(), "unknown protocol"))?;
            let profile = Profile {
                name: r.name,
                config: ProviderConfig {
                    protocol,
                    base_url: r.base_url,
                    model: r.model,
                    api_key: ApiKey::new(r.api_key),
                },
            };
            check(&profile)?;
            if !seen.insert(profile.name.clone()) {
                return Err(ProfilesError::Invalid(
                    profile.name,
                    "the name is used twice",
                ));
            }
            Ok(profile)
        })
        .collect()
}

/// Writes all profiles, replacing the file. The file is created with owner-only
/// permissions on Unix and written to a temporary name first, so a crash cannot
/// leave a half-written key file. Windows relies on the user profile directory's ACLs.
pub fn write_profiles(path: &Path, profiles: &[Profile]) -> Result<(), ProfilesError> {
    let mut seen = HashSet::new();
    for p in profiles {
        check(p)?;
        if !seen.insert(&p.name) {
            return Err(ProfilesError::Invalid(
                p.name.clone(),
                "the name is used twice",
            ));
        }
    }
    let raw = RawFile {
        profiles: profiles
            .iter()
            .map(|p| RawProfile {
                name: p.name.clone(),
                protocol: p.config.protocol.name().to_owned(),
                base_url: p.config.base_url.clone(),
                model: p.config.model.clone(),
                api_key: p.config.api_key.expose().to_owned(),
            })
            .collect(),
    };
    let text = toml::to_string_pretty(&raw)
        .map_err(|_| ProfilesError::Io("could not serialise".into()))?;
    let io = |e: std::io::Error| ProfilesError::Io(e.kind().to_string());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = path.with_extension("toml.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&tmp).map_err(io)?;
    file.write_all(text.as_bytes()).map_err(io)?;
    file.sync_all().map_err(io)?;
    fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("llm-profiles-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("providers.toml")
    }

    fn profile(name: &str, key: &str) -> Profile {
        Profile {
            name: name.into(),
            config: ProviderConfig {
                protocol: Protocol::AnthropicMessages,
                base_url: "https://api.example.com".into(),
                model: "some-model".into(),
                api_key: ApiKey::new(key),
            },
        }
    }

    #[test]
    fn profiles_round_trip_and_a_missing_file_is_empty() {
        let path = temp("round");
        assert!(read_profiles(&path).unwrap().is_empty());
        write_profiles(
            &path,
            &[
                profile("work", "fake-key-1111"),
                profile("home", "fake-key-2222"),
            ],
        )
        .unwrap();
        let back = read_profiles(&path).unwrap();
        assert_eq!(
            back.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["work", "home"]
        );
        assert_eq!(back[0].config.api_key.last_four(), "1111");
        assert_eq!(back[1].config.protocol, Protocol::AnthropicMessages);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only_and_no_temp_file_is_left() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp("perms");
        write_profiles(&path, &[profile("a", "fake-key-3333")]).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!path.with_extension("toml.tmp").exists());
        // Rewriting keeps the restriction.
        write_profiles(&path, &[profile("a", "fake-key-4444")]).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn the_summary_has_a_hint_and_never_the_key() {
        let s = profile("a", "fake-key-5555").summary();
        assert!(s.has_key && s.key_hint == "5555");
        assert!(!format!("{s:?}").contains("fake-key"));
        assert!(!format!("{:?}", profile("a", "fake-key-5555")).contains("fake-key"));
    }

    #[test]
    fn a_syntax_error_reports_a_line_and_never_the_file_text() {
        let path = temp("syntax");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "[[profile]]\nname = \"a\"\napi_key = fake-secret-key-6666 oops\n",
        )
        .unwrap();
        let err = read_profiles(&path).unwrap_err();
        assert!(matches!(err, ProfilesError::Syntax(3)), "{err:?}");
        assert!(!format!("{err} {err:?}").contains("fake-secret"));
    }

    #[test]
    fn invalid_entries_are_refused_on_read_and_on_write() {
        let path = temp("invalid");
        for bad in [
            profile("env", "k"),
            profile(" ", "k"),
            {
                let mut p = profile("x", "k");
                p.config.base_url = "http://api.example.com".into();
                p
            },
            {
                let mut p = profile("y", "k");
                p.config.model = String::new();
                p
            },
        ] {
            assert!(write_profiles(&path, &[bad]).is_err());
        }
        assert!(write_profiles(&path, &[profile("dup", "k"), profile("dup", "k")]).is_err());
        assert!(!path.exists(), "a refused write leaves no file behind");

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[[profile]]\nname=\"a\"\nprotocol=\"gemini\"\nbase_url=\"https://x.com\"\nmodel=\"m\"\napi_key=\"k\"\n").unwrap();
        assert!(matches!(
            read_profiles(&path),
            Err(ProfilesError::Invalid(_, "unknown protocol"))
        ));
    }
}
