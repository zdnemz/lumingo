//! Rule-based grammar findings through `harper-core` (assessment spec 5.3, X5;
//! the writing workshop's first layer; content warning W03).
//!
//! One concrete checker, behind the `grammar` feature (default on). It never
//! reaches the network, and it decides nothing: the callers (the rubric scorer,
//! the workshop, the content validators) own the interpretation. A build without
//! the feature has no checker at all, and its callers say "not checked" rather
//! than "clean" — that rule lives at each seam, not here.
//!
//! Spelling findings can be left out per call: a voice response is a transcript,
//! and its spelling is the recogniser's, not the learner's. The content tools
//! leave spelling out on purpose (units are full of names the dictionary does
//! not know); the workshop keeps it in (a typed draft is the learner's own
//! spelling). That split is deliberate — do not "unify" it.

use std::sync::Mutex;

use harper_core::linting::{LintGroup, LintKind, Linter};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};

/// One rule-based finding. Offsets count characters, not bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub start: usize,
    pub end: usize,
    /// The kind of the lint, as harper names it ("Agreement", "Spelling", ...).
    pub kind: String,
    pub message: String,
    pub suggestions: Vec<String>,
}

/// The `harper-core` checker. Building it loads a dictionary, so create one and
/// reuse it. `findings` takes `&self` and locks internally, so one checker can
/// sit behind an `Arc` and be shared across threads; a caller that only has one
/// thread can hold it directly.
pub struct GrammarChecker {
    linter: Mutex<LintGroup>,
}

impl Default for GrammarChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl GrammarChecker {
    pub fn new() -> Self {
        Self {
            linter: Mutex::new(LintGroup::new_curated(
                FstDictionary::curated(),
                Dialect::American,
            )),
        }
    }

    /// The findings of `text`. `include_spelling` is false for text that came
    /// from speech, where spelling belongs to the transcriber.
    ///
    /// A poisoned lock yields no findings; the caller cannot tell that apart
    /// from a clean text, so the seam above must treat a failed check as "not
    /// checked" (the rubric scorer and the workshop both do).
    pub fn findings(&self, text: &str, include_spelling: bool) -> Vec<Finding> {
        let Ok(mut linter) = self.linter.lock() else {
            return Vec::new();
        };
        let document = Document::new_curated(text, &harper_core::parsers::PlainEnglish);
        linter
            .lint(&document)
            .into_iter()
            .filter(|l| include_spelling || l.lint_kind != LintKind::Spelling)
            .map(|l| Finding {
                start: l.span.start,
                end: l.span.end,
                kind: format!("{:?}", l.lint_kind),
                message: l.message,
                suggestions: l.suggestions.iter().map(ToString::to_string).collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_article_is_reported() {
        let checker = GrammarChecker::new();
        let findings = checker.findings("This is an test.", false);
        assert!(
            findings
                .iter()
                .any(|f| f.message.to_lowercase().contains("article")),
            "{findings:?}"
        );
    }

    #[test]
    fn a_clean_sentence_is_clean() {
        let checker = GrammarChecker::new();
        assert!(
            checker
                .findings("She goes to school every day.", false)
                .is_empty()
        );
    }

    #[test]
    fn spelling_can_be_left_out_for_transcribed_speech() {
        let checker = GrammarChecker::new();
        let with = checker.findings("I recieve a letter.", true);
        assert!(with.iter().any(|f| f.kind == "Spelling"), "{with:?}");
        let without = checker.findings("I recieve a letter.", false);
        assert!(without.iter().all(|f| f.kind != "Spelling"), "{without:?}");
    }

    #[test]
    fn the_same_checker_can_be_used_more_than_once() {
        let checker = GrammarChecker::new();
        let first = checker.findings("This is an test.", false);
        let second = checker.findings("This is an test.", false);
        assert_eq!(first, second, "findings are deterministic across calls");
    }

    /// The tutor-engine seams hold the checker as `Arc<dyn GrammarCheck + Send + Sync>`
    /// and run it in `spawn_blocking`. This fails to compile if harper's
    /// `concurrent` feature is ever turned off.
    #[test]
    fn the_checker_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GrammarChecker>();
    }
}
