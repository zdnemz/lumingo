//! Rubric scoring from a validated model output (ASSESSMENT_SPEC section 5): the
//! cross-checks X1 to X7, the combination of one or two runs, and the confidence
//! table of 5.4. Pure: the model call itself happens elsewhere, and this module
//! only judges what came back. The model never states a level; a band is the
//! most it can say, and levels come from `estimate`.

use crate::{
    Attempt, Level, Origin, Scorer, Status,
    text_metrics::{VocabularyProfile, tokenize, word_count},
};
use serde::Deserialize;
use std::collections::BTreeMap;

pub const RUBRIC_SCORER_NAME: &str = "rubric_llm";

// Starting values; stage 5 tunes them on the anchor set (ASSESSMENT_SPEC intro).
/// X4: a range band of 4 needs at least this share of words above the unit level...
const RANGE_MIN_SHARE_ABOVE: f64 = 0.05;
/// ...or at least this many different words.
const RANGE_MIN_DISTINCT: usize = 40;
/// X5: an accuracy band of 4 with at least this many findings per 100 words is doubtful.
const ACCURACY_HIGH_FINDINGS_PER_100: f64 = 8.0;
/// X5: an accuracy band of 1 with no findings is only doubtful for responses this long.
const ACCURACY_MIN_WORDS_FOR_CLEAN_ALARM: usize = 15;
/// X3: fewer covered content points than this share caps task achievement.
const MIN_COVERED_SHARE: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    TaskAchievement,
    Range,
    Accuracy,
    Coherence,
    Interaction,
}

