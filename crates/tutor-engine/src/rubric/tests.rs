use super::*;

fn rubric() -> WorkshopRubric {
    let dims = [
        Dimension::TaskAchievement,
        Dimension::Range,
        Dimension::Accuracy,
        Dimension::Coherence,
    ];
    WorkshopRubric {
        id: "w-a2".into(),
        version: 1,
        dimensions: dims
            .into_iter()
            .map(|dimension| RubricDimension {
                dimension,
                bands: ["nothing", "little", "some", "enough", "more"].map(String::from),
            })
            .collect(),
    }
}

fn task() -> WorkshopTask {
    WorkshopTask {
        prompt: "Write a note to a friend.".into(),
        content_points: vec!["say when".into(), "say where".into()],
        min_words: Some(8),
    }
}

const RESPONSE: &str = "Hi Sari, come to my house on Saturday at five. We will cook rice and chicken together. Bring a bag.";

fn dim(dimension: Dimension, band: &str, quotes: &[&str]) -> RawDimension {
    RawDimension {
        dimension,
        band: band.into(),
        evidence_quotes: quotes.iter().map(|q| (*q).to_owned()).collect(),
        reason: "A reason.".into(),
    }
}

fn point(text: &str, covered: bool, quote: &str) -> RawPoint {
    RawPoint {
        point: text.into(),
        covered,
        quote: quote.into(),
    }
}

fn good() -> RawRubric {
    RawRubric {
        dimension_scores: vec![
            dim(Dimension::TaskAchievement, "4", &["on Saturday at five"]),
            dim(Dimension::Range, "3", &["cook rice and chicken"]),
            dim(Dimension::Accuracy, "3", &["Bring a bag"]),
            dim(Dimension::Coherence, "3", &["We will cook"]),
        ],
        content_points: vec![
            point("say when", true, "on Saturday at five"),
            point("say where", true, "my house"),
        ],
        on_task: true,
        feedback_en: "You gave clear details. Add one more sentence about food.".into(),
        feedback_l1: "Kamu memberi detail yang jelas. Tambahkan satu kalimat tentang makanan."
            .into(),
    }
}

fn input<'a>(
    response: &'a str,
    task: &'a WorkshopTask,
    rubric: &'a WorkshopRubric,
) -> CrossCheck<'a> {
    CrossCheck {
        response,
        level: Level::A2,
        task,
        rubric,
        grammar_findings: Some(0),
        word_levels: None,
        provider_qualified: true,
        repaired: false,
        ladder_level: LadderLevel::NativeSchema,
    }
}

fn band(result: &RubricResult, dimension: Dimension) -> Option<u8> {
    result
        .dimensions
        .iter()
        .find(|d| d.dimension == dimension)
        .and_then(|d| d.band)
}

#[test]
fn the_golden_prompt_matches_the_stored_file() {
    let golden = include_str!("../../tests/golden/rubric_score_1.txt");
    assert_eq!(system_prompt("Indonesian"), golden.trim_end_matches('\n'));
}

#[test]
fn the_golden_user_message_matches_the_stored_file() {
    let rubric = WorkshopRubric {
        id: "r".into(),
        version: 1,
        dimensions: vec![RubricDimension {
            dimension: Dimension::TaskAchievement,
            bands: ["nothing", "little", "some", "enough", "more"].map(String::from),
        }],
    };
    let task = WorkshopTask {
        prompt: "Write a note to a friend.".into(),
        content_points: vec!["say when".into(), "say where".into()],
        min_words: None,
    };
    let message = user_message(
        Level::A2,
        "text",
        &task,
        &rubric,
        "Come at 5. \"Bring\" food.",
    );
    let golden = include_str!("../../tests/golden/rubric_score_1_user.json");
    assert_eq!(message, golden.trim_end_matches('\n'));
}

#[test]
fn a_sound_reply_keeps_its_bands_quotes_and_feedback() {
    let (rubric, task) = (rubric(), task());
    let result = cross_check(&good(), &input(RESPONSE, &task, &rubric));
    assert_eq!(band(&result, Dimension::TaskAchievement), Some(4));
    assert_eq!(band(&result, Dimension::Range), Some(3));
    assert!(result.rejected.is_empty());
    assert!(result.alarms.is_empty());
    assert!(result.on_task);
    assert!(result.feedback_en.starts_with("You gave clear details"));
    assert!((result.confidence - 0.8).abs() < 1e-9);
}

