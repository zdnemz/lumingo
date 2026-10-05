//! Performance tier and the blocking-or-deferred policy (ROADMAP S3-11,
//! ADR-009).
//!
//! In conversation, phoneme analysis runs on every voiced learner turn. It
//! either finishes before the LLM request is sent (`Blocking`, the findings go
//! into that turn's prompt) or it is allowed to finish later (`Deferred`, the
//! findings appear when ready and go into the next turn). A startup
//! measurement predicts which is affordable; a setting can override it.
//!
//! The policy is a pure function of fixed numbers. The measurement harness only
//! times closures the caller supplies, so it runs without a model. It blocks:
//! call it from a dedicated thread, never from a Tokio worker.

use std::path::Path;
use std::sync::Barrier;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use speech::CancelFlag;

/// The longest utterance of the latency target, in seconds. PRD NFR-P1 sets the
/// end-to-end target for utterances of 2 to 10 seconds, so the prediction
/// defaults to the worst case it has to hold for.
pub const LONGEST_TARGET_UTTERANCE_SECONDS: f64 = 10.0;

#[derive(Debug, thiserror::Error)]
pub enum PerfError {
    #[error("{what} must be a finite positive number, got {value}")]
    NotPositive { what: &'static str, value: f64 },
    #[error("{what} must be a finite number at least 0, got {value}")]
    Negative { what: &'static str, value: f64 },
    #[error("the measurement was cancelled")]
    Cancelled,
    #[error("a workload failed: {0}")]
    Workload(String),
    #[error("a workload thread panicked")]
    Panicked,
    #[error("the measurement needs at least one timed run")]
    NoRuns,
    #[error("cannot read or write the speed profile {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("the speed profile is not valid JSON of the expected shape: {0}")]
    Json(String),
}

/// How long the machine takes, in seconds of compute per second of audio. A
/// smaller number is faster. "Together" means the two ran at the same time on
/// the same machine, the way they do in a conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub stt_alone: f64,
    pub stt_together: f64,
    /// Phoneme model inference (posteriors) only.
    pub model_alone: f64,
    pub model_together: f64,
    /// Alignment and scoring. It needs the transcript, so it runs after STT
    /// has finished and is timed alone.
    pub align: f64,
}

impl Measurement {
    pub fn validate(&self) -> Result<(), PerfError> {
        for (what, value) in [
            ("stt_alone", self.stt_alone),
            ("stt_together", self.stt_together),
            ("model_alone", self.model_alone),
            ("model_together", self.model_together),
            ("align", self.align),
        ] {
            if !(value.is_finite() && value > 0.0) {
                return Err(PerfError::NotPositive { what, value });
            }
        }
        Ok(())
    }

    /// How much slower STT and the phoneme model are when they share the
    /// machine (together divided by alone), as `(stt, model)`.
    pub fn contention(&self) -> (f64, f64) {
        (
            self.stt_together / self.stt_alone,
            self.model_together / self.model_alone,
        )
    }
}

/// A stored measurement with enough context to be a result: what was timed,
/// how, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedProfile {
    pub measurement: Measurement,
    /// Length of the audio fixture each run processed.
    pub fixture_audio_seconds: f64,
    /// Untimed runs before timing starts, so models are warm.
    pub warmup_runs: usize,
    /// Timed runs. Each reported figure is their median.
    pub timed_runs: usize,
    /// A label the caller gives the machine, for example `DEV` or `FLOOR`.
    pub label: String,
    /// Seconds since the Unix epoch, or `None` if the clock was unavailable.
    pub measured_at_unix: Option<u64>,
}

impl SpeedProfile {
    pub fn to_json(&self) -> Result<String, PerfError> {
        serde_json::to_string_pretty(self).map_err(|e| PerfError::Json(e.to_string()))
    }

