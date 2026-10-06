#![allow(clippy::unwrap_used)] // test code

use std::{path::PathBuf, process::Command};

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

#[test]
fn the_example_passes_with_exit_0() {
    let (code, out) = run(&["validate", &example_dir()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 unit(s), 0 error(s)"), "{out}");
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
