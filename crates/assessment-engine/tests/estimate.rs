#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;

use assessment_engine::{
    Attempt, AttemptStatus, Confidence, EstimateStatus, Level, Origin, Scorer, Skill,
    confidence_band, estimate_profile, estimate_skill, wilson_lower_bound,
};

type Tweak = Box<dyn Fn(&mut Attempt)>;

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

fn attempt(skill: Skill, level: Level, n: usize, success: bool) -> Attempt {
    Attempt {
        response_id: None,
        skill,
        level,
        // Spread over several activities and two sessions unless a test says otherwise.
        activity_id: format!("act-{}", n % 4),
        session_id: format!("s-{}", n % 2),
        scorer: Scorer::Deterministic,
        normalized: if success { 1.0 } else { 0.0 },
        confidence: 1.0,
        status: AttemptStatus::Scored,
        origin: Origin::Authored,
        counts_toward_estimate: true,
        productive_response: false,
        created_at: NOW - (n as i64) * 60,
    }
}

/// `successes` right answers out of `total`, for objective items.
fn record(skill: Skill, level: Level, successes: usize, total: usize) -> Vec<Attempt> {
    (0..total)
        .map(|n| attempt(skill, level, n, n < successes))
        .collect()
}

fn productive(skill: Skill, level: Level, successes: usize, total: usize) -> Vec<Attempt> {
    (0..total)
        .map(|n| {
            let mut a = attempt(skill, level, n, n < successes);
            a.scorer = Scorer::RubricLlm;
            a.productive_response = true;
            a.response_id = Some(format!("resp-{skill:?}-{level:?}-{n}"));
            a
        })
        .collect()
}

#[test]
fn the_wilson_table_in_the_spec_is_reproduced() {
    // (observations, successes needed, lower bound at that count)
    let table = [
        (8.0, 7.0, 0.661),
        (10.0, 8.0, 0.602),
        (12.0, 10.0, 0.658),
        (16.0, 13.0, 0.661),
        (20.0, 15.0, 0.610),
        (30.0, 22.0, 0.620),
        (40.0, 28.0, 0.601),
    ];
    for (n, successes, expected) in table {
        let lb = wilson_lower_bound(successes / n, n);
        assert!(
            (lb - expected).abs() < 0.002,
            "{successes}/{n}: {lb} vs {expected}"
        );
        assert!(lb >= 0.6, "{successes}/{n} must secure a level");
        let one_fewer = wilson_lower_bound((successes - 1.0) / n, n);
        assert!(
            one_fewer < 0.6,
            "{}/{n} must not secure a level: {one_fewer}",
            successes - 1.0
        );
    }
    // "Six out of eight is not enough (lower bound 0.524)."
    assert!((wilson_lower_bound(6.0 / 8.0, 8.0) - 0.524).abs() < 0.002);
}

#[test]
fn nothing_is_estimated_without_evidence() {
    let estimate = estimate_skill(&[], Skill::Reading, None, NOW);
    assert_eq!(estimate.status, EstimateStatus::InsufficientEvidence);
    assert_eq!(estimate.level, None);
    assert_eq!(estimate.band, None);
    assert_eq!(estimate.more_tasks_needed, Some(8));
}

#[test]
fn the_message_counts_down_the_tasks_still_needed() {
    let attempts = record(Skill::Reading, Level::A1, 3, 5);
    let estimate = estimate_skill(&attempts, Skill::Reading, None, NOW);
    assert_eq!(estimate.status, EstimateStatus::InsufficientEvidence);
    assert_eq!(estimate.more_tasks_needed, Some(3));
}

#[test]
fn seven_of_eight_secures_a1_and_six_of_eight_does_not() {
    let secured = estimate_skill(
        &record(Skill::Reading, Level::A1, 7, 8),
        Skill::Reading,
        None,
        NOW,
    );
    assert_eq!(secured.status, EstimateStatus::Estimated);
    assert_eq!(secured.level, Some(Level::A1));
    assert_eq!(secured.scored_tasks, 8);
    assert_eq!(secured.sessions, 2);

    let not_yet = estimate_skill(
        &record(Skill::Reading, Level::A1, 6, 8),
        Skill::Reading,
        None,
        NOW,
    );
    assert_eq!(not_yet.status, EstimateStatus::WorkingTowardsA1);
    assert_eq!(not_yet.level, None);
}

