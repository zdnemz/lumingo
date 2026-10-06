//! The result file of BENCHMARK_PLAN section 3.

use crate::machine::{Machine, Profile};
use anyhow::{Context, Result};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path, time::SystemTime};

#[derive(Debug, Serialize)]
pub struct Engine {
    pub id: String,
    pub version: String,
    pub model_sha256: Option<String>,
    pub quantization: Option<String>,
    pub threads: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Cold,
    Warm,
}

#[derive(Debug, Serialize)]
pub struct BenchResult {
    pub suite: String,
    pub run_id: String,
    pub profile: Profile,
    pub machine: Machine,
    pub engine: Engine,
    pub state: State,
    pub fixture_set: String,
    pub samples: usize,
    /// Each metric is a `Summary` or a plain number; `None` means not measured.
    pub metrics: BTreeMap<String, serde_json::Value>,
}

impl BenchResult {
    /// Writes `<dir>/<suite>-<profile>-<state>-<run_id>.json` and returns its path.
    pub fn write(&self, dir: &Path) -> Result<std::path::PathBuf> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let name = format!(
            "{}-{}-{}-{}.json",
            self.suite,
            serde_json::to_value(self.profile)?
                .as_str()
                .unwrap_or("unknown"),
            serde_json::to_value(self.state)?
                .as_str()
                .unwrap_or("unknown"),
            self.run_id
        );
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_string_pretty(self)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }
}

/// UTC time as `2026-10-20T09-15-00Z`, safe in file names on Windows.
pub fn run_id(now: SystemTime) -> String {
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil-from-days (Howard Hinnant), valid for the whole proleptic Gregorian range.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn run_id_formats_known_instants() {
        assert_eq!(run_id(SystemTime::UNIX_EPOCH), "1970-01-01T00-00-00Z");
        // 2026-10-20 09:15:00 UTC
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_487_700);
        assert_eq!(run_id(t), "2026-10-20T09-15-00Z");
        // leap day
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_709_164_800);
        assert_eq!(run_id(t), "2024-02-29T00-00-00Z");
    }
}
