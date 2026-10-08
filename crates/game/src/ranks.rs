use serde::{Deserialize, Serialize};

/// Sparks needed to reach each rank. Rank 1 starts at zero.
pub const RANK_THRESHOLDS: [u32; 6] = [0, 100, 300, 700, 1500, 3000];

/// Where a learner stands on the rank ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankInfo {
    /// 1 to 6.
    pub rank: u8,
    /// Progress toward the next rank, 0 to 100. The top rank always reports 100.
    pub progress_percent: u8,
    /// Sparks at which the next rank starts, or `None` at the top rank.
    pub next_threshold: Option<u32>,
}

/// The rank for a total number of sparks.
pub fn rank_for(sparks: u32) -> RankInfo {
    let index = RANK_THRESHOLDS
        .iter()
        .rposition(|threshold| sparks >= *threshold)
        .unwrap_or(0);
    let rank = u8::try_from(index + 1).unwrap_or(1);
    match RANK_THRESHOLDS.get(index + 1) {
        Some(&next) => {
            let start = RANK_THRESHOLDS.get(index).copied().unwrap_or(0);
            let span = next - start;
            let done = sparks - start;
            let percent = u8::try_from(u64::from(done) * 100 / u64::from(span)).unwrap_or(100);
            RankInfo {
                rank,
                progress_percent: percent.min(100),
                next_threshold: Some(next),
            }
        }
        None => RankInfo {
            rank,
            progress_percent: 100,
            next_threshold: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_start_at_zero_and_only_grow() {
        assert_eq!(RANK_THRESHOLDS[0], 0);
        assert!(RANK_THRESHOLDS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn each_threshold_starts_its_rank_and_one_less_does_not() {
        for (index, &threshold) in RANK_THRESHOLDS.iter().enumerate() {
            let expected = u8::try_from(index + 1).expect("small");
            assert_eq!(rank_for(threshold).rank, expected, "at {threshold}");
            if threshold > 0 {
                assert_eq!(
                    rank_for(threshold - 1).rank,
                    expected - 1,
                    "just under {threshold}"
                );
            }
        }
    }

    #[test]
    fn progress_runs_from_zero_to_nearly_full_inside_a_rank() {
        assert_eq!(rank_for(0).progress_percent, 0);
        assert_eq!(rank_for(50).progress_percent, 50);
        assert_eq!(rank_for(99).progress_percent, 99);
        assert_eq!(rank_for(100).progress_percent, 0);
        assert_eq!(rank_for(200).progress_percent, 50);
        assert_eq!(rank_for(0).next_threshold, Some(100));
    }

    #[test]
    fn the_top_rank_is_full_and_has_no_next_step() {
        let top = rank_for(3000);
        assert_eq!(
            (top.rank, top.progress_percent, top.next_threshold),
            (6, 100, None)
        );
        assert_eq!(rank_for(u32::MAX).rank, 6);
    }

    #[test]
    fn progress_never_goes_backward_as_sparks_grow() {
        let mut last = (1_u8, 0_u8);
        for sparks in 0..3200 {
            let info = rank_for(sparks);
            assert!(info.rank >= last.0);
            if info.rank == last.0 {
                assert!(info.progress_percent >= last.1, "at {sparks}");
            }
            last = (info.rank, info.progress_percent);
        }
    }
}