    pub fn from_json(text: &str) -> Result<SpeedProfile, PerfError> {
        let profile: SpeedProfile =
            serde_json::from_str(text).map_err(|e| PerfError::Json(e.to_string()))?;
        profile.measurement.validate()?;
        Ok(profile)
    }

    pub fn save(&self, path: &Path) -> Result<(), PerfError> {
        std::fs::write(path, self.to_json()?).map_err(|source| PerfError::Io {
            path: path.to_owned(),
            source,
        })
    }

    pub fn load(path: &Path) -> Result<SpeedProfile, PerfError> {
        let text = std::fs::read_to_string(path).map_err(|source| PerfError::Io {
            path: path.to_owned(),
            source,
        })?;
        SpeedProfile::from_json(&text)
    }
}

/// One unit of work to time, for example "transcribe the fixture".
pub trait Workload: Send {
    fn run(&mut self, cancel: &CancelFlag) -> Result<(), String>;
}

impl<F> Workload for F
where
    F: FnMut(&CancelFlag) -> Result<(), String> + Send,
{
    fn run(&mut self, cancel: &CancelFlag) -> Result<(), String> {
        self(cancel)
    }
}

fn timed(workload: &mut dyn Workload, cancel: &CancelFlag) -> Result<Duration, PerfError> {
    if cancel.is_cancelled() {
        return Err(PerfError::Cancelled);
    }
    let start = Instant::now();
    workload.run(cancel).map_err(PerfError::Workload)?;
    Ok(start.elapsed())
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

/// What the harness times.
pub struct Workloads<'a> {
    pub stt: &'a mut dyn Workload,
    pub model: &'a mut dyn Workload,
    pub align: &'a mut dyn Workload,
}

/// Measures STT and the phoneme model alone and together, and alignment alone.
///
/// Each workload first runs once untimed (a cold model would overstate the
/// cost of every later turn). Each figure is the median of `timed_runs`. The
/// "together" runs start both workloads at the same instant on two threads and
/// time each on its own, so the figures include the contention a real turn has.
pub fn measure(
    workloads: Workloads<'_>,
    fixture_audio_seconds: f64,
    timed_runs: usize,
    label: &str,
    cancel: &CancelFlag,
) -> Result<SpeedProfile, PerfError> {
    if !(fixture_audio_seconds.is_finite() && fixture_audio_seconds > 0.0) {
        return Err(PerfError::NotPositive {
            what: "fixture_audio_seconds",
            value: fixture_audio_seconds,
        });
    }
    if timed_runs == 0 {
        return Err(PerfError::NoRuns);
    }
    let Workloads { stt, model, align } = workloads;
    let per_second = |d: Duration| d.as_secs_f64() / fixture_audio_seconds;

    const WARMUP_RUNS: usize = 1;
    for _ in 0..WARMUP_RUNS {
        timed(&mut *stt, cancel)?;
        timed(&mut *model, cancel)?;
        timed(&mut *align, cancel)?;
    }

    let mut stt_alone = Vec::new();
    let mut model_alone = Vec::new();
    let mut align_alone = Vec::new();
    for _ in 0..timed_runs {
        stt_alone.push(per_second(timed(&mut *stt, cancel)?));
        model_alone.push(per_second(timed(&mut *model, cancel)?));
        align_alone.push(per_second(timed(&mut *align, cancel)?));
    }

    let mut stt_together = Vec::new();
    let mut model_together = Vec::new();
    for _ in 0..timed_runs {
        let barrier = Barrier::new(2);
        let (a, b) = std::thread::scope(|scope| {
            let stt_handle = scope.spawn(|| {
                barrier.wait();
                timed(&mut *stt, cancel)
            });
            let model_handle = scope.spawn(|| {
                barrier.wait();
                timed(&mut *model, cancel)
            });
            (stt_handle.join(), model_handle.join())
        });
        stt_together.push(per_second(a.map_err(|_| PerfError::Panicked)??));
        model_together.push(per_second(b.map_err(|_| PerfError::Panicked)??));
    }

    let measurement = Measurement {
        stt_alone: median(stt_alone),
        stt_together: median(stt_together),
        model_alone: median(model_alone),
        model_together: median(model_together),
        align: median(align_alone),
    };
    measurement.validate()?;
    Ok(SpeedProfile {
        measurement,
        fixture_audio_seconds,
        warmup_runs: WARMUP_RUNS,
        timed_runs,
        label: label.to_owned(),
        measured_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs()),
    })
}

