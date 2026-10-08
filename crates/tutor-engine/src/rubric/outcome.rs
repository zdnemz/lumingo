//! The result of scoring one response in one or two runs, after X6.
//!
//! One run is checked by [`super::cross_check`] into a [`RubricResult`]. A
//! checkpoint or placement task is scored in two runs, and X6 joins them: the
//! bands of a dimension may differ by at most 1 and the mean is used, a larger
//! gap sends the dimension to review. The outcome is what the evidence recorder
//! stores.

use assessment_engine::Level;
use serde::{Deserialize, Serialize};

use super::{
    Alarm, ConfidenceInputs, Dimension, DimensionResult, DimensionStatus, PointResult,
    RubricResult, rubric_confidence,
};

/// Two runs may differ by this many bands in a dimension (X6).
pub const MAX_BAND_GAP: u8 = 1;

/// One dimension of the outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredDimension {
    pub dimension: Dimension,
    /// Mean band over the runs, 0 to 4. `None` when the dimension needs review.
    pub band: Option<f64>,
    /// The band of each run that produced one, in run order.
    pub run_bands: Vec<u8>,
    /// Quotes that are in the response, over all runs, without repeats.
    pub evidence_quotes: Vec<String>,
    /// The first run's one-sentence reason.
    pub reason: String,
    pub status: DimensionStatus,
    /// X2 or X3 lowered the band to 2 in some run.
    pub capped: bool,
    /// The runs differed by more than [`MAX_BAND_GAP`] bands (X6).
    pub runs_disagree: bool,
}

impl ScoredDimension {
    /// The band as a score from 0 to 1, when there is one.
    pub fn normalized(&self) -> Option<f64> {
        self.band.map(|band| band / 4.0)
    }
}

/// One checked run and what its confidence depends on.
#[derive(Debug, Clone)]
pub struct CheckedRun {
    pub result: RubricResult,
    /// The output needed a repair or came from ladder level 4, in this run or in
    /// the rerun that completed it.
    pub repaired: bool,
}

/// A response scored by one or two runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RubricOutcome {
    pub dimensions: Vec<ScoredDimension>,
    pub content_points: Vec<PointResult>,
    /// False when any run found the response off task. Every band of that run is
    /// 0 (X7) and the learner is asked to try again.
    pub on_task: bool,
    pub feedback_en: String,
    pub feedback_l1: String,
    /// 0.1 to 0.9, by section 5.4.
    pub confidence: f64,
    pub alarms: Vec<Alarm>,
    /// How many runs were joined: 1 or 2.
    pub runs: u8,
    /// Two runs gave the same band in every dimension.
    pub runs_agreed: bool,
    pub repaired: bool,
}

impl RubricOutcome {
    /// Joins one or two checked runs. An empty list gives an outcome with no
    /// dimensions, which [`RubricOutcome::is_complete`] reports as incomplete.
    pub fn join(runs: &[CheckedRun], provider_qualified: bool, level: Level) -> Self {
        let Some(first) = runs.first() else {
            return Self {
                dimensions: Vec::new(),
                content_points: Vec::new(),
                on_task: true,
                feedback_en: String::new(),
                feedback_l1: String::new(),
                confidence: rubric_confidence(&ConfidenceInputs {
                    provider_qualified,
                    alarm: false,
                    repaired: false,
                    runs_agreed: false,
                    level,
                }),
                alarms: Vec::new(),
                runs: 0,
                runs_agreed: false,
                repaired: false,
            };
        };
        let dimensions: Vec<ScoredDimension> = first
            .result
            .dimensions
            .iter()
            .map(|d| join_dimension(d.dimension, runs))
            .collect();
        let two_runs = runs.len() >= 2;
        let runs_agreed = two_runs
            && !dimensions.is_empty()
            && dimensions.iter().all(|d| {
                d.status == DimensionStatus::Scored
                    && d.run_bands.len() == runs.len()
                    && d.run_bands.windows(2).all(|pair| pair[0] == pair[1])
            });
        let mut alarms: Vec<Alarm> = Vec::new();
        for alarm in runs.iter().flat_map(|r| r.result.alarms.iter()) {
            if !alarms.contains(alarm) {
                alarms.push(*alarm);
            }
        }
        let repaired = runs.iter().any(|r| r.repaired);
        let confidence = rubric_confidence(&ConfidenceInputs {
            provider_qualified,
            alarm: !alarms.is_empty(),
            repaired,
            runs_agreed,
            level,
        });
        Self {
            dimensions,
            content_points: first.result.content_points.clone(),
            on_task: runs.iter().all(|r| r.result.on_task),
            feedback_en: runs
                .iter()
                .map(|r| r.result.feedback_en.as_str())
                .find(|text| !text.is_empty())
                .unwrap_or_default()
                .to_owned(),
            feedback_l1: runs
                .iter()
                .map(|r| r.result.feedback_l1.as_str())
                .find(|text| !text.is_empty())
                .unwrap_or_default()
                .to_owned(),
            confidence,
            alarms,
            runs: u8::try_from(runs.len()).unwrap_or(u8::MAX),
            runs_agreed,
            repaired,
        }
    }

