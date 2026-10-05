//! The diagnostics page: what this machine has, how fast the measured steps
//! were, and what the provider test found. Every number comes from storage or
//! from the operating system; with no data the answer is `None`, not a guess.

use storage::{LlmOutcome, Timestamp};

use crate::api::{
    CurriculumDiagnostics, DiagnosticsReport, LatencyStat, LlmCallStats, Percentiles,
};
use crate::core::AppCore;
use crate::error::CoreResult;

/// The measured steps the schema documents (`perf_samples.metric`).
const LATENCY_METRICS: [&str; 5] = [
    "endpoint_ms",
    "stt_ms",
    "llm_first_sentence_ms",
    "tts_first_ms",
    "e2e_ms",
];
/// Latency samples older than this are not shown.
const LATENCY_WINDOW_DAYS: u32 = 30;
/// How many of the newest provider calls feed the statistics.
const LLM_CALL_WINDOW: u32 = 100;

/// Nearest-rank percentile of sorted values: the smallest value such that at
/// least `p` percent of the values are at or below it.
fn nearest_rank(sorted: &[f64], p: u32) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    let rank = (n * usize::try_from(p).unwrap_or(100)).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

fn percentiles(mut values: Vec<f64>) -> Percentiles {
    values.sort_by(f64::total_cmp);
    Percentiles {
        count: u32::try_from(values.len()).unwrap_or(u32::MAX),
        p50_ms: nearest_rank(&values, 50),
        p95_ms: nearest_rank(&values, 95),
    }
}

impl AppCore {
    pub async fn diagnostics(&self) -> CoreResult<DiagnosticsReport> {
        let now: Timestamp = self.clock().now();
        let since = now
            .minus_days(LATENCY_WINDOW_DAYS)
            .map_err(storage::StorageError::from)?;
        let diagnostics = self.db.diagnostics();

        let mut latency = Vec::new();
        for metric in LATENCY_METRICS {
            let values: Vec<f64> = diagnostics
                .perf_samples(metric, &since)
                .await?
                .into_iter()
                .map(|sample| sample.value_ms)
                .collect();
            latency.push(LatencyStat {
                metric: metric.to_owned(),
                stats: percentiles(values),
            });
        }

        let calls = diagnostics.recent_llm_calls(LLM_CALL_WINDOW).await?;
        let failures = calls
            .iter()
            .filter(|call| !matches!(call.outcome, LlmOutcome::Ok | LlmOutcome::Repaired))
            .count();
        let ms = |pick: fn(&storage::LlmCall) -> Option<i64>| -> Vec<f64> {
            calls
                .iter()
                .filter_map(pick)
                // A latency is far below 2^53 ms, so the conversion is exact.
                .map(|value| value as f64)
                .collect()
        };
        let llm = LlmCallStats {
            calls: u32::try_from(calls.len()).unwrap_or(u32::MAX),
            failures: u32::try_from(failures).unwrap_or(u32::MAX),
            ttft: percentiles(ms(|call| call.ttft_ms)),
            total: percentiles(ms(|call| call.total_ms)),
        };

        let units = self.list_units();
        Ok(DiagnosticsReport {
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            dev_mode: self.config.dev_mode,
            uptime_ms: self.uptime_ms(),
            server_address: self.config.server_address.clone(),
            data_dir: self.config.data_dir.display().to_string(),
            log_folder: None,
            curriculum_dir: self.config.curriculum_dir.display().to_string(),
            schema_version: self.db.schema_version().await?,
            hardware: self.hardware.clone(),
            provider: self.providers.active(),
            curriculum: CurriculumDiagnostics {
                unit_count: u32::try_from(units.units.len()).unwrap_or(u32::MAX),
                issue_count: u32::try_from(units.issues.len()).unwrap_or(u32::MAX),
                content_version: units.content_version,
            },
            latency,
            llm,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles_follow_the_definition() {
        let one_to_ten: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(nearest_rank(&one_to_ten, 50), Some(5.0));
        assert_eq!(nearest_rank(&one_to_ten, 95), Some(10.0));
        assert_eq!(nearest_rank(&[7.0], 50), Some(7.0));
        assert_eq!(nearest_rank(&[7.0], 95), Some(7.0));
        let hundred: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(nearest_rank(&hundred, 95), Some(95.0));
        assert_eq!(nearest_rank(&[], 50), None);
    }

    #[test]
    fn percentiles_sort_their_input_and_count_it() {
        let stats = percentiles(vec![30.0, 10.0, 20.0]);
        assert_eq!(stats.count, 3);
        assert_eq!(stats.p50_ms, Some(20.0));
        assert_eq!(stats.p95_ms, Some(30.0));
        let none = percentiles(Vec::new());
        assert_eq!((none.count, none.p50_ms, none.p95_ms), (0, None, None));
    }
}
