//! The real audio devices, through cpal. UNVERIFIED: this has been
//! compile-checked for the Windows target (through `audio-io`, which owns the
//! cpal code) and never run against a device.
//!
//! It depends on `audio-io` alone, so it can be checked for Windows without the
//! database and the rest of the program.

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