/// The two timings of ADR-009.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PronMode {
    /// Analysis finishes before the LLM request; findings go into this turn.
    Blocking,
    /// Findings appear when ready and go into the next turn.
    Deferred,
}

/// The user's setting. `Automatic` follows the measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeSetting {
    #[default]
    Automatic,
    Blocking,
    Deferred,
}

/// What the machine can afford, from the measurement alone, whatever the
/// setting says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceTier {
    /// Predicted added wait is within the budget: blocking is affordable.
    Headroom,
    /// Predicted added wait exceeds the budget: blocking would slow the reply.
    Constrained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    /// The setting named a mode.
    Setting,
    /// The setting was automatic and the measurement decided.
    Measurement,
    /// The setting was automatic and nothing was measured, so the mode that
    /// never delays a reply was chosen.
    Unmeasured,
}

/// The inputs of the prediction that are not measurements.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Length of the utterance the prediction is made for.
    pub reference_utterance_seconds: f64,
    /// How much extra wait before the LLM request is acceptable, in
    /// milliseconds. It comes from the latency budget (see [`headroom_ms`]) and
    /// is not guessed here; there is no default.
    pub max_added_wait_ms: f64,
}

impl PolicyConfig {
    pub fn new(
        reference_utterance_seconds: f64,
        max_added_wait_ms: f64,
    ) -> Result<PolicyConfig, PerfError> {
        let config = PolicyConfig {
            reference_utterance_seconds,
            max_added_wait_ms,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), PerfError> {
        if !(self.reference_utterance_seconds.is_finite() && self.reference_utterance_seconds > 0.0)
        {
            return Err(PerfError::NotPositive {
                what: "reference_utterance_seconds",
                value: self.reference_utterance_seconds,
            });
        }
        if !(self.max_added_wait_ms.is_finite() && self.max_added_wait_ms >= 0.0) {
            return Err(PerfError::Negative {
                what: "max_added_wait_ms",
                value: self.max_added_wait_ms,
            });
        }
        Ok(())
    }
}

/// Headroom between a latency target and the sum of the budget parts, never
/// negative. For example the DEV column of context_pack section 4 sums to
/// 2550 ms against the 3000 ms target of NFR-P1, so 450 ms. The caller reads
/// the numbers from the benchmark report; this only does the subtraction.
pub fn headroom_ms(target_ms: f64, budget_sum_ms: f64) -> f64 {
    (target_ms - budget_sum_ms).max(0.0)
}

/// The prediction behind a decision, in milliseconds for the reference
/// utterance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    pub reference_utterance_seconds: f64,
    pub stt_ms: f64,
    pub model_ms: f64,
    pub align_ms: f64,
    /// Wait the analysis adds before the LLM request can be sent: the part of
    /// the model run that outlasts STT, plus alignment, which needs the
    /// transcript.
    pub added_wait_ms: f64,
    pub max_added_wait_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModeDecision {
    pub mode: PronMode,
    pub source: DecisionSource,
    /// `None` when nothing was measured.
    pub tier: Option<PerformanceTier>,
    pub prediction: Option<Prediction>,
}

fn predict(measurement: &Measurement, config: &PolicyConfig) -> Prediction {
    let seconds = config.reference_utterance_seconds;
    let stt_ms = seconds * measurement.stt_together * 1000.0;
    let model_ms = seconds * measurement.model_together * 1000.0;
    let align_ms = seconds * measurement.align * 1000.0;
    Prediction {
        reference_utterance_seconds: seconds,
        stt_ms,
        model_ms,
        align_ms,
        added_wait_ms: (model_ms - stt_ms).max(0.0) + align_ms,
        max_added_wait_ms: config.max_added_wait_ms,
    }
}

/// Chooses the timing mode.
///
/// The model runs in parallel with STT, so it only delays the request by the
/// amount it outlasts STT; alignment needs the transcript and always adds its
/// own time. Blocking is chosen when that sum is within `max_added_wait_ms`.
/// With no measurement and an automatic setting the answer is `Deferred`,
/// because that is the one mode that cannot delay a reply.
pub fn decide(
    measurement: Option<&Measurement>,
    config: &PolicyConfig,
    setting: ModeSetting,
) -> Result<ModeDecision, PerfError> {
    config.validate()?;
    let prediction = match measurement {
        Some(m) => {
            m.validate()?;
            Some(predict(m, config))
        }
        None => None,
    };
    let tier = prediction.map(|p| {
        if p.added_wait_ms <= p.max_added_wait_ms {
            PerformanceTier::Headroom
        } else {
            PerformanceTier::Constrained
        }
    });
    let (mode, source) = match setting {
        ModeSetting::Blocking => (PronMode::Blocking, DecisionSource::Setting),
        ModeSetting::Deferred => (PronMode::Deferred, DecisionSource::Setting),
        ModeSetting::Automatic => match tier {
            Some(PerformanceTier::Headroom) => (PronMode::Blocking, DecisionSource::Measurement),
            Some(PerformanceTier::Constrained) => (PronMode::Deferred, DecisionSource::Measurement),
            None => (PronMode::Deferred, DecisionSource::Unmeasured),
        },
    };
    Ok(ModeDecision {
        mode,
        source,
        tier,
        prediction,
    })
}

impl std::fmt::Display for ModeDecision {
    /// One line for the command line and the diagnostics page.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = match self.mode {
            PronMode::Blocking => "blocking",
            PronMode::Deferred => "deferred",
        };
        write!(f, "{mode}")?;
        match (self.source, &self.prediction) {
            (DecisionSource::Setting, _) => write!(f, " (set by the setting")?,
            (DecisionSource::Unmeasured, _) => {
                return write!(f, " (automatic, not measured yet)");
            }
            (DecisionSource::Measurement, _) => write!(f, " (automatic")?,
        }
        match &self.prediction {
            Some(p) => write!(
                f,
                "; predicted added wait {:.0} ms against {:.0} ms allowed, for an utterance of {:.0} s)",
                p.added_wait_ms, p.max_added_wait_ms, p.reference_utterance_seconds
            ),
            None => write!(f, ")"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values chosen to be exact in binary floating point, so boundary cases
    /// compare exactly: for an 8 s utterance STT takes 1000 ms, the model
    /// 1500 ms and alignment 500 ms, so the added wait is 500 + 500 = 1000 ms.
    fn exact() -> Measurement {
        Measurement {
            stt_alone: 0.0625,
            stt_together: 0.125,
            model_alone: 0.125,
            model_together: 0.1875,
            align: 0.0625,
        }
    }

    fn config(budget: f64) -> PolicyConfig {
        PolicyConfig::new(8.0, budget).expect("valid config")
    }

    #[test]
    fn prediction_follows_the_formula() {
        let d = decide(Some(&exact()), &config(1000.0), ModeSetting::Automatic).expect("decides");
        let p = d.prediction.expect("measured");
        assert_eq!(
            (p.stt_ms, p.model_ms, p.align_ms, p.added_wait_ms),
            (1000.0, 1500.0, 500.0, 1000.0)
        );
    }

    #[test]
    fn automatic_policy_table() {
        struct Case {
            name: &'static str,
            stt_together: f64,
            model_together: f64,
            align: f64,
            budget: f64,
            expect: PronMode,
            tier: PerformanceTier,
        }
        let cases = [
            Case {
                name: "model finishes inside STT, only alignment adds wait, within budget",
                stt_together: 0.25,
                model_together: 0.125,
                align: 0.0625,
                budget: 500.0,
                expect: PronMode::Blocking,
                tier: PerformanceTier::Headroom,
            },
            Case {
                name: "model finishes inside STT but alignment alone exceeds the budget",
                stt_together: 0.25,
                model_together: 0.125,
                align: 0.0625,
                budget: 499.0,
                expect: PronMode::Deferred,
                tier: PerformanceTier::Constrained,
            },
            Case {
                name: "added wait exactly equal to the budget is allowed",
                stt_together: 0.125,
                model_together: 0.1875,
                align: 0.0625,
                budget: 1000.0,
                expect: PronMode::Blocking,
                tier: PerformanceTier::Headroom,
            },
            Case {
                name: "one millisecond over the budget is deferred",
                stt_together: 0.125,
                model_together: 0.1875,
                align: 0.0625,
                budget: 999.0,
                expect: PronMode::Deferred,
                tier: PerformanceTier::Constrained,
            },
            Case {
                name: "a slow model on a slow machine is deferred",
                stt_together: 0.125,
                model_together: 1.0,
                align: 0.0625,
                budget: 1000.0,
                expect: PronMode::Deferred,
                tier: PerformanceTier::Constrained,
            },
        ];
        for c in cases {
            let m = Measurement {
                stt_together: c.stt_together,
                model_together: c.model_together,
                align: c.align,
                ..exact()
            };
            let d = decide(Some(&m), &config(c.budget), ModeSetting::Automatic).expect(c.name);
            assert_eq!(d.mode, c.expect, "{}", c.name);
            assert_eq!(d.tier, Some(c.tier), "{}", c.name);
            assert_eq!(d.source, DecisionSource::Measurement, "{}", c.name);
        }
    }

    #[test]
    fn the_alone_figures_do_not_decide_the_mode() {
        let mut m = exact();
        m.model_alone = 0.001;
        m.stt_alone = 0.001;
        let slow_together = Measurement {
            model_together: 2.0,
            ..m
        };
        let d = decide(
            Some(&slow_together),
            &config(1000.0),
            ModeSetting::Automatic,
        )
        .expect("ok");
        assert_eq!(d.mode, PronMode::Deferred);
    }

    #[test]
    fn the_setting_overrides_the_measurement_in_both_directions() {
        let slow = Measurement {
            model_together: 1.0,
            ..exact()
        };
        let fast = Measurement {
            model_together: 0.0625,
            align: 0.0078125,
            ..exact()
        };
        let cfg = config(1000.0);

        let forced = decide(Some(&slow), &cfg, ModeSetting::Blocking).expect("ok");
        assert_eq!(
            (forced.mode, forced.source, forced.tier),
            (
                PronMode::Blocking,
                DecisionSource::Setting,
                Some(PerformanceTier::Constrained)
            )
        );
        let held = decide(Some(&fast), &cfg, ModeSetting::Deferred).expect("ok");
        assert_eq!(
            (held.mode, held.source, held.tier),
            (
                PronMode::Deferred,
                DecisionSource::Setting,
                Some(PerformanceTier::Headroom)
            )
        );
        let unmeasured_forced = decide(None, &cfg, ModeSetting::Blocking).expect("ok");
        assert_eq!(unmeasured_forced.mode, PronMode::Blocking);
        assert_eq!(unmeasured_forced.tier, None);
    }

    #[test]
    fn without_a_measurement_automatic_is_deferred_and_says_so() {
        let d = decide(None, &config(1000.0), ModeSetting::Automatic).expect("ok");
        assert_eq!(d.mode, PronMode::Deferred);
        assert_eq!(d.source, DecisionSource::Unmeasured);
        assert_eq!(d.prediction, None);
        assert_eq!(d.to_string(), "deferred (automatic, not measured yet)");
    }

    #[test]
    fn the_decision_reads_as_one_line() {
        let d = decide(Some(&exact()), &config(1000.0), ModeSetting::Automatic).expect("ok");
        assert_eq!(
            d.to_string(),
            "blocking (automatic; predicted added wait 1000 ms against 1000 ms allowed, for an utterance of 8 s)"
        );
        let forced = decide(Some(&exact()), &config(10.0), ModeSetting::Blocking).expect("ok");
        assert!(
            forced
                .to_string()
                .starts_with("blocking (set by the setting;")
        );
    }

    #[test]
    fn invalid_numbers_are_rejected() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let m = Measurement {
                stt_together: bad,
                ..exact()
            };
            assert!(m.validate().is_err(), "{bad}");
            assert!(decide(Some(&m), &config(1.0), ModeSetting::Automatic).is_err());
        }
        assert!(PolicyConfig::new(0.0, 1.0).is_err());
        assert!(PolicyConfig::new(10.0, -1.0).is_err());
        assert!(PolicyConfig::new(10.0, f64::NAN).is_err());
        assert!(PolicyConfig::new(10.0, 0.0).is_ok());
    }

