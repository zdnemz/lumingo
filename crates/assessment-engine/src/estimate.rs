//! Level estimation, algorithm `est/1` (assessment spec, section 9).
//!
//! A pure function of the attempts, so an estimate can be recomputed at any time
//! and traced back to the rows behind it. Nothing here reads a model's opinion.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::deterministic::SUCCESS_THRESHOLD;
use crate::types::{Attempt, AttemptStatus, Level, Origin, Skill};

pub const ALGORITHM_VERSION: &str = "est/1";

/// z for the Wilson interval (one-sided 90 percent).
const WILSON_Z: f64 = 1.28;
/// Observations older than this are ignored.
const MAX_AGE_SECONDS: i64 = 180 * 24 * 60 * 60;
/// At most this many of the most recent observations count per skill and level.
const MAX_OBSERVATIONS: usize = 40;
const MIN_CONFIDENCE: f64 = 0.3;
const MIN_WEIGHT: f64 = 8.0;
const MIN_ACTIVITIES: usize = 3;
const MIN_SESSIONS: usize = 2;
const MIN_LOWER_BOUND: f64 = 0.6;
const MIN_PRODUCTIVE: usize = 3;
const CAP_UPPER_PRODUCTIVE: f64 = 0.6;
const CAP_PLACEMENT_ONLY: f64 = 0.5;

