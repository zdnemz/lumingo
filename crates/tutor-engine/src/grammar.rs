//! The linked rule-based checker for the seams that take one.
//!
//! `curriculum::validate::GrammarCheck` is the seam the rubric scorer (X5), the
//! writing workshop's first layer and the content validators share. This module
//! is the concrete implementation the tutor links: harper-core through
//! `assessment-engine`'s `grammar` feature (default on).
//!
//! Spelling is **included** here, and the seam decides per call: the rubric
//! scorer drops spelling findings for a voice response (a transcript's spelling
//! is the recogniser's, not the learner's) and keeps them for typed text; the
//! workshop keeps them on purpose (a typed draft is the learner's own spelling).
//! The content tools are the opposite split — W03 leaves spelling out — and
//! build their own checker in `content-cli`; do not "unify" the two.

#[cfg(feature = "grammar")]
mod harper {
    use curriculum::validate::GrammarCheck;

    /// The harper-backed checker. Building it loads a dictionary, so build one
    /// and share it; it is `Send + Sync` (harper's `concurrent` feature), so an
    /// `Arc` of it can go into every environment.
    pub struct HarperCheck {
        checker: assessment_engine::GrammarChecker,
    }

    impl Default for HarperCheck {
        fn default() -> Self {
            Self::new()
        }
    }

    impl HarperCheck {
        pub fn new() -> Self {
            Self {
                checker: assessment_engine::GrammarChecker::new(),
            }
        }
    }

    impl GrammarCheck for HarperCheck {
        /// `Kind: message` per finding. The rubric scorer's voice filter keys on
        /// the `Spelling` prefix, so the kind must lead the string.
        fn findings(&self, text: &str) -> Vec<String> {
            self.checker
                .findings(text, true)
                .into_iter()
                .map(|f| format!("{}: {}", f.kind, f.message))
                .collect()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_seam_finds_a_real_mistake_and_leads_with_the_kind() {
            let check = HarperCheck::new();
            let findings = check.findings("This is an test.");
            assert!(
                findings
                    .iter()
                    .any(|f| f.contains(":") && f.to_lowercase().contains("article")),
                "{findings:?}"
            );
        }

        #[test]
        fn spelling_is_kept_because_a_draft_is_typed() {
            let check = HarperCheck::new();
            let findings = check.findings("I recieve a letter.");
            assert!(
                findings.iter().any(|f| f.starts_with("Spelling")),
                "the workshop's layer needs spelling findings: {findings:?}"
            );
        }

        #[test]
        fn a_clean_sentence_gives_nothing() {
            let check = HarperCheck::new();
            assert!(check.findings("She goes to school every day.").is_empty());
        }
    }
}

#[cfg(feature = "grammar")]
pub use harper::HarperCheck;