    #[test]
    fn headroom_is_the_gap_to_the_target_and_never_negative() {
        assert_eq!(headroom_ms(3000.0, 2550.0), 450.0);
        assert_eq!(headroom_ms(3000.0, 3250.0), 0.0);
    }

    #[test]
    fn contention_is_together_over_alone() {
        assert_eq!(exact().contention(), (2.0, 1.5));
    }

    #[test]
    fn settings_parse_from_their_text_form() {
        for (text, expected) in [
            ("\"automatic\"", ModeSetting::Automatic),
            ("\"blocking\"", ModeSetting::Blocking),
            ("\"deferred\"", ModeSetting::Deferred),
        ] {
            assert_eq!(
                serde_json::from_str::<ModeSetting>(text).expect("parses"),
                expected
            );
        }
        assert!(serde_json::from_str::<ModeSetting>("\"fast\"").is_err());
        assert_eq!(ModeSetting::default(), ModeSetting::Automatic);
    }

    fn sleeper(ms: u64) -> impl FnMut(&CancelFlag) -> Result<(), String> + Send {
        move |_| {
            std::thread::sleep(Duration::from_millis(ms));
            Ok(())
        }
    }

    #[test]
    fn the_harness_times_alone_and_together_and_round_trips_through_json() {
        let mut stt = sleeper(10);
        let mut model = sleeper(50);
        let mut align = sleeper(5);
        let profile = measure(
            Workloads {
                stt: &mut stt,
                model: &mut model,
                align: &mut align,
            },
            2.0,
            3,
            "TEST",
            &CancelFlag::new(),
        )
        .expect("measures");
        let m = profile.measurement;
        // A sleep never returns early, so each figure is at least its nominal
        // sleep divided by the 2 s fixture. Upper bounds are deliberately not
        // asserted: a loaded machine may be arbitrarily slower.
        assert!(m.stt_alone >= 0.005 && m.stt_together >= 0.005, "{m:?}");
        assert!(m.model_alone >= 0.025 && m.model_together >= 0.025, "{m:?}");
        assert!(m.align >= 0.0025, "{m:?}");
        assert_eq!((profile.warmup_runs, profile.timed_runs), (1, 3));
        assert_eq!(profile.label, "TEST");
        assert_eq!(profile.fixture_audio_seconds, 2.0);

        let again = SpeedProfile::from_json(&profile.to_json().expect("json")).expect("parses");
        assert_eq!(again, profile);
    }

