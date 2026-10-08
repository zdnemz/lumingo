//! Percentiles and summaries, following the reporting rules of the benchmark plan.

use serde::{Deserialize, Serialize};

/// A run needs this many samples before p95 and p99 mean anything.
pub const MIN_SAMPLES_FOR_PERCENTILES: usize = 100;

/// How a list of timings is reported.
///
/// With at least 100 samples: p50, p95 and p99. With fewer: the count, the
/// minimum, the median and the maximum, and nothing that would pretend to be a
/// tail percentile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Distribution {
    Percentiles {
        count: usize,
        min: f64,
        p50: f64,
        p95: f64,
        p99: f64,
        max: f64,
    },
    Small {
        count: usize,
        min: f64,
        median: f64,
        max: f64,
    },
}

impl Distribution {
    pub fn count(&self) -> usize {
        match self {
            Self::Percentiles { count, .. } | Self::Small { count, .. } => *count,
        }
    }

    fn all_finite(&self) -> bool {
        match self {
            Self::Percentiles {
                min,
                p50,
                p95,
                p99,
                max,
                ..
            } => [min, p50, p95, p99, max].iter().all(|v| v.is_finite()),
            Self::Small {
                min, median, max, ..
            } => [min, median, max].iter().all(|v| v.is_finite()),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.all_finite() && self.count() > 0
    }
}

/// The value at percentile `p` (0 to 100) of an ascending list, by the
/// nearest-rank method: the smallest sample such that at least `p` percent of
/// the samples are at or below it. `None` for an empty list.
pub fn percentile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    let index = rank.clamp(1, sorted.len()) - 1;
    sorted.get(index).copied()
}

/// Summarises timings. Returns `None` when the list is empty or holds a value
/// that is not a finite number, because a result file must never carry one.
pub fn summarize(samples: &[f64]) -> Option<Distribution> {
    if samples.is_empty() || samples.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    let min = *sorted.first()?;
    let max = *sorted.last()?;
    if count >= MIN_SAMPLES_FOR_PERCENTILES {
        Some(Distribution::Percentiles {
            count,
            min,
            p50: percentile_sorted(&sorted, 50.0)?,
            p95: percentile_sorted(&sorted, 95.0)?,
            p99: percentile_sorted(&sorted, 99.0)?,
            max,
        })
    } else {
        // The median of an even count is the mean of the two middle samples.
        let mid = count / 2;
        let median = if count % 2 == 1 {
            *sorted.get(mid)?
        } else {
            (sorted.get(mid - 1)? + sorted.get(mid)?) / 2.0
        };
        Some(Distribution::Small {
            count,
            min,
            median,
            max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<f64> {
        (1..=n).map(|v| v as f64).collect()
    }

    #[test]
    fn nearest_rank_matches_hand_computed_values() {
        let data = ramp(100);
        assert_eq!(percentile_sorted(&data, 50.0), Some(50.0));
        assert_eq!(percentile_sorted(&data, 95.0), Some(95.0));
        assert_eq!(percentile_sorted(&data, 99.0), Some(99.0));
        assert_eq!(percentile_sorted(&data, 100.0), Some(100.0));
        // Ten samples: p95 is the 10th (ceil of 9.5), p50 the 5th.
        let ten = ramp(10);
        assert_eq!(percentile_sorted(&ten, 95.0), Some(10.0));
        assert_eq!(percentile_sorted(&ten, 50.0), Some(5.0));
        assert_eq!(percentile_sorted(&ten, 0.0), Some(1.0));
    }

    #[test]
    fn empty_input_has_no_percentile() {
        assert_eq!(percentile_sorted(&[], 50.0), None);
        assert_eq!(summarize(&[]), None);
    }

    #[test]
    fn a_hundred_samples_report_percentiles() {
        let mut shuffled = ramp(100);
        shuffled.reverse();
        match summarize(&shuffled) {
            Some(Distribution::Percentiles {
                count,
                min,
                p50,
                p95,
                p99,
                max,
            }) => {
                assert_eq!(
                    (count, min, p50, p95, p99, max),
                    (100, 1.0, 50.0, 95.0, 99.0, 100.0)
                );
            }
            other => panic!("expected percentiles, got {other:?}"),
        }
    }

    #[test]
    fn fewer_than_a_hundred_report_count_min_median_max() {
        match summarize(&ramp(99)) {
            Some(Distribution::Small {
                count,
                min,
                median,
                max,
            }) => {
                assert_eq!((count, min, median, max), (99, 1.0, 50.0, 99.0));
            }
            other => panic!("expected the small summary, got {other:?}"),
        }
        match summarize(&[4.0, 1.0, 3.0, 2.0]) {
            Some(Distribution::Small { median, .. }) => {
                assert!((median - 2.5).abs() < f64::EPSILON)
            }
            other => panic!("expected the small summary, got {other:?}"),
        }
    }

    #[test]
    fn a_single_sample_is_its_own_summary() {
        assert_eq!(
            summarize(&[7.0]),
            Some(Distribution::Small {
                count: 1,
                min: 7.0,
                median: 7.0,
                max: 7.0
            })
        );
    }

    #[test]
    fn non_finite_samples_are_rejected() {
        assert_eq!(summarize(&[1.0, f64::NAN]), None);
        assert_eq!(summarize(&[f64::INFINITY]), None);
    }

    #[test]
    fn the_two_shapes_round_trip_through_json() {
        for samples in [ramp(100), ramp(5)] {
            let summary = summarize(&samples).expect("summary");
            let json = serde_json::to_string(&summary).expect("serialise");
            let back: Distribution = serde_json::from_str(&json).expect("parse");
            assert_eq!(back, summary);
        }
    }
}
