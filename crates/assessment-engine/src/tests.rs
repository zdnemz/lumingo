use super::*;

const NOW: i64 = 1_000_000_000;

fn attempt(response_id: u64, skill: Skill, level: Level, success: bool) -> Attempt {
    Attempt {
        response_id,
        skill,
        level,
        // Spread over 4 activities and 3 sessions so only the count matters unless a test says otherwise.
        activity_id: response_id % 4,
        session_id: response_id % 3,
        scorer: Scorer::Deterministic,
        normalized: if success { 1.0 } else { 0.0 },
        confidence: 1.0,
        status: Status::Scored,
        origin: Origin::Authored,
        counts_toward_estimate: true,
        created_at: NOW - response_id as i64,
    }
}

fn record(skill: Skill, level: Level, n: u64, successes: u64) -> Vec<Attempt> {
    (0..n)
        .map(|i| attempt(i, skill, level, i < successes))
        .collect()
}

fn cell(attempts: &[Attempt], skill: Skill, level: Level) -> LevelRow {
    let e = estimate(attempts, &BTreeMap::new(), NOW);
    e.rows
        .into_iter()
        .find(|r| r.skill == skill && r.level == level)
        .unwrap()
}

#[test]
fn the_table_in_9_2_is_reproduced() {
    // (observations, successes needed, lower bound at that count)
    let table = [
        (8, 7, 0.661),
        (10, 8, 0.602),
        (12, 10, 0.658),
        (16, 13, 0.661),
        (20, 15, 0.610),
        (30, 22, 0.620),
        (40, 28, 0.601),
    ];
    for (n, k, lb) in table {
        let at = cell(
            &record(Skill::Reading, Level::A1, n, k),
            Skill::Reading,
            Level::A1,
        );
        assert!(
            (at.lower_bound - lb).abs() < 0.0015,
            "{n}/{k}: {}",
            at.lower_bound
        );
        assert!(at.secured, "{n}/{k} should secure");
        let below = cell(
            &record(Skill::Reading, Level::A1, n, k - 1),
            Skill::Reading,
            Level::A1,
        );
        assert!(!below.secured, "{n}/{} should not secure", k - 1);
    }
}

#[test]
fn six_out_of_eight_is_not_enough() {
    let at = cell(
        &record(Skill::Reading, Level::A1, 8, 6),
        Skill::Reading,
        Level::A1,
    );
    assert!((at.lower_bound - 0.524).abs() < 0.0015);
    assert!(!at.secured);
}

#[test]
fn more_successes_never_lower_the_bound() {
    for n in 1..=60 {
        let bounds: Vec<f64> = (0..=n)
            .map(|k| wilson_lower_bound(k as f64 / n as f64, n as f64))
            .collect();
        assert!(bounds.windows(2).all(|w| w[0] <= w[1]), "n={n}");
    }
}

#[test]
fn less_evidence_at_the_same_rate_never_raises_confidence() {
    for p in [0.5, 0.7, 0.75, 0.8, 0.9, 1.0] {
        let bounds: Vec<f64> = (1..=80).map(|n| wilson_lower_bound(p, n as f64)).collect();
        assert!(bounds.windows(2).all(|w| w[0] <= w[1]), "p={p}");
    }
}

#[test]
fn attempts_that_are_generated_free_mode_uncounted_unscored_or_unsure_are_ignored() {
    let mut a = record(Skill::Reading, Level::A1, 10, 10);
    for (i, x) in a.iter_mut().enumerate() {
        match i % 5 {
            0 => x.origin = Origin::Generated,
            1 => x.origin = Origin::FreeMode,
            2 => x.counts_toward_estimate = false,
            3 => x.status = Status::PendingLlm,
            _ => x.confidence = 0.29,
        }
    }
    assert_eq!(cell(&a, Skill::Reading, Level::A1).n, 0.0);
}

#[test]
fn attempts_older_than_180_days_are_ignored() {
    let mut a = record(Skill::Reading, Level::A1, 10, 10);
    a[0].created_at = NOW - 181 * 86_400;
    assert_eq!(cell(&a, Skill::Reading, Level::A1).n, 9.0);
}

#[test]
fn only_the_40_most_recent_observations_count() {
    let mut a = record(Skill::Reading, Level::A1, 50, 50);
    for x in a.iter_mut().skip(40) {
        x.normalized = 0.0; // the oldest ten are failures and must fall outside the window
    }
    let at = cell(&a, Skill::Reading, Level::A1);
    assert_eq!((at.n, at.p), (40.0, 1.0));
}

#[test]
fn a_response_scored_on_several_dimensions_is_one_observation() {
    let mut a = Vec::new();
    for dim in 0..4 {
        for resp in 0..2 {
            let mut x = attempt(resp, Skill::Writing, Level::A1, true);
            x.scorer = Scorer::RubricLlm;
            x.confidence = 0.8;
            x.created_at = NOW - dim;
            a.push(x);
        }
    }
    let at = cell(&a, Skill::Writing, Level::A1);
    assert!(
        (at.n - 1.6).abs() < 1e-9,
        "two observations of weight 0.8, got {}",
        at.n
    );
}

#[test]
fn one_ineligible_row_spoils_the_whole_observation() {
    let mut a: Vec<Attempt> = (0..2)
        .map(|_| attempt(1, Skill::Writing, Level::A1, true))
        .collect();
    a[1].status = Status::NeedsReview;
    assert_eq!(cell(&a, Skill::Writing, Level::A1).n, 0.0);
}