#[test]
fn a_secured_level_needs_three_activities_and_two_sessions() {
    let mut one_session = record(Skill::Reading, Level::A1, 8, 8);
    for a in &mut one_session {
        a.session_id = "only".into();
    }
    assert_eq!(
        estimate_skill(&one_session, Skill::Reading, None, NOW).status,
        EstimateStatus::WorkingTowardsA1,
        "one session is not enough"
    );

    let mut two_activities = record(Skill::Reading, Level::A1, 8, 8);
    for (n, a) in two_activities.iter_mut().enumerate() {
        a.activity_id = format!("act-{}", n % 2);
    }
    assert_eq!(
        estimate_skill(&two_activities, Skill::Reading, None, NOW).status,
        EstimateStatus::WorkingTowardsA1,
        "two activities are not enough"
    );
}

#[test]
fn the_estimate_is_the_highest_level_with_every_level_below_secured() {
    let mut attempts = record(Skill::Listening, Level::A1, 9, 10);
    attempts.extend(record(Skill::Listening, Level::A2, 9, 10));
    attempts.extend(record(Skill::Listening, Level::B2, 10, 10)); // B1 missing: B2 cannot count
    let estimate = estimate_skill(&attempts, Skill::Listening, None, NOW);
    assert_eq!(estimate.level, Some(Level::A2));
    attempts.extend(record(Skill::Listening, Level::B1, 9, 10));
    assert_eq!(
        estimate_skill(&attempts, Skill::Listening, None, NOW).level,
        Some(Level::B2)
    );
}

#[test]
fn speaking_and_writing_cannot_be_secured_by_objective_items_alone() {
    let objective = record(Skill::Writing, Level::A1, 10, 10);
    assert_eq!(
        estimate_skill(&objective, Skill::Writing, None, NOW).status,
        EstimateStatus::WorkingTowardsA1
    );
    let mut with_productive = objective;
    with_productive.extend(productive(Skill::Writing, Level::A1, 3, 3));
    let estimate = estimate_skill(&with_productive, Skill::Writing, None, NOW);
    assert_eq!(estimate.status, EstimateStatus::Estimated);
    assert_eq!(estimate.level, Some(Level::A1));
}

#[test]
fn one_rubric_response_is_one_observation_however_many_dimensions() {
    // Two essays, each scored on four dimensions: two observations, not eight.
    let mut rows = Vec::new();
    for essay in 0..2 {
        for dimension in 0..4 {
            let mut a = attempt(Skill::Writing, Level::A1, essay, true);
            a.response_id = Some(format!("essay-{essay}"));
            a.scorer = Scorer::RubricLlm;
            a.productive_response = true;
            a.activity_id = format!("w-{essay}-{dimension}");
            rows.push(a);
        }
    }
    let estimate = estimate_skill(&rows, Skill::Writing, None, NOW);
    assert_eq!(estimate.status, EstimateStatus::InsufficientEvidence);
    assert_eq!(
        estimate.more_tasks_needed,
        Some(6),
        "two observations count, six more are needed"
    );
}

#[test]
fn an_observation_takes_the_mean_score_and_the_lowest_confidence() {
    // Eight observations, each a response scored on two dimensions at 0.9 and 0.4: mean 0.65, a failure.
    let mut rows = Vec::new();
    for n in 0..8 {
        for (k, score) in [0.9, 0.4].into_iter().enumerate() {
            let mut a = attempt(Skill::Speaking, Level::A1, n, true);
            a.normalized = score;
            a.response_id = Some(format!("r-{n}"));
            a.productive_response = true;
            a.scorer = Scorer::RubricLlm;
            a.confidence = if k == 0 { 0.9 } else { 0.5 };
            rows.push(a);
        }
    }
    let estimate = estimate_skill(&rows, Skill::Speaking, None, NOW);
    assert_eq!(
        estimate.status,
        EstimateStatus::InsufficientEvidence,
        "the mean 0.65 is below the 0.7 success line"
    );
}

