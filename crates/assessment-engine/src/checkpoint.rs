//! Unit checkpoints (ASSESSMENT_SPEC section 10): the mean of a unit's
//! checkpoint activity scores against the unit's pass mark.
//!
//! Pure logic: the caller reads the stored attempts back and hands one
//! [`CheckpointItem`] per checkpoint activity — `Some(score)` when every
//! dimension row of the response is scored, `None` while anything still waits
//! (a productive response stored as `pending_llm`). An unscored item makes the
//! decision provisional: it rests on the scored items, as the spec says, and
//! nothing passes until at least one item is scored.

use crate::SUCCESS_SCORE;

/// The pass mark a unit falls back to when its file does not set one
/// (ASSESSMENT_SPEC section 10). A unit file's own `pass_score` wins.
pub const DEFAULT_PASS_MARK: f64 = SUCCESS_SCORE;

/// One checkpoint activity: its score from 0 to 1, or `None` while the
/// response waits for a provider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckpointItem {
    pub score: Option<f64>,
}

/// What the checkpoint's items decide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckpointOutcome {
    pub passed: bool,
    /// Mean of the scored items, 0 when none is scored yet.
    pub mean: f64,
    /// True while at least one item is still unscored; the decision then rests
    /// on the scored items.
    pub provisional: bool,
}

/// Mean of the scored items compared with the pass mark. Unscored items make
/// the outcome provisional and leave the decision to the scored ones; with
/// nothing scored at all, nothing is passed.
pub fn evaluate_checkpoint(items: &[CheckpointItem], pass_mark: f64) -> CheckpointOutcome {
    let scored: Vec<f64> = items.iter().filter_map(|item| item.score).collect();
    if scored.is_empty() {
        return CheckpointOutcome {
            passed: false,
            mean: 0.0,
            provisional: true,
        };
    }
    let mean = scored.iter().sum::<f64>() / scored.len() as f64;
    CheckpointOutcome {
        passed: mean >= pass_mark,
        mean,
        provisional: scored.len() < items.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(scores: &[Option<f64>]) -> Vec<CheckpointItem> {
        scores
            .iter()
            .map(|score| CheckpointItem { score: *score })
            .collect()
    }

    #[test]
    fn the_mean_is_compared_with_the_pass_mark() {
        let outcome = evaluate_checkpoint(&items(&[Some(1.0), Some(0.5), Some(0.8)]), 0.7);
        assert!(outcome.passed);
        assert!(!outcome.provisional);
        assert!((outcome.mean - 0.7666).abs() < 0.001);
        assert!(!evaluate_checkpoint(&items(&[Some(1.0), Some(0.0)]), 0.7).passed);
    }

    #[test]
    fn exactly_the_pass_mark_passes() {
        assert!(evaluate_checkpoint(&items(&[Some(0.7)]), 0.7).passed);
    }

    #[test]
    fn unscored_items_make_the_result_provisional_and_leave_the_decision_to_the_rest() {
        let outcome = evaluate_checkpoint(&items(&[Some(1.0), Some(0.8), None]), 0.7);
        assert!(outcome.passed);
        assert!(outcome.provisional);
        assert!((outcome.mean - 0.9).abs() < 1e-9);
        // Without the scored items the same pending set would fail.
        assert!(!evaluate_checkpoint(&items(&[Some(0.4), None]), 0.7).passed);
    }

    #[test]
    fn nothing_scored_passes_nothing() {
        let outcome = evaluate_checkpoint(&items(&[None, None]), 0.7);
        assert_eq!(
            (outcome.passed, outcome.provisional, outcome.mean),
            (false, true, 0.0)
        );
        let empty = evaluate_checkpoint(&[], 0.7);
        assert!(!empty.passed);
        assert!(empty.provisional);
    }

    #[test]
    fn the_default_pass_mark_is_the_success_line() {
        assert_eq!(DEFAULT_PASS_MARK, 0.7);
    }
}