#[test]
fn diversity_needs_three_activities_and_two_sessions() {
    let mut few_activities = record(Skill::Reading, Level::A1, 10, 10);
    few_activities.iter_mut().for_each(|a| a.activity_id %= 2);
    assert!(!cell(&few_activities, Skill::Reading, Level::A1).secured);
    let mut one_session = record(Skill::Reading, Level::A1, 10, 10);
    one_session.iter_mut().for_each(|a| a.session_id = 7);
    assert!(!cell(&one_session, Skill::Reading, Level::A1).secured);
}

#[test]
fn objective_items_alone_cannot_secure_a_productive_skill() {
    let objective = record(Skill::Speaking, Level::A1, 12, 12);
    assert!(!cell(&objective, Skill::Speaking, Level::A1).secured);
    let mut with_rubric = objective.clone();
    for a in with_rubric.iter_mut().take(3) {
        a.scorer = Scorer::RubricLlm;
    }
    // Rubric weights are 1.0 here because confidence is 1.0.
    assert!(cell(&with_rubric, Skill::Speaking, Level::A1).secured);
    // The same record is enough for a receptive skill.
    assert!(
        cell(
            &record(Skill::Listening, Level::A1, 12, 12),
            Skill::Listening,
            Level::A1
        )
        .secured
    );
}

fn ladder(skill: Skill, top: Level) -> Vec<Attempt> {
    let mut all = Vec::new();
    for (i, level) in Level::ALL
        .iter()
        .copied()
        .take_while(|l| *l <= top)
        .enumerate()
    {
        all.extend(record(skill, level, 10, 10).into_iter().map(|mut a| {
            a.response_id += 1000 * i as u64;
            a
        }));
    }
    all
}

#[test]
fn the_estimate_is_the_highest_level_with_every_level_below_secured() {
    let e = estimate(&ladder(Skill::Reading, Level::B1), &BTreeMap::new(), NOW);
    let r = e
        .estimates
        .iter()
        .find(|e| e.skill == Skill::Reading)
        .unwrap();
    assert_eq!(
        (r.level, r.status),
        (Some(Level::B1), EstimateStatus::Estimated)
    );
    assert_eq!(r.band(), Some(ConfidenceBand::High)); // 10 of 10 gives a lower bound near 0.86
}

#[test]
fn a_secured_level_above_a_gap_does_not_count() {
    let mut a = record(Skill::Reading, Level::A1, 10, 10);
    a.extend(
        record(Skill::Reading, Level::B1, 10, 10)
            .into_iter()
            .map(|mut x| {
                x.response_id += 1000;
                x
            }),
    );
    let e = estimate(&a, &BTreeMap::new(), NOW);
    assert_eq!(e.estimates[1].level, Some(Level::A1));
}

#[test]
fn no_evidence_is_insufficient_and_eight_unsecured_observations_mean_working_towards_a1() {
    let e = estimate(&[], &BTreeMap::new(), NOW);
    assert!(
        e.estimates
            .iter()
            .all(|e| e.status == EstimateStatus::InsufficientEvidence && !e.working_towards_a1)
    );
    let e = estimate(
        &record(Skill::Reading, Level::A1, 8, 5),
        &BTreeMap::new(),
        NOW,
    );
    let r = &e.estimates[1];
    assert_eq!((r.level, r.working_towards_a1), (None, true));
}

#[test]
fn placement_alone_gives_placement_only_with_confidence_capped_at_half() {
    let placed = BTreeMap::from([(Skill::Listening, Level::B1)]);
    let e = estimate(&[], &placed, NOW);
    let l = &e.estimates[0];
    assert_eq!(
        (l.level, l.status),
        (Some(Level::B1), EstimateStatus::PlacementOnly)
    );
    assert!(l.confidence.is_some_and(|c| c <= 0.5));
    // Real attempts above the placed-out levels turn it into an estimate.
    let mut a = record(Skill::Listening, Level::B2, 10, 10);
    a.iter_mut().for_each(|x| x.response_id += 5000);
    let e = estimate(&a, &placed, NOW);
    assert_eq!(
        (e.estimates[0].level, e.estimates[0].status),
        (Some(Level::B2), EstimateStatus::Estimated)
    );
}

#[test]
fn productive_skills_at_c1_and_above_are_capped_at_0_6() {
    let mut a = ladder(Skill::Speaking, Level::C1);
    a.iter_mut().for_each(|x| x.scorer = Scorer::RubricLlm);
    let e = estimate(&a, &BTreeMap::new(), NOW);
    let s = e
        .estimates
        .iter()
        .find(|e| e.skill == Skill::Speaking)
        .unwrap();
    assert_eq!(s.level, Some(Level::C1));
    assert!(s.confidence.is_some_and(|c| c <= 0.6));
}

#[test]
fn the_working_level_needs_all_four_skills_estimated_and_is_the_lowest() {
    let mut a = Vec::new();
    for (i, (skill, top)) in [
        (Skill::Listening, Level::B1),
        (Skill::Reading, Level::A2),
        (Skill::Speaking, Level::A2),
        (Skill::Writing, Level::B1),
    ]
    .into_iter()
    .enumerate()
    {
        a.extend(ladder(skill, top).into_iter().map(|mut x| {
            x.response_id += 100_000 * i as u64;
            if skill.is_productive() {
                x.scorer = Scorer::RubricLlm;
            }
            x
        }));
    }
    let e = estimate(&a, &BTreeMap::new(), NOW);
    assert_eq!(e.working_level(), Some(Level::A2));
    let without_writing: Vec<Attempt> = a
        .into_iter()
        .filter(|x| x.skill != Skill::Writing)
        .collect();
    assert_eq!(
        estimate(&without_writing, &BTreeMap::new(), NOW).working_level(),
        None
    );
}