#[test]
fn a_response_with_one_ineligible_dimension_is_left_out_whole() {
    let mut rows = productive(Skill::Speaking, Level::A1, 10, 10);
    rows.extend(record(Skill::Speaking, Level::A1, 10, 10));
    let baseline = estimate_skill(&rows, Skill::Speaking, None, NOW);
    assert_eq!(baseline.status, EstimateStatus::Estimated);

    // Add a second dimension to every productive response, but mark it pending.
    let extra: Vec<Attempt> = rows
        .iter()
        .filter(|a| a.response_id.is_some())
        .map(|a| {
            let mut b = a.clone();
            b.status = AttemptStatus::PendingLlm;
            b
        })
        .collect();
    rows.extend(extra);
    let after = estimate_skill(&rows, Skill::Speaking, None, NOW);
    assert_ne!(
        after.status,
        EstimateStatus::Estimated,
        "the productive observations are gone, so speaking is not secured"
    );
}

#[test]
fn ineligible_attempts_never_count() {
    let base = record(Skill::Reading, Level::A1, 8, 8);
    let tweaks: Vec<(&str, Tweak)> = vec![
        ("generated", Box::new(|a| a.origin = Origin::Generated)),
        ("free mode", Box::new(|a| a.origin = Origin::FreeMode)),
        (
            "not counted",
            Box::new(|a| a.counts_toward_estimate = false),
        ),
        ("low confidence", Box::new(|a| a.confidence = 0.29)),
        (
            "pending",
            Box::new(|a| a.status = AttemptStatus::PendingLlm),
        ),
        (
            "needs review",
            Box::new(|a| a.status = AttemptStatus::NeedsReview),
        ),
        ("rejected", Box::new(|a| a.status = AttemptStatus::Rejected)),
        ("too old", Box::new(|a| a.created_at = NOW - 181 * DAY)),
    ];
    for (name, tweak) in tweaks {
        let mut rows = base.clone();
        rows.iter_mut().for_each(&tweak);
        let estimate = estimate_skill(&rows, Skill::Reading, None, NOW);
        assert_eq!(
            estimate.status,
            EstimateStatus::InsufficientEvidence,
            "{name}"
        );
    }
    // The same rows are fine when nothing is wrong, and 180 days exactly is still recent.
    let mut edge = base;
    edge.iter_mut().for_each(|a| a.created_at = NOW - 180 * DAY);
    assert_eq!(
        estimate_skill(&edge, Skill::Reading, None, NOW).status,
        EstimateStatus::Estimated
    );
}

#[test]
fn only_the_forty_most_recent_observations_per_level_count() {
    // 40 recent failures then 60 old successes: only the failures are used.
    let mut rows: Vec<Attempt> = (0..40)
        .map(|n| attempt(Skill::Reading, Level::A1, n, false))
        .collect();
    for n in 0..60 {
        let mut a = attempt(Skill::Reading, Level::A1, n, true);
        a.created_at = NOW - 10 * DAY - n as i64;
        a.activity_id = format!("old-{}", n % 5);
        rows.push(a);
    }
    assert_eq!(
        estimate_skill(&rows, Skill::Reading, None, NOW).status,
        EstimateStatus::WorkingTowardsA1
    );
}

#[test]
fn placement_alone_gives_a_placement_only_estimate_with_capped_confidence() {
    let estimate = estimate_skill(&[], Skill::Reading, Some(Level::A2), NOW);
    assert_eq!(estimate.status, EstimateStatus::PlacementOnly);
    assert_eq!(estimate.level, Some(Level::A2));
    assert!(estimate.confidence <= 0.5);
    assert_eq!(estimate.band, Some(Confidence::Low));
}

#[test]
fn real_attempts_above_the_placement_make_a_real_estimate() {
    let attempts = record(Skill::Reading, Level::B1, 9, 10);
    let estimate = estimate_skill(&attempts, Skill::Reading, Some(Level::A2), NOW);
    assert_eq!(estimate.status, EstimateStatus::Estimated);
    assert_eq!(estimate.level, Some(Level::B1));
}

