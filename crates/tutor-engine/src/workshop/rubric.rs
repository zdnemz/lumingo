//! The rubric score of a writing draft: prompt `rubric_score/1` (T3), the typed
//! reply and the cross-checks X1 to X5 and X7 of `docs/ASSESSMENT_SPEC.md`
//! section 5.3.
//!
//! Everything here is pure. A model never states a level: the schema has no
//! field for one, and feedback prose that names a level or a percentage is
//! discarded.

use assessment_engine::{Level, normalize};
use curriculum::validate::WordLevels;
use llm_client::LadderLevel;
use serde::{Deserialize, Serialize};

use crate::support::{curriculum_level, names_a_level, states_a_percentage, truncate_words};

pub const RUBRIC_SCORE_VERSION: &str = "rubric_score/1";

/// Words in each feedback text, at most.
pub const MAX_FEEDBACK_WORDS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    TaskAchievement,
    Range,
    Accuracy,
    Coherence,
    Interaction,
}

impl Dimension {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskAchievement => "task_achievement",
            Self::Range => "range",
            Self::Accuracy => "accuracy",
            Self::Coherence => "coherence",
            Self::Interaction => "interaction",
        }
    }
}

/// One dimension of an authored rubric, with the description of each band.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RubricDimension {
    pub dimension: Dimension,
    /// Descriptions of bands 0 to 4, in that order, in the project's own words.
    pub bands: [String; 5],
}

/// An authored rubric for a written task (`curriculum/catalogs/rubrics/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkshopRubric {
    pub id: String,
    pub version: u32,
    pub dimensions: Vec<RubricDimension>,
}

/// What the learner was asked to write.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkshopTask {
    /// The prompt in English. Empty when the learner chose their own topic.
    pub prompt: String,
    pub content_points: Vec<String>,
    pub min_words: Option<u32>,
}

#[derive(Debug, Serialize)]
struct BandMessage<'a> {
    band: String,
    description: &'a str,
}

#[derive(Debug, Serialize)]
struct DimensionMessage<'a> {
    dimension: &'a str,
    bands: Vec<BandMessage<'a>>,
}

/// The user message of the rubric call. Field order is the wire order.
#[derive(Debug, Serialize)]
struct RubricMessage<'a> {
    level: &'a str,
    input_mode: &'a str,
    task_prompt: &'a str,
    content_points: &'a [String],
    rubric: Vec<DimensionMessage<'a>>,
    response: &'a str,
}

/// The system prompt, `rubric_score/1`.
pub fn system_prompt(l1_name: &str) -> String {
    format!(
        "You score one response from an English learner against a rubric. Return only JSON that matches the schema.\n\
         \n\
         - Score each dimension listed in the rubric with a band from \"0\" to \"4\", using the band descriptions given. Do not add dimensions.\n\
         - For every band above \"0\", give one to three \"evidence_quotes\" copied exactly from the response. A band without evidence will be rejected.\n\
         - \"reason\" is one sentence.\n\
         - For each content point, say whether the response covers it and quote the words that do.\n\
         - \"on_task\" is false if the response is empty, in another language, or does not attempt the task. In that case give band \"0\" everywhere.\n\
         - If input_mode is \"voice\", the response is a speech transcript: ignore spelling, capital letters, and punctuation.\n\
         - Never state a CEFR level or a percentage.\n\
         - \"feedback_en\": at most 40 words, written so a learner at the stated level can read it. One thing done well, one thing to improve.\n\
         - \"feedback_l1\": the same message in {l1_name}, at most 40 words."
    )
}

/// The user message: one JSON object. The learner's text only travels inside a
/// JSON string.
pub fn user_message(
    level: Level,
    task: &WorkshopTask,
    rubric: &WorkshopRubric,
    response: &str,
) -> String {
    let message = RubricMessage {
        level: level.as_str(),
        input_mode: "text",
        task_prompt: &task.prompt,
        content_points: &task.content_points,
        rubric: rubric
            .dimensions
            .iter()
            .map(|d| DimensionMessage {
                dimension: d.dimension.as_str(),
                bands: d
                    .bands
                    .iter()
                    .enumerate()
                    .map(|(band, description)| BandMessage {
                        band: band.to_string(),
                        description,
                    })
                    .collect(),
            })
            .collect(),
        response,
    };
    serde_json::to_string(&message).unwrap_or_else(|_| "{}".to_owned())
}