#[test]
fn x1_removes_quotes_that_are_not_in_the_response_and_rejects_a_band_left_without_evidence() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[1] = dim(
        Dimension::Range,
        "3",
        &["a sentence the learner never wrote"],
    );
    raw.dimension_scores[2] = dim(
        Dimension::Accuracy,
        "3",
        &["BRING   a bag", "invented words"],
    );
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(band(&result, Dimension::Range), None);
    assert_eq!(result.rejected, [Dimension::Range]);
    let accuracy = result
        .dimensions
        .iter()
        .find(|d| d.dimension == Dimension::Accuracy)
        .expect("accuracy");
    assert_eq!(
        accuracy.evidence_quotes,
        ["BRING   a bag"],
        "only the valid quote stays"
    );
    assert_eq!(accuracy.band, Some(3));
}

#[test]
fn a_band_of_zero_needs_no_evidence() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[3] = dim(Dimension::Coherence, "0", &[]);
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(band(&result, Dimension::Coherence), Some(0));
    assert!(result.rejected.is_empty());
}

#[test]
fn a_band_outside_zero_to_four_is_rejected() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[0] = dim(Dimension::TaskAchievement, "7", &["my house"]);
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(result.rejected, [Dimension::TaskAchievement]);
}

#[test]
fn a_dimension_the_reply_leaves_out_needs_review_and_an_extra_one_is_ignored() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores.remove(3);
    raw.dimension_scores
        .push(dim(Dimension::Interaction, "3", &["my house"]));
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(result.dimensions.len(), 4, "only the rubric's dimensions");
    let missing = result
        .dimensions
        .iter()
        .find(|d| d.dimension == Dimension::Coherence)
        .expect("coherence");
    assert_eq!(missing.status, DimensionStatus::NeedsReview);
    assert!(
        result
            .dimensions
            .iter()
            .all(|d| d.dimension != Dimension::Interaction)
    );
}

#[test]
fn x2_caps_task_achievement_below_the_minimum_word_count() {
    let (rubric, mut task) = (rubric(), task());
    task.min_words = Some(50);
    let result = cross_check(&good(), &input(RESPONSE, &task, &rubric));
    assert_eq!(band(&result, Dimension::TaskAchievement), Some(2));
    assert!(result.dimensions[0].capped);
    assert_eq!(
        band(&result, Dimension::Range),
        Some(3),
        "other dimensions are untouched"
    );
}

#[test]
fn x3_counts_a_point_without_a_valid_quote_as_not_covered() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.content_points = vec![
        point("say when", true, "next Tuesday"),
        point("say where", true, "at the lake"),
    ];
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert!(result.content_points.iter().all(|p| !p.covered));
    assert_eq!(
        band(&result, Dimension::TaskAchievement),
        Some(2),
        "fewer than half covered"
    );
}

#[test]
fn x3_half_covered_is_enough() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.content_points[1] = point("say where", false, "");
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(band(&result, Dimension::TaskAchievement), Some(4));
}

#[test]
fn x7_an_off_task_response_gets_band_zero_everywhere() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.on_task = false;
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert!(!result.on_task);
    for d in &result.dimensions {
        assert_eq!(d.band, Some(0), "{:?}", d.dimension);
    }
    assert!(
        result.rejected.is_empty(),
        "no evidence is needed for band 0"
    );
}

#[test]
fn x5_an_accuracy_band_of_four_with_many_findings_lowers_confidence_only() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[2] = dim(Dimension::Accuracy, "4", &["Bring a bag"]);
    let mut check = input(RESPONSE, &task, &rubric);
    check.grammar_findings = Some(4);
    let result = cross_check(&raw, &check);
    assert_eq!(result.alarms, [Alarm::Accuracy]);
    assert_eq!(
        band(&result, Dimension::Accuracy),
        Some(4),
        "the band is not changed"
    );
    assert!((result.confidence - 0.6).abs() < 1e-9);
}

#[test]
fn x5_a_band_of_one_with_no_findings_is_an_alarm_too() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[2] = dim(Dimension::Accuracy, "1", &["Bring a bag"]);
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(result.alarms, [Alarm::Accuracy]);
}

#[test]
fn x4_a_range_band_of_four_on_a_poor_profile_lowers_confidence() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[1] = dim(Dimension::Range, "4", &["cook rice and chicken"]);
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(
        result.alarms,
        [Alarm::Range],
        "a short response has few distinct words"
    );
    assert_eq!(band(&result, Dimension::Range), Some(4));
}

