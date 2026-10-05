#![allow(dead_code, clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn example_path() -> PathBuf {
    repo_root().join("curriculum/examples/a1-u01.example.json")
}

pub fn example_bytes() -> Vec<u8> {
    std::fs::read(example_path()).expect("the example unit is part of the repository")
}

pub fn example_value() -> Value {
    serde_json::from_slice(&example_bytes()).expect("the example unit is valid JSON")
}

/// A fresh empty directory under the cargo target tmp dir, unique per test name.
pub fn scratch_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}
