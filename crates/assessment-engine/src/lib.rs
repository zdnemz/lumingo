//! Level estimation `est/1` (ASSESSMENT_SPEC section 9). A pure function of the
//! attempts, so estimates can be recomputed at any time. No model output enters
//! here: a level only ever comes from stored, locally scored attempts.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashSet};

pub const ALGORITHM_VERSION: &str = "est/1";

const MIN_CONFIDENCE: f64 = 0.3;
const MAX_AGE_SECONDS: i64 = 180 * 86_400;
const MAX_OBSERVATIONS: usize = 40;
const SUCCESS_SCORE: f64 = 0.7;
const Z: f64 = 1.28;
const MIN_WEIGHT_SUM: f64 = 8.0;
const MIN_ACTIVITIES: usize = 3;
const MIN_SESSIONS: usize = 2;
const MIN_LOWER_BOUND: f64 = 0.6;
const MIN_PRODUCTIVE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    A1,
    A2,
    B1,
    B2,
    C1,
    C2,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::A1,
        Level::A2,
        Level::B1,
        Level::B2,
        Level::C1,
        Level::C2,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Skill {
    Listening,
    Reading,
    Speaking,
    Writing,
}

impl Skill {
    pub const ALL: [Skill; 4] = [
        Skill::Listening,
        Skill::Reading,
        Skill::Speaking,
        Skill::Writing,
    ];