#[test]
fn speaking_and_writing_confidence_is_capped_at_c1_and_c2() {
    let mut attempts = Vec::new();
    for level in Level::ALL {
        attempts.extend(record(Skill::Writing, level, 40, 40));
        attempts.extend(productive(Skill::Writing, level, 12, 12));
    }
    let estimate = estimate_skill(&attempts, Skill::Writing, None, NOW);
    assert_eq!(estimate.level, Some(Level::C2));
    assert!(estimate.confidence <= 0.6, "{}", estimate.confidence);

    // Reading at C2 is not capped.
    let mut reading = Vec::new();
    for level in Level::ALL {
        reading.extend(record(Skill::Reading, level, 40, 40));
    }
    assert!(estimate_skill(&reading, Skill::Reading, None, NOW).confidence > 0.8);
}

#[test]
fn confidence_words_follow_the_spec_boundaries() {
    assert_eq!(confidence_band(0.0), Confidence::Low);
    assert_eq!(confidence_band(0.599), Confidence::Low);
    assert_eq!(confidence_band(0.6), Confidence::Medium);
    assert_eq!(confidence_band(0.75), Confidence::Medium);
    assert_eq!(confidence_band(0.751), Confidence::High);
}

#[test]
fn the_working_level_is_shown_only_when_all_four_skills_are_estimated() {
    let mut attempts = Vec::new();
    for skill in Skill::ALL {
        attempts.extend(record(skill, Level::A1, 9, 10));
        if skill.is_productive() {
            attempts.extend(productive(skill, Level::A1, 3, 3));
        }
    }
    attempts.extend(record(Skill::Reading, Level::A2, 9, 10));
    let profile = estimate_profile(&attempts, &BTreeMap::new(), NOW);
    assert_eq!(
        profile.working_level,
        Some(Level::A1),
        "the lowest of the four"
    );

    let without_writing: Vec<Attempt> = attempts
        .into_iter()
        .filter(|a| a.skill != Skill::Writing)
        .collect();
    let partial = estimate_profile(&without_writing, &BTreeMap::new(), NOW);
    assert_eq!(partial.working_level, None);
}

/// A small deterministic generator, so the property checks need no extra crate.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

fn random_record(rng: &mut Lcg) -> Vec<Attempt> {
    let mut rows = Vec::new();
    for level in &Level::ALL[..3] {
        let total = (rng.next() % 30) as usize;
        for n in 0..total {
            rows.push(attempt(Skill::Reading, *level, n, rng.next() % 100 < 80));
        }
    }
    rows
}

#[test]
fn adding_a_success_never_lowers_the_estimate_or_its_confidence() {
    let mut rng = Lcg(7);
    for _ in 0..300 {
        let rows = random_record(&mut rng);
        let before = estimate_skill(&rows, Skill::Reading, None, NOW);
        let level = Level::ALL[(rng.next() % 3) as usize];
        let mut more = rows.clone();
        let mut extra = attempt(Skill::Reading, level, 1000, true);
        extra.created_at = NOW; // the newest
        more.push(extra);
        let after = estimate_skill(&more, Skill::Reading, None, NOW);
        assert!(
            (after.level, after.confidence) >= (before.level, before.confidence - 1e-9)
                || after.level > before.level,
            "before {before:?} after {after:?}"
        );
    }
}

#[test]
fn removing_a_success_never_raises_the_estimate_or_its_confidence() {
    let mut rng = Lcg(11);
    for _ in 0..300 {
        let rows = random_record(&mut rng);
        let successes: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, a)| a.normalized >= 0.7)
            .map(|(i, _)| i)
            .collect();
        if successes.is_empty() {
            continue;
        }
        let drop = successes[(rng.next() as usize) % successes.len()];
        let mut fewer = rows.clone();
        fewer.remove(drop);
        let before = estimate_skill(&rows, Skill::Reading, None, NOW);
        let after = estimate_skill(&fewer, Skill::Reading, None, NOW);
        assert!(
            after.level < before.level
                || (after.level == before.level && after.confidence <= before.confidence + 1e-9),
            "before {before:?} after {after:?}"
        );
    }
}

#[test]
fn the_result_names_its_algorithm_version() {
    assert_eq!(
        estimate_skill(&[], Skill::Reading, None, NOW).algorithm,
        "est/1"
    );
}
