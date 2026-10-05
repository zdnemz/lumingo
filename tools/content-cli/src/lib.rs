//! `content-cli`: validates curriculum units and catalogs.
//!
//! All rules live in the `curriculum` crate. This crate reads files, decides what each file
//! is, hands the parsed documents to the rules and renders the reports.

#![forbid(unsafe_code)]

pub mod discover;
pub mod render;

use std::path::PathBuf;

use curriculum::validate::{FileKind, SetReport};

/// How to print the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
}

/// Everything `validate` was asked to do.
#[derive(Debug, Clone)]
pub struct ValidateOptions {
    /// Files or folders. A folder is searched for `.json` files, recursively.
    pub paths: Vec<PathBuf>,
    /// Treat every file as this kind instead of guessing from its location.
    pub kind: Option<FileKind>,
    /// Require a finished curriculum: strict X01, X05 and the K03 coverage rules.
    pub complete: bool,
    /// A word list for W01 and W02.
    pub word_list: Option<PathBuf>,
    /// The `curriculum` folder, for syllabi and rubrics. Found from the first path when absent.
    pub curriculum_root: Option<PathBuf>,
}

/// The result of a run: the report and how many files it looked at.
#[derive(Debug)]
pub struct Outcome {
    pub report: SetReport,
    pub files_examined: usize,
    /// The curriculum folder used for syllabi and rubrics, when one was found.
    pub curriculum_root: Option<PathBuf>,
}

impl Outcome {
    /// The process exit code: 0 only when there are zero errors, 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        i32::from(!self.report.passed())
    }
}
