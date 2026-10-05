//! Devices: what a backend offers, what the learner picked, and what to do when
//! the pick is gone.
//!
//! [`AudioBackend`] is the seam between this crate and an audio API. The real one
//! is `cpal` behind the `cpal-backend` feature; tests use a fake. Everything above
//! the seam (the registry here, the session in [`crate::session`]) is hardware-free.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::format::StreamFormat;
use crate::sync::lock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,
    Output,
}

impl Direction {
    fn noun(self) -> &'static str {
        match self {
            Self::Input => "microphone",
            Self::Output => "speaker",
        }
    }
}

/// A backend's own identifier for a device. It is opaque: only the backend that
/// produced it can interpret it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

/// One device a backend offers, with the format its streams will use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    /// The name the learner sees.
    pub name: String,
    pub direction: Direction,
    pub is_default: bool,
    /// The device's default format. Streams are opened with exactly this.
    pub format: StreamFormat,
}

/// Why a device operation failed. These are typed so the UI can say what
/// happened and offer the right action.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    #[error("no {} is available", .0.noun())]
    NoDevice(Direction),
    #[error("the {} \"{name}\" is not available", .direction.noun())]
    NotFound { direction: Direction, name: String },
    /// The device disappeared while a stream was using it, for example an
    /// unplugged headset. Recovery reopens the stream on another device.
    #[error("the {} \"{name}\" was lost", .direction.noun())]
    Lost { direction: Direction, name: String },
    #[error("the {} \"{name}\" cannot be opened: {detail}", .direction.noun())]
    Unsupported {
        direction: Direction,
        name: String,
        detail: String,
    },
    #[error("the audio system reported an error: {0}")]
    Backend(String),
}

/// Called on a backend thread with each block of interleaved input samples.
pub type InputCallback = Box<dyn FnMut(&[f32]) + Send + 'static>;
/// Called on a backend thread to fill one block of interleaved output samples.
pub type OutputCallback = Box<dyn FnMut(&mut [f32]) + Send + 'static>;
/// Called when a running stream fails, including when its device is lost. It may
/// run on any thread and must not block.
pub type StreamErrorCallback = Arc<dyn Fn(DeviceError) + Send + Sync + 'static>;

/// A running stream. Dropping it stops the stream and ends every thread the
/// backend started for it, before `drop` returns.
pub trait AudioStream: Send {}

pub trait AudioBackend: Send + Sync {
    fn devices(&self, direction: Direction) -> Result<Vec<DeviceInfo>, DeviceError>;

    fn open_input(
        &self,
        device: &DeviceInfo,
        callback: InputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError>;

    fn open_output(
        &self,
        device: &DeviceInfo,
        callback: OutputCallback,
        on_error: StreamErrorCallback,
    ) -> Result<Box<dyn AudioStream>, DeviceError>;
}

/// A remembered choice. The name is kept next to the id because ids can change
/// when a device is plugged into another port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRef {
    pub id: DeviceId,
    pub name: String,
}

/// What the learner chose, per direction. `None` means "use the default".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DevicePrefs {
    pub input: Option<DeviceRef>,
    pub output: Option<DeviceRef>,
}

impl DevicePrefs {
    fn slot(&mut self, direction: Direction) -> &mut Option<DeviceRef> {
        match direction {
            Direction::Input => &mut self.input,
            Direction::Output => &mut self.output,
        }
    }