#[test]
fn confidence_follows_section_5_4() {
    let (rubric, task) = (rubric(), task());
    let raw = good();
    let mut check = input(RESPONSE, &task, &rubric);
    check.provider_qualified = false;
    assert!((cross_check(&raw, &check).confidence - 0.5).abs() < 1e-9);
    check.provider_qualified = true;
    check.repaired = true;
    assert!((cross_check(&raw, &check).confidence - 0.6).abs() < 1e-9);
    check.repaired = false;
    check.ladder_level = LadderLevel::PromptOnly;
    assert!((cross_check(&raw, &check).confidence - 0.6).abs() < 1e-9);
    check.ladder_level = LadderLevel::NativeSchema;
    check.level = Level::C1;
    assert!((cross_check(&raw, &check).confidence - 0.6).abs() < 1e-9);
    // A repair and a level-4 ladder are one deduction, not two.
    check.level = Level::A2;
    check.provider_qualified = false;
    check.repaired = true;
    check.ladder_level = LadderLevel::PromptOnly;
    assert!((cross_check(&raw, &check).confidence - 0.3).abs() < 1e-9);
}

#[test]
fn feedback_that_names_a_level_or_a_percentage_is_discarded_and_long_feedback_is_cut() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.feedback_en = "This is B1 writing.".into();
    raw.feedback_l1 = "Kamu benar 80%.".into();
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(result.feedback_en, "");
    assert_eq!(result.feedback_l1, "");

    raw.feedback_en = (1..=60)
        .map(|n| format!("w{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let result = cross_check(&raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(
        result.feedback_en.split_whitespace().count(),
        MAX_FEEDBACK_WORDS
    );
}

#[test]
fn a_rerun_replaces_only_the_rejected_dimensions() {
    let (rubric, task) = (rubric(), task());
    let mut first_raw = good();
    first_raw.dimension_scores[1] = dim(Dimension::Range, "3", &["made up"]);
    let first = cross_check(&first_raw, &input(RESPONSE, &task, &rubric));
    assert_eq!(first.rejected, [Dimension::Range]);

    let mut second_raw = good();
    second_raw.dimension_scores[0] = dim(Dimension::TaskAchievement, "1", &["my house"]);
    let second = cross_check(&second_raw, &input(RESPONSE, &task, &rubric));
    let merged = merge_rerun(first.clone(), &second);
    assert_eq!(
        band(&merged, Dimension::Range),
        Some(3),
        "taken from the second run"
    );
    assert_eq!(
        band(&merged, Dimension::TaskAchievement),
        Some(4),
        "first run kept"
    );
    assert!(merged.rejected.is_empty());

    // A second run that fails too leaves the dimension for review.
    let mut bad_raw = good();
    bad_raw.dimension_scores[1] = dim(Dimension::Range, "3", &["also made up"]);
    let bad = cross_check(&bad_raw, &input(RESPONSE, &task, &rubric));
    let merged = merge_rerun(first, &bad);
    assert_eq!(merged.rejected, [Dimension::Range]);
    assert_eq!(band(&merged, Dimension::Range), None);
}

#[test]
fn x5_is_skipped_when_no_checker_is_linked_and_is_not_a_finding_count_of_zero() {
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[2] = dim(Dimension::Accuracy, "1", &["Bring a bag"]);
    let mut check = input(RESPONSE, &task, &rubric);
    check.grammar_findings = Some(0);
    assert_eq!(cross_check(&raw, &check).alarms, [Alarm::Accuracy]);
    check.grammar_findings = None;
    let result = cross_check(&raw, &check);
    assert!(result.alarms.is_empty(), "no checker, no alarm");
    assert!((result.confidence - 0.8).abs() < 1e-9);
}

#[test]
fn x4_uses_the_vocabulary_profile_of_the_response() {
    use std::collections::HashMap;
    let (rubric, task) = (rubric(), task());
    let mut raw = good();
    raw.dimension_scores[1] = dim(Dimension::Range, "4", &["cook rice and chicken"]);
    // A response with enough distinct words for the count test, so only the profile decides.
    let wide = "Hi Sari, come to my house on Saturday at five. We will cook rice and chicken together, \
        then eat, talk, play music, watch a film and drink tea. Bring a bag, a book, a jacket and some fruit \
        because the evening may be cold and long.";
    let mut check = input(wide, &task, &rubric);
    assert!(
        cross_check(&raw, &check).alarms.is_empty(),
        "without a list only the distinct words count"
    );
    let mut list: HashMap<String, Level> = HashMap::new();
    for word in [
        "hi", "come", "house", "cook", "rice", "chicken", "eat", "talk", "play",
    ] {
        list.insert(word.to_owned(), Level::A1);
    }
    check.word_levels = Some(&list);
    assert_eq!(
        cross_check(&raw, &check).alarms,
        [Alarm::Range],
        "a list that knows the words and finds none above the level does not support a 4"
    );
    let mut richer = list.clone();
    richer.insert("jacket".to_owned(), Level::B1);
    check.word_levels = Some(&richer);
    assert!(cross_check(&raw, &check).alarms.is_empty());
    // A list that knows nothing of the response says nothing.
    let unrelated: HashMap<String, Level> = HashMap::from([("zebra".to_owned(), Level::A1)]);
    check.word_levels = Some(&unrelated);
    assert!(cross_check(&raw, &check).alarms.is_empty());
}

