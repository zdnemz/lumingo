#![allow(clippy::unwrap_used)] // test code

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_content-cli"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn example_dir() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../curriculum/examples")
        .to_string_lossy()
        .into_owned()
}

fn example_json() -> String {
    fs::read_to_string(Path::new(&example_dir()).join("a1-u01.example.json")).unwrap()
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh directory under the system temp dir that is removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "lumingo-content-cli-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    /// Writes `a1-u01.json` holding `unit` and returns the directory path.
    fn with_unit(&self, unit: &str) -> String {
        fs::write(self.0.join("a1-u01.json"), unit).unwrap();
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_example_passes_with_exit_0() {
    let (code, out) = run(&["validate", &example_dir()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 unit(s), 0 error(s)"), "{out}");
    // The real checker is linked, so W03 runs: nothing may be reported as skipped.
    assert!(out.contains("0 skipped"), "{out}");
    assert!(!out.contains("W03"), "{out}");
}

#[test]
fn a_grammar_error_in_an_at_answer_is_a_w03_warning_from_the_real_checker() {
    let unit = example_json().replace(
        "Hello! I am Arif. I am from Makassar.",
        "Hello! I has Arif. I am from Makassar.",
    );
    let dir = TempDir::new();
    let (code, out) = run(&["validate", &dir.with_unit(&unit)]);
    assert_eq!(code, 0, "a warning must not fail the run: {out}");
    assert!(
        out.contains("warning W03 /activities/11/model_answers/1/text"),
        "{out}"
    );
    assert!(
        out.contains("Agreement"),
        "the finding names its kind: {out}"
    );
}

#[test]
fn a_complete_curriculum_is_demanded_with_a_flag_and_fails_with_exit_1() {
    let (code, out) = run(&["validate", &example_dir(), "--complete"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("X01"), "{out}");
}

#[test]
fn bad_usage_and_a_missing_folder_exit_with_2() {
    assert_eq!(run(&["nonsense"]).0, 2);
    assert_eq!(run(&["validate", "/definitely/not/here"]).0, 2);
}
