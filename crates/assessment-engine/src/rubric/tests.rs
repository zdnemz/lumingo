use super::*;
use crate::Skill;
use serde_json::json;

const RESPONSE: &str = "Yesterday I went to the market with my sister. We bought fresh fruit and vegetables, and then we went home.";
const ALL: [Dimension; 4] = [
    Dimension::TaskAchievement,
    Dimension::Range,
    Dimension::Accuracy,
    Dimension::Coherence,
];

fn dim(name: &str, band: &str, quote: &str) -> serde_json::Value {
    json!({ "dimension": name, "band": band, "evidence_quotes": if quote.is_empty() { vec![] } else { vec![quote] }, "reason": "r" })
}

fn output(bands: [&str; 4], on_task: bool, points: &[(bool, &str)]) -> RubricOutput {
    let q = "I went to the market";
    let value = json!({
        "dimension_scores": [
            dim("task_achievement", bands[0], if bands[0] == "0" { "" } else { q }),
            dim("range", bands[1], if bands[1] == "0" { "" } else { q }),
            dim("accuracy", bands[2], if bands[2] == "0" { "" } else { q }),
            dim("coherence", bands[3], if bands[3] == "0" { "" } else { q }),
        ],
        "content_points": points.iter().map(|(c, quote)| json!({ "point": "p", "covered": c, "quote": quote })).collect::<Vec<_>>(),
        "on_task": on_task,
        "feedback_en": "Good.",
        "feedback_l1": "Bagus.",
    });
    RubricOutput::from_json(&value).unwrap()
}

fn task() -> TaskContext<'static> {
    TaskContext {
        response: RESPONSE,
        min_words: None,
        dimensions: &ALL,
        unit_level: Level::A2,
        voice: false,
        vocabulary: None,
        findings_per_100_words: None,
    }
}

fn scored(run: RunScore) -> ScoredRun {
    match run {
        RunScore::Scored(s) => s,
        RunScore::Rejected(why) => panic!("rejected: {why}"),
    }
}

fn facts() -> RunFacts {
    RunFacts {
        qualified: true,
        weak_output: false,
        advanced_productive: false,
    }
}

#[test]
fn a_contract_shaped_output_parses_and_a_good_run_keeps_its_bands_and_quotes() {
    let run = scored(check_run(
        &output(["3", "3", "2", "3"], true, &[(true, "my sister")]),
        &task(),
    ));
    assert_eq!(run.bands[&Dimension::Accuracy], 2);
    assert_eq!(run.quotes[&Dimension::Range], ["I went to the market"]);
    assert!(!run.alarm && !run.off_task);
    assert!(RubricOutput::from_json(&json!({ "nope": 1 })).is_none());
}

#[test]
fn x1_quotes_match_ignoring_case_punctuation_and_spacing_but_only_on_whole_words() {
    assert!(quote_in_response(
        "YESTERDAY, i went  to the market!",
        RESPONSE
    ));
    assert!(quote_in_response("went home", "we went home."));
    assert!(
        !quote_in_response("market wi", RESPONSE),
        "partial words do not match"
    );
    assert!(!quote_in_response("I flew to the moon", RESPONSE));
    assert!(!quote_in_response("   ", RESPONSE));
}

#[test]
fn x1_a_band_above_zero_without_a_real_quote_rejects_the_run() {
    let mut o = output(["3", "3", "3", "3"], true, &[]);
    o.dimension_scores[1].evidence_quotes = vec!["words the learner never wrote".into()];
    assert_eq!(
        check_run(&o, &task()),
        RunScore::Rejected("a band above 0 has no valid evidence quote")
    );
}

#[test]
fn x1_invented_quotes_are_removed_when_a_real_one_remains() {
    let mut o = output(["3", "3", "3", "3"], true, &[]);
    o.dimension_scores[0]
        .evidence_quotes
        .push("invented words".into());
    let run = scored(check_run(&o, &task()));
    assert_eq!(
        run.quotes[&Dimension::TaskAchievement],
        ["I went to the market"]
    );
}

#[test]
fn bands_must_be_0_to_4_every_dimension_once_and_extras_are_ignored() {
    let mut o = output(["3", "3", "3", "3"], true, &[]);
    o.dimension_scores[0].band = "5".into();
    assert!(matches!(check_run(&o, &task()), RunScore::Rejected(_)));
    o.dimension_scores[0].band = "high".into();
    assert!(matches!(check_run(&o, &task()), RunScore::Rejected(_)));

    let mut missing = output(["3", "3", "3", "3"], true, &[]);
    missing.dimension_scores.pop();
    assert_eq!(
        check_run(&missing, &task()),
        RunScore::Rejected("a rubric dimension is missing")
    );

    let mut twice = output(["3", "3", "3", "3"], true, &[]);
    twice
        .dimension_scores
        .push(twice.dimension_scores[0].clone());
    assert_eq!(
        check_run(&twice, &task()),
        RunScore::Rejected("a dimension appears twice")
    );

    let mut extra = output(["3", "3", "3", "3"], true, &[]);
    extra
        .dimension_scores
        .push(extra.dimension_scores[0].clone());
    extra.dimension_scores[4].dimension = Dimension::Interaction;
    assert_eq!(scored(check_run(&extra, &task())).bands.len(), 4);
}