#[test]
fn the_confidence_table_adds_a_tenth_when_two_runs_agree_and_keeps_its_bounds() {
    let base = ConfidenceInputs {
        provider_qualified: true,
        alarm: false,
        repaired: false,
        runs_agreed: false,
        level: Level::A2,
    };
    assert!((rubric_confidence(&base) - 0.8).abs() < 1e-9);
    let agreed = ConfidenceInputs {
        runs_agreed: true,
        ..base
    };
    assert!((rubric_confidence(&agreed) - 0.9).abs() < 1e-9);
    // Productive at C1 or C2: capped at 0.6 even when two runs agree.
    for level in [Level::C1, Level::C2] {
        let capped = rubric_confidence(&ConfidenceInputs { level, ..agreed });
        assert!((capped - 0.6).abs() < 1e-9, "{level:?}");
    }
    // Everything wrong: not qualified, alarm, repaired: 0.5 - 0.2 - 0.2 = 0.1.
    let worst = ConfidenceInputs {
        provider_qualified: false,
        alarm: true,
        repaired: true,
        runs_agreed: false,
        level: Level::A2,
    };
    assert!((rubric_confidence(&worst) - 0.1).abs() < 1e-9);
}

fn checked(raw: &RawRubric, repaired: bool) -> CheckedRun {
    let (rubric, task) = (rubric(), task());
    CheckedRun {
        result: cross_check(raw, &input(RESPONSE, &task, &rubric)),
        repaired,
    }
}

fn with_bands(ta: &str, range: &str, accuracy: &str, coherence: &str) -> RawRubric {
    let mut raw = good();
    raw.dimension_scores = vec![
        dim(Dimension::TaskAchievement, ta, &["on Saturday at five"]),
        dim(Dimension::Range, range, &["cook rice and chicken"]),
        dim(Dimension::Accuracy, accuracy, &["Bring a bag"]),
        dim(Dimension::Coherence, coherence, &["We will cook"]),
    ];
    raw
}

fn mean_of(outcome: &RubricOutcome, dimension: Dimension) -> Option<f64> {
    outcome
        .dimensions
        .iter()
        .find(|d| d.dimension == dimension)
        .and_then(|d| d.band)
}

#[test]
fn x6_two_runs_that_agree_exactly_keep_the_band_and_add_a_tenth_of_confidence() {
    let run = checked(&with_bands("3", "3", "3", "3"), false);
    let outcome = RubricOutcome::join(&[run.clone(), run], true, Level::A2);
    assert_eq!(outcome.runs, 2);
    assert!(outcome.runs_agreed);
    assert!(outcome.is_complete());
    assert_eq!(mean_of(&outcome, Dimension::Range), Some(3.0));
    assert!((outcome.confidence - 0.9).abs() < 1e-9);
    assert_eq!(outcome.mean_normalized(), Some(0.75));
}

#[test]
fn x6_a_gap_of_one_band_uses_the_mean_and_gets_no_bonus() {
    let first = checked(&with_bands("3", "3", "3", "3"), false);
    let second = checked(&with_bands("4", "3", "2", "3"), false);
    let outcome = RubricOutcome::join(&[first, second], true, Level::A2);
    assert!(outcome.is_complete());
    assert!(!outcome.runs_agreed);
    assert_eq!(mean_of(&outcome, Dimension::TaskAchievement), Some(3.5));
    assert_eq!(mean_of(&outcome, Dimension::Accuracy), Some(2.5));
    assert!((outcome.confidence - 0.8).abs() < 1e-9);
    let ta = &outcome.dimensions[0];
    assert_eq!(ta.run_bands, [3, 4]);
    assert_eq!(ta.normalized(), Some(3.5 / 4.0));
}

