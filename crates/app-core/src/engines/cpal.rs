//! The real audio devices, through cpal. UNVERIFIED: this is the same code as
//! `tools/tutor-cli/src/cpal_devices.rs`, whose cpal part (in `audio-io`) was
//! compile-checked for Windows earlier. This copy was not compiled with the
//! feature on in the build container (no ALSA headers, no Windows linker tools),
//! and nothing has run against a device.

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