    /// A workload whose first `free_calls` calls return at once and whose later
    /// calls wait until the other workload has also started. It can only
    /// succeed if the two really run at the same time.
    fn meets(
        mine: std::sync::Arc<std::sync::atomic::AtomicBool>,
        theirs: std::sync::Arc<std::sync::atomic::AtomicBool>,
        free_calls: usize,
    ) -> impl FnMut(&CancelFlag) -> Result<(), String> + Send {
        use std::sync::atomic::Ordering::SeqCst;
        let mut calls = 0usize;
        move |_| {
            calls += 1;
            if calls <= free_calls {
                return Ok(());
            }
            mine.store(true, SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !theirs.load(SeqCst) {
                if Instant::now() > deadline {
                    return Err("the other workload never ran at the same time".to_owned());
                }
                std::thread::yield_now();
            }
            Ok(())
        }
    }

    #[test]
    fn the_harness_runs_the_together_pair_concurrently() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let stt_started = Arc::new(AtomicBool::new(false));
        let model_started = Arc::new(AtomicBool::new(false));
        // One warm-up run and one alone run come first, then the together run.
        let mut stt = meets(stt_started.clone(), model_started.clone(), 2);
        let mut model = meets(model_started, stt_started, 2);
        let mut align = sleeper(1);
        let profile = measure(
            Workloads {
                stt: &mut stt,
                model: &mut model,
                align: &mut align,
            },
            1.0,
            1,
            "TEST",
            &CancelFlag::new(),
        );
        assert!(profile.is_ok(), "{profile:?}");
    }

