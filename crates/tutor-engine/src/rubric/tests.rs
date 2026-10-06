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
        grammar_findings: 0,
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
    let message = user_message(Level::A2, &task, &rubric, "Come at 5. \"Bring\" food.");
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
    check.grammar_findings = 4;
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