#[test]
fn x2_a_response_below_min_words_caps_task_achievement_at_2() {
    let ctx = TaskContext {
        min_words: Some(50),
        ..task()
    };
    let run = scored(check_run(&output(["4", "4", "4", "4"], true, &[]), &ctx));
    assert_eq!(
        (
            run.bands[&Dimension::TaskAchievement],
            run.bands[&Dimension::Range]
        ),
        (2, 4)
    );
    let low = scored(check_run(&output(["1", "3", "3", "3"], true, &[]), &ctx));
    assert_eq!(
        low.bands[&Dimension::TaskAchievement],
        1,
        "a cap never raises a band"
    );
    let enough = TaskContext {
        min_words: Some(10),
        ..task()
    };
    assert_eq!(
        scored(check_run(&output(["4", "4", "4", "4"], true, &[]), &enough)).bands
            [&Dimension::TaskAchievement],
        4
    );
}

#[test]
fn x3_content_points_with_invalid_quotes_count_as_not_covered() {
    let fake = [
        (true, "my sister"),
        (true, "never written anywhere"),
        (true, "also invented"),
    ];
    let run = scored(check_run(
        &output(["4", "4", "4", "4"], true, &fake),
        &task(),
    ));
    assert_eq!(
        run.bands[&Dimension::TaskAchievement],
        2,
        "only 1 of 3 points is really covered"
    );
    let real = [(true, "my sister"), (true, "fresh fruit"), (false, "")];
    let run = scored(check_run(
        &output(["4", "4", "4", "4"], true, &real),
        &task(),
    ));
    assert_eq!(
        run.bands[&Dimension::TaskAchievement],
        4,
        "2 of 3 is at least half"
    );
}

#[test]
fn x7_off_task_sets_every_band_to_zero() {
    let run = scored(check_run(
        &output(["4", "4", "4", "4"], false, &[]),
        &task(),
    ));
    assert!(run.off_task);
    assert!(run.bands.values().all(|b| *b == 0));
}

#[test]
fn x4_a_range_band_of_4_without_a_supporting_profile_raises_an_alarm() {
    let weak = VocabularyProfile {
        words: 20,
        distinct: 12,
        by_level: BTreeMap::from([(Level::A1, 20)]),
        unlisted: 0,
    };
    let strong = VocabularyProfile {
        words: 20,
        distinct: 12,
        by_level: BTreeMap::from([(Level::A1, 15), (Level::B2, 5)]),
        unlisted: 0,
    };
    let o = output(["3", "4", "3", "3"], true, &[]);
    assert!(
        scored(check_run(
            &o,
            &TaskContext {
                vocabulary: Some(&weak),
                ..task()
            }
        ))
        .alarm
    );
    assert!(
        !scored(check_run(
            &o,
            &TaskContext {
                vocabulary: Some(&strong),
                ..task()
            }
        ))
        .alarm
    );
    assert!(
        !scored(check_run(&o, &task())).alarm,
        "no word list, no judgement"
    );
    let not_four = output(["3", "3", "3", "3"], true, &[]);
    assert!(
        !scored(check_run(
            &not_four,
            &TaskContext {
                vocabulary: Some(&weak),
                ..task()
            }
        ))
        .alarm
    );
}

#[test]
fn x5_accuracy_against_rule_based_findings_raises_an_alarm_only_at_the_extremes() {
    let at = |bands, rate| {
        scored(check_run(
            &output(bands, true, &[]),
            &TaskContext {
                findings_per_100_words: Some(rate),
                ..task()
            },
        ))
        .alarm
    };
    assert!(at(["3", "3", "4", "3"], 12.0), "band 4 with many findings");
    assert!(!at(["3", "3", "4", "3"], 2.0));
    assert!(
        !at(["3", "3", "3", "3"], 12.0),
        "only band 4 is doubted for many findings"
    );
    assert!(
        at(["3", "3", "1", "3"], 0.0),
        "band 1 with none, on a response long enough to judge"
    );
    let short = TaskContext {
        response: "I went to the market.",
        findings_per_100_words: Some(0.0),
        ..task()
    };
    assert!(!scored(check_run(&output(["3", "3", "1", "3"], true, &[]), &short)).alarm);
}