/// The typed reply of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawRubric {
    pub dimension_scores: Vec<RawDimension>,
    pub content_points: Vec<RawPoint>,
    pub on_task: bool,
    pub feedback_en: String,
    pub feedback_l1: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawDimension {
    pub dimension: Dimension,
    pub band: String,
    pub evidence_quotes: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawPoint {
    pub point: String,
    pub covered: bool,
    pub quote: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionStatus {
    Scored,
    /// The dimension is missing or its evidence did not hold, twice.
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionResult {
    pub dimension: Dimension,
    /// 0 to 4. `None` for `NeedsReview`.
    pub band: Option<u8>,
    /// The quotes that are in the response.
    pub evidence_quotes: Vec<String>,
    pub reason: String,
    pub status: DimensionStatus,
    /// X2 or X3 lowered the band to 2.
    pub capped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alarm {
    /// X4: a range band of 4 the vocabulary profile does not support.
    Range,
    /// X5: an accuracy band that the rule-based findings contradict.
    Accuracy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointResult {
    pub point: String,
    pub covered: bool,
    pub quote: String,
}

/// A checked rubric score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RubricResult {
    pub dimensions: Vec<DimensionResult>,
    pub content_points: Vec<PointResult>,
    /// False: all bands are 0 and the learner is asked to try again (X7).
    pub on_task: bool,
    pub feedback_en: String,
    pub feedback_l1: String,
    /// 0.1 to 0.9, by section 5.4.
    pub confidence: f64,
    pub alarms: Vec<Alarm>,
    /// Dimensions whose evidence failed X1 and may be run again.
    pub rejected: Vec<Dimension>,
}

/// What the cross-checks need besides the reply.
#[derive(Debug, Clone, Copy)]
pub struct CrossCheck<'a> {
    pub response: &'a str,
    pub level: Level,
    pub task: &'a WorkshopTask,
    pub rubric: &'a WorkshopRubric,
    /// Findings of the rule-based checker for the response.
    pub grammar_findings: usize,
    pub word_levels: Option<&'a WordLevels>,
    /// Whether the provider passed scorer qualification (section 6).
    pub provider_qualified: bool,
    pub repaired: bool,
    pub ladder_level: LadderLevel,
}

/// Distinct words a range band of 4 should show. A starting value.
const RANGE_MIN_DISTINCT_WORDS: usize = 30;
/// Findings per 100 words above which an accuracy band of 4 raises an alarm. A starting value.
const ACCURACY_MAX_RATE: f64 = 5.0;
/// Shortest response, in words, for which the finding rate means anything.
const ACCURACY_MIN_WORDS: usize = 10;

fn words_of(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn valid_quote(response_norm: &str, quote: &str) -> bool {
    let quote = normalize(quote);
    !quote.is_empty() && response_norm.contains(&quote)
}

fn parse_band(text: &str) -> Option<u8> {
    match text.trim() {
        "0" => Some(0),
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        _ => None,
    }
}

fn clean_feedback(text: &str) -> String {
    let cut = truncate_words(text.trim(), MAX_FEEDBACK_WORDS);
    if names_a_level(&cut) || states_a_percentage(&cut) {
        String::new()
    } else {
        cut
    }
}

/// Applies the cross-checks to one run.
///
/// X1: quotes that are not in the response (after normalisation) are removed,
/// and a band above 0 left with no quote is rejected. X2: fewer words than the
/// task's minimum cap `task_achievement` at 2. X3: a content point counts as
/// covered only with a valid quote, and fewer than half covered cap
/// `task_achievement` at 2. X7: `on_task` false makes every band 0. X4 and X5
/// only lower the confidence.
pub fn cross_check(raw: &RawRubric, input: &CrossCheck<'_>) -> RubricResult {
    let response_norm = normalize(input.response);
    let words = words_of(input.response);

    // X3, on the task's own list of points.
    let points: Vec<PointResult> = raw
        .content_points
        .iter()
        .map(|p| PointResult {
            point: p.point.clone(),
            covered: p.covered && valid_quote(&response_norm, &p.quote),
            quote: if valid_quote(&response_norm, &p.quote) {
                p.quote.clone()
            } else {
                String::new()
            },
        })
        .collect();
    let wanted = input.task.content_points.len();
    let covered = points.iter().filter(|p| p.covered).count();
    let too_few_points = wanted > 0 && covered * 2 < wanted;
    let too_short = input
        .task
        .min_words
        .is_some_and(|min| words.len() < usize::try_from(min).unwrap_or(usize::MAX));

    let mut dimensions = Vec::new();
    let mut rejected = Vec::new();
    for wanted_dimension in input.rubric.dimensions.iter().map(|d| d.dimension) {
        let reply = raw
            .dimension_scores
            .iter()
            .find(|d| d.dimension == wanted_dimension);
        let Some(reply) = reply else {
            dimensions.push(DimensionResult {
                dimension: wanted_dimension,
                band: None,
                evidence_quotes: Vec::new(),
                reason: String::new(),
                status: DimensionStatus::NeedsReview,
                capped: false,
            });
            continue;
        };
        let quotes: Vec<String> = reply
            .evidence_quotes
            .iter()
            .filter(|q| valid_quote(&response_norm, q))
            .cloned()
            .collect();
        let band = parse_band(&reply.band);
        let result = match band {
            None => {
                rejected.push(wanted_dimension);
                DimensionResult {
                    dimension: wanted_dimension,
                    band: None,
                    evidence_quotes: quotes,
                    reason: reply.reason.clone(),
                    status: DimensionStatus::NeedsReview,
                    capped: false,
                }
            }
            Some(band) if band > 0 && quotes.is_empty() && raw.on_task => {
                rejected.push(wanted_dimension);
                DimensionResult {
                    dimension: wanted_dimension,
                    band: None,
                    evidence_quotes: quotes,
                    reason: reply.reason.clone(),
                    status: DimensionStatus::NeedsReview,
                    capped: false,
                }
            }
            Some(band) => {
                let mut band = band;
                let mut capped = false;
                if wanted_dimension == Dimension::TaskAchievement
                    && (too_short || too_few_points)
                    && band > 2
                {
                    band = 2;
                    capped = true;
                }
                if !raw.on_task {
                    band = 0;
                }
                DimensionResult {
                    dimension: wanted_dimension,
                    band: Some(band),
                    evidence_quotes: quotes,
                    reason: reply.reason.clone(),
                    status: DimensionStatus::Scored,
                    capped,
                }
            }
        };
        dimensions.push(result);
    }

    // X4 and X5: alarms that lower confidence only.
    let band_of = |d: Dimension| {
        dimensions
            .iter()
            .find(|r| r.dimension == d)
            .and_then(|r| r.band)
    };
    let mut alarms = Vec::new();
    if raw.on_task && band_of(Dimension::Range) == Some(4) {
        let distinct: std::collections::HashSet<&String> = words.iter().collect();
        let above = input.word_levels.is_some_and(|levels| {
            words.iter().any(|w| {
                levels
                    .level_of(w)
                    .is_some_and(|l| l > curriculum_level(input.level))
            })
        });
        let no_support =
            distinct.len() < RANGE_MIN_DISTINCT_WORDS || (input.word_levels.is_some() && !above);
        if no_support {
            alarms.push(Alarm::Range);
        }
    }
    if raw.on_task && words.len() >= ACCURACY_MIN_WORDS {
        let rate = input.grammar_findings as f64 * 100.0 / words.len() as f64;
        let accuracy = band_of(Dimension::Accuracy);
        if (accuracy == Some(4) && rate > ACCURACY_MAX_RATE)
            || (accuracy == Some(1) && input.grammar_findings == 0)
        {
            alarms.push(Alarm::Accuracy);
        }
    }

    let mut confidence: f64 = if input.provider_qualified { 0.8 } else { 0.5 };
    if !alarms.is_empty() {
        confidence -= 0.2;
    }
    if input.repaired || input.ladder_level == LadderLevel::PromptOnly {
        confidence -= 0.2;
    }
    if matches!(input.level, Level::C1 | Level::C2) {
        confidence = confidence.min(0.6);
    }
    let confidence = confidence.clamp(0.1, 0.9);

    RubricResult {
        dimensions,
        content_points: points,
        on_task: raw.on_task,
        feedback_en: clean_feedback(&raw.feedback_en),
        feedback_l1: clean_feedback(&raw.feedback_l1),
        confidence,
        alarms,
        rejected,
    }
}

/// Joins two runs of the same response: a dimension rejected in the first run
/// is taken from the second run when that held, and stays `NeedsReview`
/// otherwise. Everything else comes from the first run.
pub fn merge_rerun(first: RubricResult, second: &RubricResult) -> RubricResult {
    let mut merged = first;
    for dimension in &mut merged.dimensions {
        if dimension.status == DimensionStatus::NeedsReview
            && let Some(again) = second
                .dimensions
                .iter()
                .find(|d| d.dimension == dimension.dimension)
            && again.status == DimensionStatus::Scored
        {
            *dimension = again.clone();
        }
    }
    merged.rejected = merged
        .dimensions
        .iter()
        .filter(|d| d.status == DimensionStatus::NeedsReview)
        .map(|d| d.dimension)
        .collect();
    merged
}

#[cfg(test)]
mod tests;