    #[test]
    fn the_harness_honours_cancellation_and_reports_failures() {
        let cancel = CancelFlag::new();
        cancel.cancel();
        let mut a = sleeper(1);
        let mut b = sleeper(1);
        let mut c = sleeper(1);
        let r = measure(
            Workloads {
                stt: &mut a,
                model: &mut b,
                align: &mut c,
            },
            1.0,
            1,
            "T",
            &cancel,
        );
        assert!(matches!(r, Err(PerfError::Cancelled)));

        let mut failing =
            |_: &CancelFlag| -> Result<(), String> { Err("model missing".to_owned()) };
        let mut b = sleeper(1);
        let mut c = sleeper(1);
        let r = measure(
            Workloads {
                stt: &mut failing,
                model: &mut b,
                align: &mut c,
            },
            1.0,
            1,
            "T",
            &CancelFlag::new(),
        );
        assert!(matches!(r, Err(PerfError::Workload(m)) if m == "model missing"));

        let (mut a, mut b, mut c) = (sleeper(1), sleeper(1), sleeper(1));
        let none = measure(
            Workloads {
                stt: &mut a,
                model: &mut b,
                align: &mut c,
            },
            1.0,
            0,
            "T",
            &CancelFlag::new(),
        );
        assert!(matches!(none, Err(PerfError::NoRuns)));
        let bad_len = measure(
            Workloads {
                stt: &mut a,
                model: &mut b,
                align: &mut c,
            },
            0.0,
            1,
            "T",
            &CancelFlag::new(),
        );
        assert!(matches!(bad_len, Err(PerfError::NotPositive { .. })));
    }

    #[test]
    fn a_profile_saves_and_loads_from_a_file() {
        let dir = std::env::temp_dir().join(format!("pron-engine-perf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("speed.json");
        let profile = SpeedProfile {
            measurement: exact(),
            fixture_audio_seconds: 4.0,
            warmup_runs: 1,
            timed_runs: 5,
            label: "DEV".to_owned(),
            measured_at_unix: Some(1),
        };
        profile.save(&path).expect("saves");
        assert_eq!(SpeedProfile::load(&path).expect("loads"), profile);
        std::fs::write(&path, "{\"measurement\":{}}").expect("writes");
        assert!(matches!(SpeedProfile::load(&path), Err(PerfError::Json(_))));
        std::fs::remove_dir_all(&dir).expect("cleans up");
        assert!(matches!(
            SpeedProfile::load(&path),
            Err(PerfError::Io { .. })
        ));
    }
}
