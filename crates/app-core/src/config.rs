//! What the core needs to know before it starts: where files live and where
//! "now" and the environment come from.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use llm_client::EnvProfileLoader;

use crate::api::HardwareProfile;
use crate::clock::{Clock, SystemClock};

/// Name of the database file inside the data directory.
pub const DATABASE_FILE: &str = "lumingo.sqlite";
/// Name of the plain-text provider file inside the data directory.
pub const PROVIDERS_FILE: &str = "providers.toml";

/// Reads one process environment variable. A function instead of
/// `std::env::var` so tests never touch the real environment.
pub type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Settings for [`crate::AppCore::open`].
#[derive(Clone)]
pub struct CoreConfig {
    /// The app data directory: database, `providers.toml`, recordings.
    pub data_dir: PathBuf,
    /// The folder of unit files that ship with the program.
    pub curriculum_dir: PathBuf,
    /// The folder of catalogs that ship with the program: `rubrics/` and
    /// `topics.json`. Without them, productive activities are stored unscored and
    /// topic-bank picks are refused; typed topics still work.
    pub catalogs_dir: PathBuf,
    /// `models/manifest.toml`, the only source of model download locations. When
    /// it cannot be read, model downloads are not available.
    pub models_manifest: PathBuf,
    /// A word list that gives each word a level, for the vocabulary checks of
    /// generated texts. Optional: without it those checks are skipped and say so.
    pub word_list: Option<PathBuf>,
    /// Development mode of the server, reported in the snapshot.
    pub dev_mode: bool,
    /// The address the server listens on, for the diagnostics page. The core
    /// does not bind anything; the server tells it.
    pub server_address: Option<String>,
    /// Where `.env` files are looked for.
    pub env_profiles: EnvProfileLoader,
    /// The process environment the `env` provider profile is read from.
    pub env_lookup: EnvLookup,
    pub clock: Arc<dyn Clock>,
    /// A measured profile to use instead of measuring. Tests only; the
    /// executable leaves it `None`.
    pub hardware: Option<HardwareProfile>,
}

impl CoreConfig {
    /// The real clock, the real environment and the default `.env` locations.
    pub fn new(data_dir: PathBuf, curriculum_dir: PathBuf) -> Self {
        Self {
            data_dir,
            catalogs_dir: curriculum_dir
                .parent()
                .map_or_else(|| PathBuf::from("catalogs"), |dir| dir.join("catalogs")),
            curriculum_dir,
            models_manifest: default_models_manifest(),
            word_list: None,
            dev_mode: false,
            server_address: None,
            env_profiles: EnvProfileLoader::with_default_paths(),
            env_lookup: Arc::new(|name| std::env::var(name).ok()),
            clock: Arc::new(SystemClock),
            hardware: None,
        }
    }

    /// The SQLite file.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(DATABASE_FILE)
    }

    /// The file the settings screen saves provider profiles to.
    pub fn providers_path(&self) -> PathBuf {
        self.data_dir.join(PROVIDERS_FILE)
    }

    /// The folder models are installed under, one subfolder per model.
    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }
}

impl fmt::Debug for CoreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreConfig")
            .field("data_dir", &self.data_dir)
            .field("curriculum_dir", &self.curriculum_dir)
            .field("catalogs_dir", &self.catalogs_dir)
            .field("models_manifest", &self.models_manifest)
            .field("dev_mode", &self.dev_mode)
            .field("server_address", &self.server_address)
            .finish_non_exhaustive()
    }
}

/// The operating systems whose per-user data folder differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Windows,
    MacOs,
    Other,
}

impl Platform {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

fn data_dir_for(platform: Platform, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let non_empty = |name: &str| var(name).filter(|value| !value.is_empty());
    match platform {
        Platform::Windows => {
            non_empty("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Lumingo"))
        }
        Platform::MacOs => non_empty("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Lumingo")),
        Platform::Other => non_empty("XDG_DATA_HOME")
            .map(|dir| PathBuf::from(dir).join("lumingo"))
            .or_else(|| {
                non_empty("HOME").map(|home| PathBuf::from(home).join(".local/share/lumingo"))
            }),
    }
}

/// The per-user data folder: `%LOCALAPPDATA%\Lumingo` on Windows. `None` when
/// the environment names no home, in which case the caller asks for `--data-dir`.
pub fn default_data_dir() -> Option<PathBuf> {
    data_dir_for(Platform::current(), |name| std::env::var_os(name))
}

/// The unit folder that ships with the program: `curriculum/units` next to the
/// executable when it exists, otherwise the one in the working directory (a
/// source checkout).
pub fn default_curriculum_dir() -> PathBuf {
    let next_to_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("curriculum").join("units")));
    match next_to_exe {
        Some(dir) if Path::new(&dir).is_dir() => dir,
        _ => PathBuf::from("curriculum").join("units"),
    }
}

/// The model manifest that ships with the program: `models/manifest.toml` next
/// to the executable when it exists, otherwise the one in the working directory
/// (a source checkout).
pub fn default_models_manifest() -> PathBuf {
    let next_to_exe = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join("models").join("manifest.toml"))
    });
    match next_to_exe {
        Some(file) if file.is_file() => file,
        _ => PathBuf::from("models").join("manifest.toml"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn each_platform_uses_its_own_per_user_folder() {
        let windows = data_dir_for(
            Platform::Windows,
            env(&[("LOCALAPPDATA", "C:/Users/a/AppData/Local")]),
        );
        assert_eq!(
            windows,
            Some(PathBuf::from("C:/Users/a/AppData/Local/Lumingo"))
        );

        let mac = data_dir_for(Platform::MacOs, env(&[("HOME", "/Users/a")]));
        assert_eq!(
            mac,
            Some(PathBuf::from(
                "/Users/a/Library/Application Support/Lumingo"
            ))
        );

        let xdg = data_dir_for(
            Platform::Other,
            env(&[("XDG_DATA_HOME", "/x"), ("HOME", "/h")]),
        );
        assert_eq!(xdg, Some(PathBuf::from("/x/lumingo")));
        let home = data_dir_for(Platform::Other, env(&[("HOME", "/h")]));
        assert_eq!(home, Some(PathBuf::from("/h/.local/share/lumingo")));
    }

    #[test]
    fn an_empty_or_missing_variable_gives_no_folder() {
        assert_eq!(data_dir_for(Platform::Windows, env(&[])), None);
        assert_eq!(
            data_dir_for(Platform::Windows, env(&[("LOCALAPPDATA", "")])),
            None
        );
        assert_eq!(data_dir_for(Platform::Other, env(&[("HOME", "")])), None);
    }
}
