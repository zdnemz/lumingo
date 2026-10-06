//! The real audio devices, through cpal. UNVERIFIED: compile-checked for the
//! Windows target through `audio-io`, which owns the cpal code, and never run
//! against a device.

use std::path::Path;
use std::sync::Arc;

use audio_io::{CpalBackend, DeviceRegistry, FilePrefs, MemoryPrefs, PrefsStore};

/// A registry over the machine's devices. The learner's pick is remembered in
/// `audio-devices.json` inside `dir`, or not at all when there is no folder.
pub fn registry(dir: Option<&Path>) -> Arc<DeviceRegistry> {
    let prefs: Box<dyn PrefsStore> = match dir {
        Some(dir) => Box::new(FilePrefs::new(dir.join("audio-devices.json"))),
        None => Box::new(MemoryPrefs::default()),
    };
    Arc::new(DeviceRegistry::new(Arc::new(CpalBackend::new()), prefs))
}