/// Lower bound of the Wilson score interval for a proportion `p` over `n`
/// observations of total weight.
pub fn wilson_lower_bound(p: f64, n: f64) -> f64 {
    if n <= 0.0 {
        return 0.0;
    }
    let z2 = WILSON_Z * WILSON_Z;
    let centre = p + z2 / (2.0 * n);
    let margin = WILSON_Z * ((p * (1.0 - p) + z2 / (4.0 * n)) / n).sqrt();
    ((centre - margin) / (1.0 + z2 / n)).max(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// Below 0.6 is low, 0.6 to 0.75 medium, above 0.75 high.
pub fn confidence_band(value: f64) -> Confidence {
    if value < 0.6 {
        Confidence::Low
    } else if value <= 0.75 {
        Confidence::Medium
    } else {
        Confidence::High
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimateStatus {
    /// The top level is secured by real attempts.
    Estimated,
    /// The level rests on the placement test alone.
    PlacementOnly,
    /// A1 has enough observations but is not yet secured.
    WorkingTowardsA1,
    /// Not even A1 can be decided.
    InsufficientEvidence,
}

/// The estimate for one skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillEstimate {
    pub skill: Skill,
    pub status: EstimateStatus,
    /// Present only for `Estimated` and `PlacementOnly`.
    pub level: Option<Level>,
    /// 0 when there is no level.
    pub confidence: f64,
    pub band: Option<Confidence>,
    /// Observations behind the estimated level, and the sessions they came from.
    pub scored_tasks: usize,
    pub sessions: usize,
    /// For `WorkingTowardsA1` and `InsufficientEvidence`: how many more tasks would help.
    pub more_tasks_needed: Option<usize>,
    pub algorithm: String,
}

/// What is known about one skill at one level.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelEvidence {
    /// Total weight of the observations.
    pub weight: f64,
    pub observations: usize,
    pub proportion: f64,
    pub lower_bound: f64,
    pub activities: usize,
    pub sessions: usize,
    pub productive: usize,
    pub secured: bool,
}

/// One response, however many dimensions it was scored on.
#[derive(Debug, Clone)]
struct Observation {
    skill: Skill,
    level: Level,
    score: f64,
    weight: f64,
    activity_id: String,
    session_id: String,
    created_at: i64,
    productive: bool,
}

fn eligible(attempt: &Attempt, now: i64) -> bool {
    attempt.status == AttemptStatus::Scored
        && attempt.origin == Origin::Authored
        && attempt.counts_toward_estimate
        && attempt.confidence >= MIN_CONFIDENCE
        && now.saturating_sub(attempt.created_at) <= MAX_AGE_SECONDS
}

/// Groups attempts into observations. Attempts that share a `response_id` are
/// one observation, whose score is the mean and whose weight is the lowest
/// confidence. If any attempt in a response is not eligible, the whole response
/// is left out, so two essays scored on four dimensions never look like eight
/// pieces of evidence.
fn observations(attempts: &[Attempt], now: i64) -> Vec<Observation> {
    let mut grouped: BTreeMap<&str, Vec<&Attempt>> = BTreeMap::new();
    let mut singles: Vec<&Attempt> = Vec::new();
    for attempt in attempts {
        match &attempt.response_id {
            Some(id) => grouped.entry(id.as_str()).or_default().push(attempt),
            None => singles.push(attempt),
        }
    }
    let mut out = Vec::new();
    for attempt in singles {
        if eligible(attempt, now) {
            out.push(Observation {
                skill: attempt.skill,
                level: attempt.level,
                score: attempt.normalized,
                weight: 1.0,
                activity_id: attempt.activity_id.clone(),
                session_id: attempt.session_id.clone(),
                created_at: attempt.created_at,
                productive: attempt.productive_response,
            });
        }
    }
    for rows in grouped.values() {
        let Some(first) = rows.first() else { continue };
        if !rows.iter().all(|row| eligible(row, now)) {
            continue;
        }
        let count = rows.len() as f64;
        out.push(Observation {
            skill: first.skill,
            level: first.level,
            score: rows.iter().map(|row| row.normalized).sum::<f64>() / count,
            weight: rows.iter().map(|row| row.confidence).fold(1.0, f64::min),
            activity_id: first.activity_id.clone(),
            session_id: first.session_id.clone(),
            created_at: rows
                .iter()
                .map(|row| row.created_at)
                .max()
                .unwrap_or(first.created_at),
            productive: rows.iter().any(|row| row.productive_response),
        });
    }
    out
}

fn evidence_for(skill: Skill, level: Level, all: &[Observation]) -> LevelEvidence {
    let mut selected: Vec<&Observation> = all
        .iter()
        .filter(|o| o.skill == skill && o.level == level)
        .collect();
    selected.sort_by_key(|o| std::cmp::Reverse(o.created_at));
    selected.truncate(MAX_OBSERVATIONS);

    let weight: f64 = selected.iter().map(|o| o.weight).sum();
    let successes: f64 = selected
        .iter()
        .filter(|o| o.score >= SUCCESS_THRESHOLD)
        .map(|o| o.weight)
        .sum();
    let proportion = if weight > 0.0 {
        successes / weight
    } else {
        0.0
    };
    let lower_bound = wilson_lower_bound(proportion, weight);
    let activities: BTreeSet<&str> = selected.iter().map(|o| o.activity_id.as_str()).collect();
    let sessions: BTreeSet<&str> = selected.iter().map(|o| o.session_id.as_str()).collect();
    let productive = selected.iter().filter(|o| o.productive).count();

    let secured = weight >= MIN_WEIGHT
        && activities.len() >= MIN_ACTIVITIES
        && sessions.len() >= MIN_SESSIONS
        && lower_bound >= MIN_LOWER_BOUND
        && (!skill.is_productive() || productive >= MIN_PRODUCTIVE);
    LevelEvidence {
        weight,
        observations: selected.len(),
        proportion,
        lower_bound,
        activities: activities.len(),
        sessions: sessions.len(),
        productive,
        secured,
    }
}

/// Estimates one skill.
///
/// `placed_out_through` is the highest level the placement test cleared for this
/// skill, if any. Levels up to and including it count as placed out. `now` is
/// Unix seconds, passed in so the function stays pure.
pub fn estimate_skill(
    attempts: &[Attempt],
    skill: Skill,
    placed_out_through: Option<Level>,
    now: i64,
) -> SkillEstimate {
    let all = observations(attempts, now);
    let evidence: Vec<(Level, LevelEvidence)> = Level::ALL
        .iter()
        .map(|&level| (level, evidence_for(skill, level, &all)))
        .collect();

    let mut top: Option<(Level, &LevelEvidence)> = None;
    for (level, ev) in &evidence {
        let placed = placed_out_through.is_some_and(|through| *level <= through);
        if ev.secured || placed {
            top = Some((*level, ev));
        } else {
            break;
        }
    }

    let make = |status, level: Option<Level>, confidence: f64, ev: Option<&LevelEvidence>, more| {
        SkillEstimate {
            skill,
            status,
            level,
            confidence,
            band: level.map(|_| confidence_band(confidence)),
            scored_tasks: ev.map_or(0, |e| e.observations),
            sessions: ev.map_or(0, |e| e.sessions),
            more_tasks_needed: more,
            algorithm: ALGORITHM_VERSION.to_owned(),
        }
    };

    match top {
        Some((level, ev)) => {
            let mut confidence = ev.lower_bound;
            let placement_only = !ev.secured;
            if skill.is_productive() && level >= Level::C1 {
                confidence = confidence.min(CAP_UPPER_PRODUCTIVE);
            }
            if placement_only {
                confidence = confidence.min(CAP_PLACEMENT_ONLY);
            }
            let status = if placement_only {
                EstimateStatus::PlacementOnly
            } else {
                EstimateStatus::Estimated
            };
            make(status, Some(level), confidence, Some(ev), None)
        }
        None => {
            let a1 = &evidence[0].1;
            let missing = (MIN_WEIGHT - a1.weight).max(0.0).ceil() as usize;
            if a1.weight >= MIN_WEIGHT {
                make(
                    EstimateStatus::WorkingTowardsA1,
                    None,
                    0.0,
                    Some(a1),
                    Some(1),
                )
            } else {
                make(
                    EstimateStatus::InsufficientEvidence,
                    None,
                    0.0,
                    Some(a1),
                    Some(missing.max(1)),
                )
            }
        }
    }
}

/// The four skills together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub skills: Vec<SkillEstimate>,
    /// The lowest of the four levels, shown only when all four are `Estimated`.
    pub working_level: Option<Level>,
}

pub fn estimate_profile(
    attempts: &[Attempt],
    placed_out_through: &BTreeMap<Skill, Level>,
    now: i64,
) -> Profile {
    let skills: Vec<SkillEstimate> = Skill::ALL
        .iter()
        .map(|&skill| {
            estimate_skill(
                attempts,
                skill,
                placed_out_through.get(&skill).copied(),
                now,
            )
        })
        .collect();
    let all_estimated = skills.iter().all(|s| s.status == EstimateStatus::Estimated);
    let working_level = if all_estimated {
        skills.iter().filter_map(|s| s.level).min()
    } else {
        None
    };
    Profile {
        skills,
        working_level,
    }
}
