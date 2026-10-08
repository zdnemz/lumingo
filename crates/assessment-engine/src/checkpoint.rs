//! Unit checkpoints (assessment spec, section 10).

use crate::deterministic::SUCCESS_THRESHOLD;

/// The pass mark when a unit file does not set one.
pub const DEFAULT_PASS_MARK: f64 = SUCCESS_THRESHOLD;

/// One activity of a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckpointItem {
    /// 0 to 1, or `None` while a productive item waits for a provider to score it.
    pub score: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckpointOutcome {
    pub passed: bool,
    /// Mean of the scored items, 0 when none is scored yet.
    pub mean: f64,
    /// True while some item is still unscored. The pass decision then rests on
    /// the scored items, and the result is shown as provisional.
    pub provisional: bool,
}

/// Mean of `normalized` over the checkpoint's activities, compared with the
/// pass mark. When no provider is reachable the productive items stay unscored;
/// the decision then uses the deterministic ones and the outcome says it is
/// provisional. With nothing scored at all, nothing is passed.
pub fn evaluate_checkpoint(items: &[CheckpointItem], pass_mark: f64) -> CheckpointOutcome {
    let scored: Vec<f64> = items.iter().filter_map(|item| item.score).collect();
    let provisional = scored.len() < items.len();
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
        provisional,
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
    }

    #[test]
    fn the_default_pass_mark_is_the_success_line() {
        assert_eq!(DEFAULT_PASS_MARK, 0.7);
    }
}