/// The `rubric_score` contract (`contracts/rubric_score.schema.json`), after schema validation.
#[derive(Debug, Clone, Deserialize)]
pub struct RubricOutput {
    pub dimension_scores: Vec<DimensionScore>,
    pub content_points: Vec<ContentPoint>,
    pub on_task: bool,
    pub feedback_en: String,
    pub feedback_l1: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DimensionScore {
    pub dimension: Dimension,
    /// The contract makes bands strings so every provider can produce them.
    pub band: String,
    pub evidence_quotes: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContentPoint {
    pub point: String,
    pub covered: bool,
    pub quote: String,
}

impl RubricOutput {
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// What the task and the response look like, for the cross-checks.
pub struct TaskContext<'a> {
    pub response: &'a str,
    pub min_words: Option<usize>,
    /// The dimensions the rubric lists. Others in the output are ignored.
    pub dimensions: &'a [Dimension],
    pub unit_level: Level,
    /// A speech transcript: spelling findings are not held against the learner.
    pub voice: bool,
    /// From `text_metrics::vocabulary_profile`, when a word list is available.
    pub vocabulary: Option<&'a VocabularyProfile>,
    /// From `text_metrics::GrammarChecker`, per 100 running words.
    pub findings_per_100_words: Option<f64>,
}

/// One run after the cross-checks.
#[derive(Debug, Clone, PartialEq)]
pub enum RunScore {
    Scored(ScoredRun),
    /// X1: a band above 0 had no valid quote, or the output was incomplete. The
    /// caller reruns once; a second rejection ends as `needs_review`.
    Rejected(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredRun {
    pub bands: BTreeMap<Dimension, u8>,
    /// Quotes that are really in the response, per dimension (X1 removes the rest).
    pub quotes: BTreeMap<Dimension, Vec<String>>,
    pub off_task: bool,
    /// X4 or X5 raised an alarm. These change confidence only.
    pub alarm: bool,
    pub feedback_en: String,
    pub feedback_l1: String,
}

/// Whether `quote` appears in `response`, ignoring case, punctuation and spacing,
/// on whole words only.
fn quote_in_response(quote: &str, response: &str) -> bool {
    let q = tokenize(quote);
    if q.is_empty() {
        return false;
    }
    let haystack = format!(" {} ", tokenize(response).join(" "));
    haystack.contains(&format!(" {} ", q.join(" ")))
}

fn parse_band(band: &str) -> Option<u8> {
    band.trim().parse::<u8>().ok().filter(|b| *b <= 4)
}

/// Applies X1 to X5 and X7 to one run.
pub fn check_run(output: &RubricOutput, task: &TaskContext) -> RunScore {
    let mut bands = BTreeMap::new();
    let mut quotes = BTreeMap::new();
    for d in &output.dimension_scores {
        if !task.dimensions.contains(&d.dimension) {
            continue; // "Do not add dimensions": extras are dropped, not trusted
        }
        let Some(band) = parse_band(&d.band) else {
            return RunScore::Rejected("a band is not 0 to 4");
        };
        let valid: Vec<String> = d
            .evidence_quotes
            .iter()
            .filter(|q| quote_in_response(q, task.response))
            .cloned()
            .collect();
        if band > 0 && valid.is_empty() {
            return RunScore::Rejected("a band above 0 has no valid evidence quote");
        }
        if bands.insert(d.dimension, band).is_some() {
            return RunScore::Rejected("a dimension appears twice");
        }
        quotes.insert(d.dimension, valid);
    }
    if task.dimensions.iter().any(|d| !bands.contains_key(d)) {
        return RunScore::Rejected("a rubric dimension is missing");
    }

    let words = word_count(task.response);
    let mut alarm = false;
    if output.on_task {
        // X2
        let short = task.min_words.is_some_and(|m| words < m);
        // X3: a content point only counts when its quote is real
        let covered = output
            .content_points
            .iter()
            .filter(|p| p.covered && quote_in_response(&p.quote, task.response))
            .count();
        let thin = !output.content_points.is_empty()
            && (covered as f64) < MIN_COVERED_SHARE * output.content_points.len() as f64;
        if short || thin {
            if let Some(b) = bands.get_mut(&Dimension::TaskAchievement) {
                *b = (*b).min(2);
            }
        }
        // X4
        if bands.get(&Dimension::Range) == Some(&4) {
            alarm |= task.vocabulary.is_some_and(|v| {
                v.share_above(task.unit_level) < RANGE_MIN_SHARE_ABOVE
                    && v.distinct < RANGE_MIN_DISTINCT
            });
        }
        // X5
        if let Some(rate) = task.findings_per_100_words {
            let accuracy = bands.get(&Dimension::Accuracy).copied();
            alarm |= accuracy == Some(4) && rate >= ACCURACY_HIGH_FINDINGS_PER_100;
            alarm |=
                accuracy == Some(1) && rate == 0.0 && words >= ACCURACY_MIN_WORDS_FOR_CLEAN_ALARM;
        }
    } else {
        // X7: off task means zero everywhere
        bands.values_mut().for_each(|b| *b = 0);
    }
    RunScore::Scored(ScoredRun {
        bands,
        quotes,
        off_task: !output.on_task,
        alarm,
        feedback_en: output.feedback_en.clone(),
        feedback_l1: output.feedback_l1.clone(),
    })
}

/// Facts about how the runs were obtained, for the confidence table of 5.4.
#[derive(Debug, Clone, Copy)]
pub struct RunFacts {
    /// The provider passed scorer qualification (section 6).
    pub qualified: bool,
    /// An output needed a repair, or came from ladder level 4.
    pub weak_output: bool,
    /// Productive task at C1 or C2.
    pub advanced_productive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DimensionResult {
    pub dimension: Dimension,
    /// Mean of the runs' bands.
    pub band: f64,
    pub quotes: Vec<String>,
}

impl DimensionResult {
    pub fn normalized(&self) -> f64 {
        self.band / 4.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RubricResult {
    pub status: Status,
    pub dimensions: Vec<DimensionResult>,
    pub confidence: f64,
    pub off_task: bool,
    pub feedback_en: String,
    pub feedback_l1: String,
}

/// Combines one or two runs. Any rejected run, or runs that differ by more than 1
/// band on a dimension (X6), give `needs_review`.
pub fn finalize(runs: &[RunScore], facts: RunFacts) -> RubricResult {
    let review = |reason_runs: &[ScoredRun]| RubricResult {
        status: Status::NeedsReview,
        dimensions: Vec::new(),
        confidence: 0.0,
        off_task: reason_runs.first().is_some_and(|r| r.off_task),
        feedback_en: reason_runs
            .first()
            .map(|r| r.feedback_en.clone())
            .unwrap_or_default(),
        feedback_l1: reason_runs
            .first()
            .map(|r| r.feedback_l1.clone())
            .unwrap_or_default(),
    };
    let scored: Vec<ScoredRun> = runs
        .iter()
        .filter_map(|r| {
            if let RunScore::Scored(s) = r {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    if scored.is_empty() || scored.len() != runs.len() {
        return review(&scored);
    }
    let first = &scored[0];
    let mut dimensions = Vec::new();
    let mut agreed = scored.len() == 2;
    for (&dimension, &band) in &first.bands {
        let all: Vec<u8> = scored
            .iter()
            .filter_map(|r| r.bands.get(&dimension).copied())
            .collect();
        let (lo, hi) = (
            all.iter().min().copied().unwrap_or(band),
            all.iter().max().copied().unwrap_or(band),
        );
        if hi - lo > 1 {
            return review(&scored);
        }
        agreed &= lo == hi;
        let mut quotes: Vec<String> = scored
            .iter()
            .flat_map(|r| r.quotes.get(&dimension).cloned().unwrap_or_default())
            .collect();
        quotes.dedup();
        dimensions.push(DimensionResult {
            dimension,
            band: all.iter().map(|b| f64::from(*b)).sum::<f64>() / all.len() as f64,
            quotes,
        });
    }
    let mut confidence: f64 = if facts.qualified { 0.8 } else { 0.5 };
    if scored.iter().any(|r| r.alarm) {
        confidence -= 0.2;
    }
    if facts.weak_output {
        confidence -= 0.2;
    }
    if agreed {
        confidence += 0.1;
    }
    if facts.advanced_productive {
        confidence = confidence.min(0.6);
    }
    RubricResult {
        status: Status::Scored,
        dimensions,
        confidence: confidence.clamp(0.1, 0.9),
        off_task: first.off_task,
        feedback_en: first.feedback_en.clone(),
        feedback_l1: first.feedback_l1.clone(),
    }
}

/// Where the attempt rows belong.
#[derive(Debug, Clone, Copy)]
pub struct AttemptBase {
    pub response_id: u64,
    pub skill: crate::Skill,
    pub level: Level,
    pub activity_id: u64,
    pub session_id: u64,
    pub origin: Origin,
    pub created_at: i64,
}

impl RubricResult {
    /// One attempt per dimension, all sharing the response id, as section 3 requires.
    /// A `needs_review` result stores no per-dimension rows; the caller keeps the
    /// response for a human or a later rerun. Attempts under the confidence floor
    /// are stored by the caller with `counts_toward_estimate` false.
    pub fn attempts(&self, base: &AttemptBase) -> Vec<Attempt> {
        self.dimensions
            .iter()
            .map(|d| Attempt {
                response_id: base.response_id,
                skill: base.skill,
                level: base.level,
                activity_id: base.activity_id,
                session_id: base.session_id,
                scorer: Scorer::RubricLlm,
                normalized: d.normalized(),
                confidence: self.confidence,
                status: self.status,
                origin: base.origin,
                counts_toward_estimate: base.origin == Origin::Authored && self.confidence >= 0.3,
                created_at: base.created_at,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
