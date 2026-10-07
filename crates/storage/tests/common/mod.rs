//! Shared helpers for the integration tests.
//!
//! A tiny std-only temporary directory: the tests need a fresh directory per
//! case and the crate does not add a dev-dependency for that. The directory is
//! removed when the guard drops, so a failing assertion still cleans up.
#![allow(dead_code)] // not every test file uses every helper
#![allow(clippy::unwrap_used, clippy::expect_used)] // helpers, not #[test] bodies

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp dir that is deleted on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a unique directory. Panics if the temp dir is unusable, which
    /// in a test is the correct outcome.
    pub fn new() -> Self {
        let unique = format!(
            "lumingo-storage-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The conventional database file inside this directory.
    pub fn db_path(&self) -> PathBuf {
        self.path.join("lumingo.sqlite")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Names of the `<db>.bak-*` files in a directory, sorted.
pub fn backups(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".bak-"))
        .collect();
    found.sort();
    found
}
