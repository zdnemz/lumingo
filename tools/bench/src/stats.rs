//! Percentiles under the reporting rule of BENCHMARK_PLAN section 1, rule 2.

use serde::Serialize;

/// With at least this many samples p50, p95 and p99 are reported; below it the
/// count, minimum, median and maximum are.
pub const MIN_SAMPLES_FOR_PERCENTILES: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Summary {
    Percentiles {
        p50: f64,
        p95: f64,
        p99: f64,
    },
    Range {
        count: usize,
        min: f64,
        median: f64,
        max: f64,
    },
}

/// Nearest-rank percentile of an already sorted slice. `p` is in (0, 100].
fn nearest_rank(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Returns `None` for no samples or any NaN, so a broken measurement is never
/// summarised as a number.
pub fn summarize(samples: &[f64]) -> Option<Summary> {
    if samples.is_empty() || samples.iter().any(|s| s.is_nan()) {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(if sorted.len() >= MIN_SAMPLES_FOR_PERCENTILES {
        Summary::Percentiles {
            p50: nearest_rank(&sorted, 50.0),
            p95: nearest_rank(&sorted, 95.0),
            p99: nearest_rank(&sorted, 99.0),
        }
    } else {
        Summary::Range {
            count: sorted.len(),
            min: sorted[0],
            median: nearest_rank(&sorted, 50.0),
            max: sorted[sorted.len() - 1],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hundred_samples_give_nearest_rank_percentiles() {
        let samples: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(
            summarize(&samples),
            Some(Summary::Percentiles {
                p50: 50.0,
                p95: 95.0,
                p99: 99.0
            })
        );
    }

    #[test]
    fn input_order_does_not_matter() {
        let mut samples: Vec<f64> = (1..=100).map(f64::from).collect();
        samples.reverse();
        assert!(
            matches!(summarize(&samples), Some(Summary::Percentiles { p95, .. }) if p95 == 95.0)
        );
    }

    #[test]
    fn fewer_than_hundred_report_count_and_range_only() {
        assert_eq!(
            summarize(&[30.0, 10.0, 20.0]),
            Some(Summary::Range {
                count: 3,
                min: 10.0,
                median: 20.0,
                max: 30.0
            })
        );
    }

    #[test]
    fn empty_and_nan_input_are_refused() {
        assert_eq!(summarize(&[]), None);
        assert_eq!(summarize(&[1.0, f64::NAN]), None);
    }
}