#[test]
fn the_confidence_table_of_5_4_is_applied_in_order_and_bounded() {
    let run = check_run(&output(["3", "3", "3", "3"], true, &[]), &task());
    let runs = std::slice::from_ref(&run);
    assert!((finalize(runs, facts()).confidence - 0.8).abs() < 1e-9);
    assert!(
        (finalize(
            runs,
            RunFacts {
                qualified: false,
                ..facts()
            }
        )
        .confidence
            - 0.5)
            .abs()
            < 1e-9
    );
    assert!(
        (finalize(
            runs,
            RunFacts {
                weak_output: true,
                ..facts()
            }
        )
        .confidence
            - 0.6)
            .abs()
            < 1e-9
    );
    assert!(
        (finalize(
            runs,
            RunFacts {
                advanced_productive: true,
                ..facts()
            }
        )
        .confidence
            - 0.6)
            .abs()
            < 1e-9
    );
    let floor = RunFacts {
        qualified: false,
        weak_output: true,
        advanced_productive: false,
    };
    let alarmed = RunScore::Scored(ScoredRun {
        alarm: true,
        ..scored(run.clone())
    });
    assert!(
        (finalize(std::slice::from_ref(&alarmed), floor).confidence - 0.1).abs() < 1e-9,
        "0.5 - 0.2 - 0.2"
    );
    assert!((finalize(&[alarmed], facts()).confidence - 0.6).abs() < 1e-9);
    // two runs that agree exactly add 0.1
    assert!((finalize(&[run.clone(), run], facts()).confidence - 0.9).abs() < 1e-9);
}

#[test]
fn x6_two_runs_use_the_mean_and_a_gap_over_one_band_needs_review() {
    let a = check_run(&output(["3", "3", "3", "3"], true, &[]), &task());
    let b = check_run(&output(["4", "3", "3", "3"], true, &[]), &task());
    let r = finalize(&[a.clone(), b], facts());
    assert_eq!(r.status, Status::Scored);
    let ta = r
        .dimensions
        .iter()
        .find(|d| d.dimension == Dimension::TaskAchievement)
        .unwrap();
    assert_eq!(ta.band, 3.5);
    assert!(
        (r.confidence - 0.8).abs() < 1e-9,
        "no agreement bonus when the runs differ"
    );
    let far = check_run(&output(["1", "3", "3", "3"], true, &[]), &task());
    let r = finalize(&[a, far], facts());
    assert_eq!((r.status, r.dimensions.len()), (Status::NeedsReview, 0));
}

#[test]
fn a_rejected_run_ends_as_needs_review_after_the_caller_reran() {
    let good = check_run(&output(["3", "3", "3", "3"], true, &[]), &task());
    let bad = RunScore::Rejected("x");
    assert_eq!(
        finalize(std::slice::from_ref(&bad), facts()).status,
        Status::NeedsReview
    );
    assert_eq!(finalize(&[good, bad], facts()).status, Status::NeedsReview);
    assert_eq!(finalize(&[], facts()).status, Status::NeedsReview);
}

#[test]
fn attempts_are_one_per_dimension_with_band_over_four_and_feed_the_estimator_rules() {
    let run = check_run(&output(["4", "3", "2", "0"], true, &[]), &task());
    let result = finalize(&[run], facts());
    let base = AttemptBase {
        response_id: 7,
        skill: Skill::Writing,
        level: Level::A1,
        activity_id: 1,
        session_id: 1,
        origin: Origin::Authored,
        created_at: 0,
    };
    let attempts = result.attempts(&base);
    assert_eq!(attempts.len(), 4);
    assert!(attempts.iter().all(|a| a.response_id == 7
        && a.scorer == Scorer::RubricLlm
        && a.status == Status::Scored
        && a.counts_toward_estimate));
    let normalized: Vec<f64> = attempts.iter().map(|a| a.normalized).collect();
    assert_eq!(normalized, [1.0, 0.75, 0.5, 0.0]);
    let generated = result.attempts(&AttemptBase {
        origin: Origin::Generated,
        ..base
    });
    assert!(generated.iter().all(|a| !a.counts_toward_estimate));
    // 0.5 - 0.2 = 0.3 is exactly the floor and still counts; below it does not (section 5.4).
    let ok_run = check_run(&output(["3", "3", "3", "3"], true, &[]), &task());
    let at_floor = finalize(
        std::slice::from_ref(&ok_run),
        RunFacts {
            qualified: false,
            weak_output: true,
            advanced_productive: false,
        },
    );
    assert!((at_floor.confidence - 0.3).abs() < 1e-9);
    assert!(
        at_floor
            .attempts(&base)
            .iter()
            .all(|a| a.counts_toward_estimate)
    );
    let alarmed = RunScore::Scored(ScoredRun {
        alarm: true,
        ..scored(ok_run)
    });
    let below = finalize(
        &[alarmed],
        RunFacts {
            qualified: false,
            weak_output: true,
            advanced_productive: false,
        },
    );
    assert!(
        below
            .attempts(&base)
            .iter()
            .all(|a| !a.counts_toward_estimate && a.confidence < 0.3)
    );
    assert!(finalize(&[], facts()).attempts(&base).is_empty());
}
