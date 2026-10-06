//! Benchmark harness. One binary, suites as subcommands (BENCHMARK_PLAN section 3).
//! Only the `dummy` suite exists so far: it proves the result format, the
//! percentile rule, memory sampling and the `floor` guard end to end.

mod machine;
mod memory;
mod result;
mod stats;

use anyhow::{Result, bail};
use machine::Profile;
use result::{BenchResult, Engine, State};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Instant, SystemTime},
};

const USAGE: &str = "usage: bench dummy --profile <dev|floor> [--state <cold|warm>] [--out <dir>]";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("dummy") => dummy(args.collect()),
        _ => bail!(USAGE),
    }
}

fn dummy(args: Vec<String>) -> Result<()> {
    let (mut profile, mut state, mut out) =
        (None, State::Warm, PathBuf::from("benchmarks/results"));
    let mut it = args.into_iter();
    while let Some(flag) = it.next() {
        let value = it.next();
        match (flag.as_str(), value) {
            ("--profile", Some(v)) => profile = Some(v.parse::<Profile>()?),
            ("--state", Some(v)) if v == "cold" => state = State::Cold,
            ("--state", Some(v)) if v == "warm" => state = State::Warm,
            ("--out", Some(v)) => out = PathBuf::from(v),
            _ => bail!(USAGE),
        }
    }
    let Some(profile) = profile else { bail!(USAGE) };

    let machine = machine::detect()?;
    machine::check_profile(profile, &machine)?;

    let samples: Vec<f64> = (0..120)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box((0..10_000u64).sum::<u64>());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();

    let mut metrics = BTreeMap::new();
    metrics.insert(
        "work_ms".to_owned(),
        serde_json::to_value(stats::summarize(&samples))?,
    );
    metrics.insert(
        "rss_peak_mb".to_owned(),
        serde_json::to_value(memory::rss_peak_mb())?,
    );

    let path = BenchResult {
        suite: "dummy".into(),
        run_id: result::run_id(SystemTime::now()),
        profile,
        machine,
        engine: Engine {
            id: "none".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            model_sha256: None,
            quantization: None,
            threads: 1,
        },
        state,
        fixture_set: "none".into(),
        samples: samples.len(),
        metrics,
    }
    .write(&out)?;
    println!("wrote {}", path.display());
    Ok(())
}
