//! A suite that measures the harness itself, so the result path can be checked
//! without any engine. It sorts a fixed array and times that. It says nothing
//! about speech.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use time::OffsetDateTime;

use crate::machine::MachineInfo;
use crate::memory::PeakMemorySampler;
use crate::profile::{Profile, check_profile};
use crate::result::{BenchResult, EngineRecord, Metric, ResultError, RunState, run_id};
use crate::stats::summarize;

const ARRAY_LEN: usize = 50_000;

fn workload() -> f64 {
    // A fixed, scrambled array: the work is the same on every run.
    let mut data: Vec<u32> = (0..ARRAY_LEN as u32)
        .map(|n| n.wrapping_mul(2_654_435_761).rotate_left(7))
        .collect();
    let start = Instant::now();
    data.sort_unstable();
    black_box(&data);
    start.elapsed().as_secs_f64() * 1000.0
}

/// Runs the self-test and writes two result files: one cold run of a single
/// sample, and one warm run of `warm_samples`. Cold and warm are never mixed.
pub fn run(
    profile: Profile,
    warm_samples: usize,
    out_dir: &Path,
) -> Result<Vec<std::path::PathBuf>, ResultError> {
    let machine = MachineInfo::detect();
    check_profile(profile, &machine)?;
    let id = run_id(OffsetDateTime::now_utc());

    let sampler = PeakMemorySampler::start();
    let cold = workload();
    let warm: Vec<f64> = (0..warm_samples.max(1)).map(|_| workload()).collect();
    let rss_peak_mb = sampler.stop();

    let engine = EngineRecord {
        id: "selftest-sort".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        model_sha256: None,
        quantization: None,
        threads: 1,
    };
    let make = |state: RunState, timings: &[f64]| -> Result<BenchResult, ResultError> {
        let summary = summarize(timings).ok_or_else(|| {
            ResultError::Invalid("the self-test produced no usable timing".into())
        })?;
        let mut metrics = BTreeMap::new();
        metrics.insert("sort_ms".to_owned(), Metric::Distribution(summary));
        if let Some(peak) = rss_peak_mb {
            metrics.insert("rss_peak_mb".to_owned(), Metric::Number(peak));
        }
        Ok(BenchResult {
            suite: "selftest".into(),
            run_id: id.clone(),
            profile,
            machine: machine.clone(),
            engine: engine.clone(),
            state,
            fixture_set: format!("generated-{ARRAY_LEN}@1"),
            samples: timings.len(),
            metrics,
        })
    };

    Ok(vec![
        make(RunState::Cold, &[cold])?.write_to(out_dir)?,
        make(RunState::Warm, &warm)?.write_to(out_dir)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_separate_valid_cold_and_warm_files() {
        let dir = tempfile::tempdir().expect("dir");
        let paths = run(Profile::Dev, 120, dir.path()).expect("run");
        assert_eq!(paths.len(), 2);
        let cold = BenchResult::read_from(&paths[0]).expect("cold");
        let warm = BenchResult::read_from(&paths[1]).expect("warm");
        assert_eq!(cold.state, RunState::Cold);
        assert_eq!(cold.samples, 1);
        assert_eq!(warm.state, RunState::Warm);
        assert_eq!(warm.samples, 120);
        // 120 warm samples are enough for percentiles; one cold sample is not.
        assert!(matches!(
            warm.metrics.get("sort_ms"),
            Some(Metric::Distribution(
                crate::stats::Distribution::Percentiles { .. }
            ))
        ));
        assert!(matches!(
            cold.metrics.get("sort_ms"),
            Some(Metric::Distribution(
                crate::stats::Distribution::Small { .. }
            ))
        ));
        assert_eq!(warm.engine.id, "selftest-sort");
    }

    #[test]
    fn the_floor_tag_is_refused_on_an_uncapped_machine() {
        let machine = MachineInfo::detect();
        if machine.logical_processors <= 4 && machine.ram_mb <= 8704 {
            return; // This machine really is the floor, so the refusal cannot be shown here.
        }
        let dir = tempfile::tempdir().expect("dir");
        assert!(run(Profile::Floor, 5, dir.path()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
    }
}