    /// Every dimension has a band. A response with a dimension in review is not
    /// scored: its attempts get the status `needs_review`.
    pub fn is_complete(&self) -> bool {
        !self.dimensions.is_empty()
            && self
                .dimensions
                .iter()
                .all(|d| d.status == DimensionStatus::Scored && d.band.is_some())
    }

    /// The learner should try again: the response did not attempt the task.
    pub fn try_again(&self) -> bool {
        !self.on_task
    }

    /// Mean of the dimension scores, 0 to 1, when the outcome is complete.
    pub fn mean_normalized(&self) -> Option<f64> {
        if !self.is_complete() {
            return None;
        }
        let scores: Vec<f64> = self
            .dimensions
            .iter()
            .filter_map(ScoredDimension::normalized)
            .collect();
        Some(scores.iter().sum::<f64>() / scores.len() as f64)
    }
}

fn join_dimension(dimension: Dimension, runs: &[CheckedRun]) -> ScoredDimension {
    let results: Vec<Option<&DimensionResult>> = runs
        .iter()
        .map(|run| {
            run.result
                .dimensions
                .iter()
                .find(|d| d.dimension == dimension)
        })
        .collect();
    let mut evidence_quotes: Vec<String> = Vec::new();
    for result in results.iter().flatten() {
        for quote in &result.evidence_quotes {
            if !evidence_quotes.contains(quote) {
                evidence_quotes.push(quote.clone());
            }
        }
    }
    let reason = results
        .iter()
        .flatten()
        .map(|d| d.reason.as_str())
        .find(|r| !r.trim().is_empty())
        .unwrap_or_default()
        .to_owned();
    let capped = results.iter().flatten().any(|d| d.capped);
    // A run that has no band for the dimension leaves it for review.
    let bands: Vec<u8> = results
        .iter()
        .filter_map(|r| {
            r.filter(|d| d.status == DimensionStatus::Scored)
                .and_then(|d| d.band)
        })
        .collect();
    let all_scored = bands.len() == runs.len();
    let spread = match (bands.iter().min(), bands.iter().max()) {
        (Some(low), Some(high)) => high - low,
        _ => 0,
    };
    let runs_disagree = all_scored && spread > MAX_BAND_GAP;
    let (band, status) = if all_scored && !runs_disagree {
        let mean = bands.iter().map(|b| f64::from(*b)).sum::<f64>() / bands.len() as f64;
        (Some(mean), DimensionStatus::Scored)
    } else {
        (None, DimensionStatus::NeedsReview)
    };
    ScoredDimension {
        dimension,
        band,
        run_bands: bands,
        evidence_quotes,
        reason,
        status,
        capped,
        runs_disagree,
    }
}