    fn get(&self, direction: Direction) -> Option<&DeviceRef> {
        match direction {
            Direction::Input => self.input.as_ref(),
            Direction::Output => self.output.as_ref(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrefsError {
    #[error("could not read or write the device preferences: {0}")]
    Io(#[from] std::io::Error),
    #[error("the device preferences are not valid: {0}")]
    Format(#[from] serde_json::Error),
}

/// Where device choices are remembered. `app-core` can implement this over its
/// settings storage; [`FilePrefs`] is the stand-alone version.
pub trait PrefsStore: Send {
    /// A store with nothing saved yet returns the default preferences.
    fn load(&self) -> Result<DevicePrefs, PrefsError>;
    fn save(&self, prefs: &DevicePrefs) -> Result<(), PrefsError>;
}

/// Keeps preferences in memory. For tests and for runs that should not remember.
#[derive(Debug, Clone, Default)]
pub struct MemoryPrefs(Arc<Mutex<DevicePrefs>>);

impl PrefsStore for MemoryPrefs {
    fn load(&self) -> Result<DevicePrefs, PrefsError> {
        Ok(lock(&self.0).clone())
    }

    fn save(&self, prefs: &DevicePrefs) -> Result<(), PrefsError> {
        *lock(&self.0) = prefs.clone();
        Ok(())
    }
}

/// Keeps preferences in one small JSON file. The file holds device names and ids
/// only. A save writes a temporary file and renames it, so a crash cannot leave
/// half a file behind.
#[derive(Debug, Clone)]
pub struct FilePrefs {
    path: PathBuf,
}

impl FilePrefs {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl PrefsStore for FilePrefs {
    fn load(&self) -> Result<DevicePrefs, PrefsError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DevicePrefs::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, prefs: &DevicePrefs) -> Result<(), PrefsError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut temp = self.path.clone().into_os_string();
        temp.push(".tmp");
        let temp = PathBuf::from(temp);
        std::fs::write(&temp, serde_json::to_vec_pretty(prefs)?)?;
        std::fs::rename(&temp, &self.path)?;
        Ok(())
    }
}

/// Why a device was chosen for a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The learner's remembered pick.
    Selected,
    /// Nothing was picked, so the system default is used.
    Default,
    /// The remembered pick is not available now, so the default is used. The
    /// preference is kept for when the device comes back.
    DefaultInstead { missing: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDevice {
    pub info: DeviceInfo,
    pub choice: Choice,
}

/// Lists devices, remembers the learner's pick and resolves it to a device that
/// exists right now.
pub struct DeviceRegistry {
    backend: Arc<dyn AudioBackend>,
    store: Box<dyn PrefsStore>,
    prefs: Mutex<DevicePrefs>,
}

impl std::fmt::Debug for DeviceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceRegistry").finish_non_exhaustive()
    }
}

impl DeviceRegistry {
    /// Loads the remembered choices. A damaged preferences file is logged and
    /// treated as empty: losing a remembered device must not stop the app.
    pub fn new(backend: Arc<dyn AudioBackend>, store: Box<dyn PrefsStore>) -> Self {
        let prefs = store.load().unwrap_or_else(|error| {
            tracing::warn!(%error, "ignoring unreadable device preferences");
            DevicePrefs::default()
        });
        Self {
            backend,
            store,
            prefs: Mutex::new(prefs),
        }
    }

    pub fn backend(&self) -> &Arc<dyn AudioBackend> {
        &self.backend
    }

    pub fn list(&self, direction: Direction) -> Result<Vec<DeviceInfo>, DeviceError> {
        self.backend.devices(direction)
    }

    /// The remembered choice, whether or not the device is present.
    pub fn remembered(&self, direction: Direction) -> Option<DeviceRef> {
        lock(&self.prefs).get(direction).cloned()
    }

    /// Picks a device and remembers it. `None` goes back to the system default.
    /// A device that is not in the current list is refused.
    pub fn select(
        &self,
        direction: Direction,
        device: Option<&DeviceId>,
    ) -> Result<(), DeviceError> {
        let chosen = match device {
            None => None,
            Some(id) => {
                let found = self
                    .backend
                    .devices(direction)?
                    .into_iter()
                    .find(|d| &d.id == id)
                    .ok_or_else(|| DeviceError::NotFound {
                        direction,
                        name: id.0.clone(),
                    })?;
                Some(DeviceRef {
                    id: found.id,
                    name: found.name,
                })
            }
        };
        let mut prefs = lock(&self.prefs);
        let mut updated = prefs.clone();
        *updated.slot(direction) = chosen;
        self.store
            .save(&updated)
            .map_err(|e| DeviceError::Backend(e.to_string()))?;
        *prefs = updated;
        Ok(())
    }

    /// The device a new stream should use right now.
    pub fn resolve(&self, direction: Direction) -> Result<ResolvedDevice, DeviceError> {
        let devices = self.backend.devices(direction)?;
        let remembered = self.remembered(direction);
        if let Some(wanted) = &remembered {
            let by_id = devices.iter().find(|d| d.id == wanted.id);
            let by_name = || {
                let mut same_name = devices.iter().filter(|d| d.name == wanted.name);
                match (same_name.next(), same_name.next()) {
                    (Some(only), None) => Some(only),
                    _ => None,
                }
            };
            if let Some(found) = by_id.or_else(by_name) {
                return Ok(ResolvedDevice {
                    info: found.clone(),
                    choice: Choice::Selected,
                });
            }
        }
        let default = devices
            .iter()
            .find(|d| d.is_default)
            .or_else(|| devices.first())
            .ok_or(DeviceError::NoDevice(direction))?;
        Ok(ResolvedDevice {
            info: default.clone(),
            choice: match remembered {
                None => Choice::Default,
                Some(missing) => Choice::DefaultInstead {
                    missing: missing.name,
                },
            },
        })
    }
}

#[cfg(test)]
mod tests;
