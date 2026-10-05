#![allow(dead_code, clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn example_value() -> Value {
    let path = repo_root().join("curriculum/examples/a1-u01.example.json");
    serde_json::from_slice(&std::fs::read(path).expect("example exists")).expect("example is JSON")
}

/// A fresh empty directory under the cargo target tmp dir, unique per test name.
pub fn scratch_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

pub fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, serde_json::to_vec_pretty(value).expect("serialise")).expect("write");
}

pub fn content_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_content-cli"))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("run content-cli")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub fn json_report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is a JSON report")
}