#[test]
fn x6_a_gap_of_two_bands_sends_that_dimension_to_review() {
    let first = checked(&with_bands("3", "3", "3", "3"), false);
    let second = checked(&with_bands("3", "1", "3", "3"), false);
    let outcome = RubricOutcome::join(&[first, second], true, Level::A2);
    assert!(!outcome.is_complete());
    let range = outcome
        .dimensions
        .iter()
        .find(|d| d.dimension == Dimension::Range)
        .unwrap();
    assert_eq!(range.status, DimensionStatus::NeedsReview);
    assert!(range.runs_disagree);
    assert_eq!(range.band, None);
    assert_eq!(range.run_bands, [3, 1]);
    assert_eq!(
        outcome.mean_normalized(),
        None,
        "an unfinished response has no mean"
    );
    // The other dimensions are still scored.
    assert_eq!(mean_of(&outcome, Dimension::Accuracy), Some(3.0));
    assert!(!outcome.runs_agreed);
}

#[test]
fn x6_a_dimension_one_run_could_not_score_stays_in_review() {
    let first = checked(&with_bands("3", "3", "3", "3"), false);
    // A band the evidence does not support is rejected by X1 in this run.
    let mut raw = with_bands("3", "3", "3", "3");
    raw.dimension_scores[1] = dim(Dimension::Range, "3", &["words the learner never wrote"]);
    let second = checked(&raw, false);
    let outcome = RubricOutcome::join(&[first, second], true, Level::A2);
    let range = &outcome.dimensions[1];
    assert_eq!(range.status, DimensionStatus::NeedsReview);
    assert!(!range.runs_disagree, "no disagreement, a missing score");
    assert!(!outcome.is_complete());
}

#[test]
fn the_confidence_of_joined_runs_takes_the_alarms_and_repairs_of_either() {
    let (rubric, task) = (rubric(), task());
    let mut accuracy_four = good();
    accuracy_four.dimension_scores[2] = dim(Dimension::Accuracy, "4", &["Bring a bag"]);
    let mut check = input(RESPONSE, &task, &rubric);
    check.grammar_findings = Some(4);
    let alarming = CheckedRun {
        result: cross_check(&accuracy_four, &check),
        repaired: false,
    };
    assert_eq!(alarming.result.alarms, [Alarm::Accuracy]);
    let plain = checked(&good(), false);
    let outcome = RubricOutcome::join(&[plain.clone(), alarming], true, Level::A2);
    assert_eq!(outcome.alarms, [Alarm::Accuracy]);
    assert!(
        (outcome.confidence - 0.6).abs() < 1e-9,
        "0.8 - 0.2 alarm, no agreement"
    );

    let repaired = checked(&good(), true);
    let outcome = RubricOutcome::join(&[plain.clone(), repaired], true, Level::A2);
    assert!(outcome.repaired);
    assert!(
        (outcome.confidence - 0.7).abs() < 1e-9,
        "0.8 - 0.2 repair + 0.1 agreement"
    );

    let c1 = RubricOutcome::join(&[plain.clone(), plain], true, Level::C1);
    assert!((c1.confidence - 0.6).abs() < 1e-9, "capped at C1");
}

#[test]
fn a_single_run_joins_to_itself_and_an_off_task_run_makes_the_response_off_task() {
    let run = checked(&with_bands("3", "2", "3", "4"), false);
    let outcome = RubricOutcome::join(std::slice::from_ref(&run), true, Level::A2);
    assert_eq!(outcome.runs, 1);
    assert!(!outcome.runs_agreed, "agreement needs two runs");
    assert_eq!(mean_of(&outcome, Dimension::Range), Some(2.0));
    assert!((outcome.confidence - run.result.confidence).abs() < 1e-9);
    assert!(!outcome.try_again());
    assert_eq!(outcome.feedback_en, run.result.feedback_en);

    let mut off = good();
    off.on_task = false;
    let outcome = RubricOutcome::join(&[checked(&off, false)], true, Level::A2);
    assert!(outcome.try_again());
    assert_eq!(mean_of(&outcome, Dimension::Range), Some(0.0));
}

#[test]
fn nothing_to_join_is_incomplete() {
    let outcome = RubricOutcome::join(&[], true, Level::A1);
    assert!(!outcome.is_complete());
    assert_eq!(outcome.runs, 0);
}

#[test]
fn the_voice_message_says_so() {
    let (rubric, task) = (rubric(), task());
    let message = user_message(Level::A2, "voice", &task, &rubric, "hello my name is dewi");
    assert!(message.contains(r#""input_mode":"voice""#), "{message}");
}