    fn is_productive(self) -> bool {
        matches!(self, Skill::Speaking | Skill::Writing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scorer {
    Deterministic,
    RubricLlm,
    PronEngine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Scored,
    PendingLlm,
    NeedsReview,
    Insufficient,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Authored,
    Generated,
    FreeMode,
}

/// One scored dimension of one response, as stored in `assessment_attempts`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt {
    /// Rows of the same response share this id and form one observation.
    pub response_id: u64,
    pub skill: Skill,
    pub level: Level,
    pub activity_id: u64,
    pub session_id: u64,
    pub scorer: Scorer,
    pub normalized: f64,
    pub confidence: f64,
    pub status: Status,
    pub origin: Origin,
    pub counts_toward_estimate: bool,
    /// Unix seconds.
    pub created_at: i64,
}

impl Attempt {
    fn eligible(&self, now: i64) -> bool {
        self.status == Status::Scored
            && self.origin == Origin::Authored
            && self.counts_toward_estimate
            && self.confidence >= MIN_CONFIDENCE
            && now.saturating_sub(self.created_at) <= MAX_AGE_SECONDS
    }
}

struct Observation {
    score: f64,
    weight: f64,
    activity_id: u64,
    session_id: u64,
    rubric_scored: bool,
    created_at: i64,
}

/// Lower bound of the Wilson score interval for proportion `p` over `n` trials.
pub fn wilson_lower_bound(p: f64, n: f64) -> f64 {
    if n <= 0.0 {
        return 0.0;
    }
    let z2 = Z * Z;
    let centre = p + z2 / (2.0 * n);
    let spread = Z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    ((centre - spread) / (1.0 + z2 / n)).max(0.0)
}

/// The numbers behind one (skill, level) cell. These are the history rows.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelRow {
    pub skill: Skill,
    pub level: Level,
    pub n: f64,
    pub p: f64,
    pub lower_bound: f64,
    pub secured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateStatus {
    Estimated,
    /// The top level rests on the placement test alone.
    PlacementOnly,
    InsufficientEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceBand {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub skill: Skill,
    pub level: Option<Level>,
    pub status: EstimateStatus,
    pub confidence: Option<f64>,
    /// A1 has enough observations but is not secured yet.
    pub working_towards_a1: bool,
}

impl Estimate {
    pub fn band(&self) -> Option<ConfidenceBand> {
        self.confidence.map(|c| match c {
            c if c < 0.6 => ConfidenceBand::Low,
            c if c <= 0.75 => ConfidenceBand::Medium,
            _ => ConfidenceBand::High,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Estimation {
    pub estimates: Vec<Estimate>,
    pub rows: Vec<LevelRow>,
}

impl Estimation {
    /// The lowest of the four skills, shown only when all four are `Estimated` (section 9.4).
    pub fn working_level(&self) -> Option<Level> {
        if self
            .estimates
            .iter()
            .any(|e| e.status != EstimateStatus::Estimated)
        {
            return None;
        }
        self.estimates.iter().filter_map(|e| e.level).min()
    }
}

fn observations(attempts: &[Attempt], now: i64) -> BTreeMap<(Skill, Level), Vec<Observation>> {
    let mut by_response: BTreeMap<u64, Vec<&Attempt>> = BTreeMap::new();
    for a in attempts {
        by_response.entry(a.response_id).or_default().push(a);
    }
    let mut cells: BTreeMap<(Skill, Level), Vec<Observation>> = BTreeMap::new();
    for rows in by_response.values() {
        // One bad row spoils the observation: a half-scored essay is not evidence.
        if !rows.iter().all(|a| a.eligible(now)) {
            continue;
        }
        let first = rows[0];
        // Deterministic items weigh 1.0; otherwise the weakest confidence in the group.
        let weight = rows
            .iter()
            .map(|a| {
                if a.scorer == Scorer::Deterministic {
                    1.0
                } else {
                    a.confidence
                }
            })
            .fold(f64::INFINITY, f64::min);
        cells
            .entry((first.skill, first.level))
            .or_default()
            .push(Observation {
                score: rows.iter().map(|a| a.normalized).sum::<f64>() / rows.len() as f64,
                weight,
                activity_id: first.activity_id,
                session_id: first.session_id,
                rubric_scored: rows.iter().any(|a| a.scorer == Scorer::RubricLlm),
                created_at: rows
                    .iter()
                    .map(|a| a.created_at)
                    .max()
                    .unwrap_or(first.created_at),
            });
    }
    for obs in cells.values_mut() {
        obs.sort_by_key(|o| std::cmp::Reverse(o.created_at));
        obs.truncate(MAX_OBSERVATIONS);
    }
    cells
}

fn row(skill: Skill, level: Level, obs: &[Observation]) -> LevelRow {
    let n: f64 = obs.iter().map(|o| o.weight).sum();
    let p = if n > 0.0 {
        obs.iter()
            .filter(|o| o.score >= SUCCESS_SCORE)
            .map(|o| o.weight)
            .sum::<f64>()
            / n
    } else {
        0.0
    };
    let lower_bound = wilson_lower_bound(p, n);
    let activities: HashSet<_> = obs.iter().map(|o| o.activity_id).collect();
    let sessions: HashSet<_> = obs.iter().map(|o| o.session_id).collect();
    let productive = obs.iter().filter(|o| o.rubric_scored).count();
    let secured = n >= MIN_WEIGHT_SUM
        && activities.len() >= MIN_ACTIVITIES
        && sessions.len() >= MIN_SESSIONS
        && lower_bound >= MIN_LOWER_BOUND
        && (!skill.is_productive() || productive >= MIN_PRODUCTIVE);
    LevelRow {
        skill,
        level,
        n,
        p,
        lower_bound,
        secured,
    }
}

/// Estimates all four skills. `placed_out` gives, per skill, the highest level the
/// placement test passed over; levels up to it count as secured without attempts.
pub fn estimate(attempts: &[Attempt], placed_out: &BTreeMap<Skill, Level>, now: i64) -> Estimation {
    let cells = observations(attempts, now);
    let mut rows = Vec::new();
    let mut estimates = Vec::new();
    for skill in Skill::ALL {
        let skill_rows: Vec<LevelRow> = Level::ALL
            .iter()
            .map(|&level| {
                row(
                    skill,
                    level,
                    cells.get(&(skill, level)).map_or(&[][..], Vec::as_slice),
                )
            })
            .collect();
        let placed = placed_out.get(&skill).copied();
        let is_placed = |l: Level| placed.is_some_and(|p| l <= p);
        let mut top: Option<Level> = None;
        for r in &skill_rows {
            if r.secured || is_placed(r.level) {
                top = Some(r.level);
            } else {
                break;
            }
        }
        let a1 = &skill_rows[0];
        let estimate = match top {
            None => Estimate {
                skill,
                level: None,
                status: EstimateStatus::InsufficientEvidence,
                confidence: None,
                working_towards_a1: a1.n >= MIN_WEIGHT_SUM,
            },
            Some(level) => {
                let r = &skill_rows[level as usize];
                let placement_only = !r.secured;
                // With no attempts at a placement-only level the lower bound is 0: "low".
                let mut confidence = r.lower_bound;
                if placement_only {
                    confidence = confidence.min(0.5);
                } else if skill.is_productive() && level >= Level::C1 {
                    confidence = confidence.min(0.6);
                }
                Estimate {
                    skill,
                    level: Some(level),
                    status: if placement_only {
                        EstimateStatus::PlacementOnly
                    } else {
                        EstimateStatus::Estimated
                    },
                    confidence: Some(confidence),
                    working_towards_a1: false,
                }
            }
        };
        estimates.push(estimate);
        rows.extend(skill_rows);
    }
    Estimation { estimates, rows }
}

#[cfg(test)]
mod tests;
