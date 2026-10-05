//! The result file every suite writes (benchmark plan, section 3).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::machine::MachineInfo;
use crate::profile::{GuardError, Profile, check_profile};
use crate::stats::Distribution;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Warm,
    Cold,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warm => "warm",
            Self::Cold => "cold",
        }
    }
}

/// Which engine and model produced the numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineRecord {
    pub id: String,
    pub version: String,
    /// `None` only for an engine that loads no model.
    pub model_sha256: Option<String>,
    pub quantization: Option<String>,
    pub threads: u32,
}

/// One measured quantity. A number is a single value such as a word error rate;
/// a distribution is a list of timings summarised by `stats::summarize`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Metric {
    Number(f64),
    Distribution(Distribution),
}

impl Metric {
    fn is_valid(&self) -> bool {
        match self {
            Self::Number(value) => value.is_finite(),
            Self::Distribution(distribution) => distribution.is_valid(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchResult {
    pub suite: String,
    /// UTC start time, written like `2026-10-20T09-15-00Z` so it is safe in a file name.
    pub run_id: String,
    pub profile: Profile,
    pub machine: MachineInfo,
    pub engine: EngineRecord,
    pub state: RunState,
    /// Fixture set and version, for example `F2@1`.
    pub fixture_set: String,
    pub samples: usize,
    pub metrics: BTreeMap<String, Metric>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResultError {
    #[error("result is invalid: {0}")]
    Invalid(String),
    #[error(transparent)]
    Guard(#[from] GuardError),
    #[error("could not read or write a result file: {0}")]
    Io(#[from] std::io::Error),
    #[error("a result file is not valid JSON for this format: {0}")]
    Json(#[from] serde_json::Error),
}

/// `2026-10-20T09-15-00Z` for the given UTC time.
pub fn run_id(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

impl BenchResult {
    /// Checks that the file states everything the plan's first rule asks for,
    /// that no number is missing or not finite, and that a `floor` tag matches
    /// the machine recorded in the file.
    pub fn validate(&self) -> Result<(), ResultError> {
        let bad = |message: &str| Err(ResultError::Invalid(message.to_owned()));
        if self.suite.trim().is_empty() {
            return bad("suite is empty");
        }
        if self.run_id.len() != 20 || !self.run_id.ends_with('Z') {
            return bad("run_id must look like 2026-10-20T09-15-00Z");
        }
        if self.machine.cpu.trim().is_empty() || self.machine.os.trim().is_empty() {
            return bad("machine.cpu and machine.os must be filled in");
        }
        if self.machine.logical_processors == 0 || self.machine.ram_mb == 0 {
            return bad("machine counts must be positive");
        }
        if self.engine.id.trim().is_empty() || self.engine.version.trim().is_empty() {
            return bad("engine id and version must be filled in");
        }
        if self.engine.threads == 0 {
            return bad("engine.threads must be at least 1");
        }
        if self.fixture_set.trim().is_empty() {
            return bad("fixture_set is empty");
        }
        if self.samples == 0 {
            return bad("a result needs at least one sample");
        }
        if self.metrics.is_empty() {
            return bad("a result needs at least one metric");
        }
        for (name, metric) in &self.metrics {
            if !metric.is_valid() {
                return Err(ResultError::Invalid(format!(
                    "metric {name} is empty or not finite"
                )));
            }
        }
        check_profile(self.profile, &self.machine)?;
        Ok(())
    }

    pub fn file_name(&self) -> String {
        format!(
            "{}-{}-{}-{}.json",
            self.suite,
            self.profile.as_str(),
            self.state.as_str(),
            self.run_id
        )
    }

    /// Validates, then writes `<dir>/<file_name>` and returns the path.
    pub fn write_to(&self, dir: &Path) -> Result<PathBuf, ResultError> {
        self.validate()?;
        std::fs::create_dir_all(dir)?;
        let path = dir.join(self.file_name());
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        std::fs::write(&path, text)?;
        Ok(path)
    }

    /// Reads a result file and validates it.
    pub fn read_from(path: &Path) -> Result<Self, ResultError> {
        let result: Self = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::summarize;

    pub(crate) fn sample_result() -> BenchResult {
        let mut metrics = BTreeMap::new();
        metrics.insert(
            "finalize_ms".to_owned(),
            Metric::Distribution(summarize(&[10.0, 12.0, 11.0]).expect("summary")),
        );
        metrics.insert("wer".to_owned(), Metric::Number(0.12));
        BenchResult {
            suite: "stt".into(),
            run_id: "2026-10-20T09-15-00Z".into(),
            profile: Profile::Dev,
            machine: MachineInfo {
                cpu: "Test CPU".into(),
                logical_processors: 12,
                ram_mb: 24_576,
                os: "Test OS".into(),
            },
            engine: EngineRecord {
                id: "stt-test".into(),
                version: "1.0".into(),
                model_sha256: Some("abc".into()),
                quantization: Some("int8".into()),
                threads: 4,
            },
            state: RunState::Warm,
            fixture_set: "F2@1".into(),
            samples: 3,
            metrics,
        }
    }

    #[test]
    fn run_id_is_a_file_safe_utc_stamp() {
        let at = OffsetDateTime::from_unix_timestamp(1_792_487_700).expect("time");
        assert_eq!(run_id(at), "2026-10-20T09-15-00Z");
    }

    #[test]
    fn a_complete_result_is_valid() {
        assert!(sample_result().validate().is_ok());
    }

    type Mutation = Box<dyn Fn(&mut BenchResult)>;

    #[test]
    fn each_missing_piece_is_caught() {
        let cases: Vec<(&str, Mutation)> = vec![
            ("suite", Box::new(|r| r.suite = String::new())),
            ("run id", Box::new(|r| r.run_id = "now".into())),
            ("cpu", Box::new(|r| r.machine.cpu = " ".into())),
            ("engine id", Box::new(|r| r.engine.id = String::new())),
            ("threads", Box::new(|r| r.engine.threads = 0)),
            ("fixtures", Box::new(|r| r.fixture_set = String::new())),
            ("samples", Box::new(|r| r.samples = 0)),
            ("metrics", Box::new(|r| r.metrics.clear())),
            (
                "nan",
                Box::new(|r| {
                    r.metrics.insert("wer".into(), Metric::Number(f64::NAN));
                }),
            ),
        ];
        for (name, break_it) in cases {
            let mut result = sample_result();
            break_it(&mut result);
            assert!(result.validate().is_err(), "{name} should be rejected");
        }
    }

    #[test]
    fn a_floor_tag_on_an_uncapped_machine_is_invalid() {
        let mut result = sample_result();
        result.profile = Profile::Floor;
        assert!(matches!(result.validate(), Err(ResultError::Guard(_))));
        result.machine.logical_processors = 4;
        result.machine.ram_mb = 8192;
        assert!(result.validate().is_ok());
    }

    #[test]
    fn writing_and_reading_round_trips_through_a_file() {
        let dir = tempfile::tempdir().expect("dir");
        let result = sample_result();
        let path = result.write_to(dir.path()).expect("written");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("stt-dev-warm-2026-10-20T09-15-00Z.json")
        );
        assert_eq!(BenchResult::read_from(&path).expect("read"), result);
    }

    #[test]
    fn an_invalid_result_is_never_written() {
        let dir = tempfile::tempdir().expect("dir");
        let mut result = sample_result();
        result.samples = 0;
        assert!(result.write_to(dir.path()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
    }
}
