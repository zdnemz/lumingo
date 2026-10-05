//! Per-unit error rules E02 to E20 of `docs/CURRICULUM_SPEC.md` section 6.
//! E01 (the schema) is applied by the caller before a typed unit exists.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::report::{Finding, RuleCode};
use super::text::{normalise_answer, tts_problems, word_count};
use super::texts::localized_texts;
use crate::model::{
    Activity, ActivityType, Band, Level, ModelAnswer, Question, Scoring, Skill, Unit,
};

/// The error categories of `contracts/turn_analysis.schema.json`. A test compares this list
/// with the contract file, so a category added to the contract cannot be forgotten here.
pub const ERROR_CATEGORIES: [&str; 28] = [
    "verb_tense",
    "verb_form",
    "subject_verb_agreement",
    "copula",
    "article",
    "preposition",
    "plural_number",
    "pronoun",
    "word_order",
    "question_formation",
    "negation",
    "modal",
    "conditional",
    "passive",
    "relative_clause",
    "comparison",
    "quantifier_countability",
    "linking",
    "word_choice",
    "collocation",
    "word_form",
    "l1_transfer",
    "register",
    "missing_word",
    "extra_word",
    "spelling",
    "punctuation",
    "unclear_meaning",
];

/// Passage length per level for a `reading_set`, in words (spec section 4, inclusive).
pub fn reading_range(level: Level) -> (usize, usize) {
    match level {
        Level::A1 => (60, 100),
        Level::A2 => (100, 180),
        Level::B1 => (180, 300),
        Level::B2 => (300, 450),
        Level::C1 => (450, 650),
        Level::C2 => (600, 800),
    }
}

/// Audio text length per level for a `listening_set`, in words (spec section 4, inclusive).
pub fn listening_range(level: Level) -> (usize, usize) {
    match level {
        Level::A1 => (25, 60),
        Level::A2 => (50, 100),
        Level::B1 => (90, 160),
        Level::B2 => (140, 220),
        Level::C1 => (180, 280),
        Level::C2 => (220, 320),
    }
}

/// `at` model answer length per level for a `guided_writing`, in words (spec section 4, inclusive).
pub fn writing_range(level: Level) -> (usize, usize) {
    match level {
        Level::A1 => (8, 40),
        Level::A2 => (35, 80),
        Level::B1 => (80, 150),
        Level::B2 => (150, 250),
        Level::C1 => (220, 320),
        Level::C2 => (280, 400),
    }
}

/// Longest dialogue line in characters (rule 15).
const MAX_DIALOGUE_LINE_CHARS: usize = 200;
/// Longest A1 dialogue line in words (rule 15).
const MAX_A1_DIALOGUE_LINE_WORDS: usize = 12;

fn finding(code: RuleCode, path: impl Into<String>, message: impl Into<String>) -> Finding {
    Finding::new(code, path, message)
}

/// Checks that need nothing but the activity itself: E05, E06, E07, E09 (without the unit),
/// E15 for its audio text, E17 for its questions, E19 and E20.
///
/// The tutor can call this on a generated `mcq`, `gap_fill` or `reorder` before the learner
/// sees it. `path` is the JSON pointer of the activity in whatever document it came from.
pub fn check_activity_alone(activity: &Activity, path: &str, out: &mut Vec<Finding>) {
    match activity {
        Activity::Mcq(a) => {
            if usize::from(a.answer_index) >= a.options.len() {
                out.push(finding(
                    RuleCode::E05,
                    format!("{path}/answer_index"),
                    format!(
                        "answer_index {} is outside the {} options",
                        a.answer_index,
                        a.options.len()
                    ),
                ));
            }
            if a.passage.is_some() && a.audio_text.is_some() {
                out.push(finding(
                    RuleCode::E05,
                    path,
                    "passage and audio_text are both present; an mcq is either a reading item or a listening item",
                ));
            }
            if let Some(text) = &a.audio_text {
                tts_findings(text, &format!("{path}/audio_text"), out);
            }
        }
        Activity::GapFill(a) => {
            let gaps = a.text.matches("___").count();
            if gaps != a.answers.len() {
                out.push(finding(
                    RuleCode::E06,
                    format!("{path}/answers"),
                    format!(
                        "the text has {gaps} gap(s) but there are {} answer list(s)",
                        a.answers.len()
                    ),
                ));
            }
        }
        Activity::Reorder(a) => check_reorder(&a.tokens, &a.answer, path, out),
        Activity::Dictation(a) => tts_findings(&a.audio_text, &format!("{path}/audio_text"), out),
        Activity::ReadAloud(a) => tts_findings(&a.text, &format!("{path}/text"), out),
        Activity::GuidedSpeaking(a) | Activity::GuidedWriting(a) => {
            check_production(
                &a.model_answers,
                Some((a.min_words, a.max_words)),
                path,
                out,
            );
        }
        Activity::Mediation(a) => check_production(&a.model_answers, None, path, out),
        Activity::ReadingSet(a) => check_questions(&a.questions, path, out),
        Activity::ListeningSet(a) => {
            check_questions(&a.questions, path, out);
            if let Some(text) = &a.audio_text {
                tts_findings(text, &format!("{path}/audio_text"), out);
            }
        }
        Activity::ErrorCorrection(a) => {
            if !ERROR_CATEGORIES.contains(&a.error_category.as_str()) {
                out.push(finding(
                    RuleCode::E19,
                    format!("{path}/error_category"),
                    format!(
                        "\"{}\" is not a category of contracts/turn_analysis.schema.json",
                        a.error_category
                    ),
                ));
            }
            let sentence = normalise_answer(&a.sentence);
            if a.accepted_answers
                .iter()
                .any(|accepted| normalise_answer(accepted) == sentence)
            {
                out.push(finding(
                    RuleCode::E20,
                    format!("{path}/sentence"),
                    "the sentence is equal to an accepted answer after normalisation, so it has no mistake to fix",
                ));
            }
        }
        Activity::Match(_)
        | Activity::MinimalPairs(_)
        | Activity::Shadowing(_)
        | Activity::Roleplay(_) => {}
    }
}

fn check_reorder(tokens: &[String], answer: &str, path: &str, out: &mut Vec<Finding>) {
    let answer_words: Vec<&str> = answer.split_whitespace().collect();
    let mut expected: Vec<&str> = answer_words.clone();
    let mut actual: Vec<&str> = tokens.iter().map(String::as_str).collect();
    expected.sort_unstable();
    actual.sort_unstable();
    if expected != actual {
        out.push(finding(
            RuleCode::E07,
            format!("{path}/answer"),
            "answer does not use exactly the tokens: the words of the answer and the tokens differ",
        ));
    } else if tokens
        .iter()
        .map(String::as_str)
        .eq(answer_words.iter().copied())
    {
        out.push(finding(
            RuleCode::E07,
            format!("{path}/tokens"),
            "the tokens are already in answer order",
        ));
    }
}

fn check_production(
    model_answers: &[ModelAnswer],
    limits: Option<(u32, u32)>,
    path: &str,
    out: &mut Vec<Finding>,
) {
    for band in [Band::Below, Band::At, Band::Above] {
        let count = model_answers.iter().filter(|m| m.band == band).count();
        if count != 1 {
            out.push(finding(
                RuleCode::E09,
                format!("{path}/model_answers"),
                format!("needs exactly one {band:?} model answer, found {count}"),
            ));
        }
    }
    let Some((min, max)) = limits else { return };
    if min >= max {
        out.push(finding(
            RuleCode::E09,
            format!("{path}/max_words"),
            format!("min_words ({min}) must be below max_words ({max})"),
        ));
    }
    if let Some((index, at)) = model_answers
        .iter()
        .enumerate()
        .find(|(_, m)| m.band == Band::At)
    {
        let words = word_count(&at.text);
        if words < min as usize || words > max as usize {
            out.push(finding(
                RuleCode::E09,
                format!("{path}/model_answers/{index}/text"),
                format!("the at answer has {words} words, outside {min} to {max}"),
            ));
        }
    }
}

fn check_questions(questions: &[Question], path: &str, out: &mut Vec<Finding>) {
    let mut seen: HashSet<String> = HashSet::new();
    for (q, question) in questions.iter().enumerate() {
        if usize::from(question.answer_index) >= question.options.len() {
            out.push(finding(
                RuleCode::E17,
                format!("{path}/questions/{q}/answer_index"),
                format!(
                    "answer_index {} is outside the {} options",
                    question.answer_index,
                    question.options.len()
                ),
            ));
        }
        let stem = question
            .stem
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !seen.insert(stem) {
            out.push(finding(
                RuleCode::E17,
                format!("{path}/questions/{q}/stem"),
                "another question of this set has the same stem",
            ));
        }
    }
}

fn tts_findings(text: &str, path: &str, out: &mut Vec<Finding>) {
    for problem in tts_problems(text) {
        out.push(finding(RuleCode::E15, path, problem));
    }
}

/// E02 to E20 for a unit that already passed the schema.
/// `document` is the JSON the unit was read from; E14 walks it so every localized text
/// is reached whatever field it sits in.
pub fn check_unit(unit: &Unit, document: &Value) -> Vec<Finding> {
    let mut out = Vec::new();
    check_id(unit, &mut out);
    check_unique_ids(unit, &mut out);
    check_objective_links(unit, &mut out);
    for (i, activity) in unit.activities.iter().enumerate() {
        check_activity_alone(activity, &format!("/activities/{i}"), &mut out);
    }
    check_activity_links(unit, &mut out);
    check_checkpoint(unit, &mut out);
    check_generation_and_review(unit, &mut out);
    check_indonesian_present(unit, document, &mut out);
    check_dialogues(unit, &mut out);
    check_minimum_content(unit, &mut out);
    check_set_context(unit, &mut out);
    out
}

fn check_id(unit: &Unit, out: &mut Vec<Finding>) {
    let expected = format!("{}-u{:02}", unit.level.lowercase(), unit.sequence);
    if unit.id != expected {
        out.push(finding(
            RuleCode::E02,
            "/id",
            format!(
                "id is \"{}\" but level {} and sequence {} give \"{expected}\"",
                unit.id, unit.level, unit.sequence
            ),
        ));
    }
}

fn check_unique_ids(unit: &Unit, out: &mut Vec<Finding>) {
    fn scan<'a>(ids: impl Iterator<Item = &'a str>, collection: &str, out: &mut Vec<Finding>) {
        let mut seen = HashSet::new();
        for (i, id) in ids.enumerate() {
            if !seen.insert(id) {
                out.push(finding(
                    RuleCode::E03,
                    format!("{collection}/{i}/id"),
                    format!("id \"{id}\" is used twice in this collection"),
                ));
            }
        }
    }
    scan(
        unit.objectives.iter().map(|o| o.id.as_str()),
        "/objectives",
        out,
    );
    scan(
        unit.targets.vocabulary.iter().map(|v| v.id.as_str()),
        "/targets/vocabulary",
        out,
    );
    scan(
        unit.targets.grammar.iter().map(|g| g.id.as_str()),
        "/targets/grammar",
        out,
    );
    scan(
        unit.targets.pronunciation.iter().map(|p| p.id.as_str()),
        "/targets/pronunciation",
        out,
    );
    scan(
        unit.dialogues.iter().map(|d| d.id.as_str()),
        "/dialogues",
        out,
    );
    scan(unit.activities.iter().map(Activity::id), "/activities", out);
}

fn check_objective_links(unit: &Unit, out: &mut Vec<Finding>) {
    let known: HashSet<&str> = unit.objectives.iter().map(|o| o.id.as_str()).collect();
    let mut served: HashSet<&str> = HashSet::new();
    for (i, activity) in unit.activities.iter().enumerate() {
        for (j, objective_id) in activity.common().objective_ids.iter().enumerate() {
            if known.contains(objective_id.as_str()) {
                served.insert(objective_id.as_str());
            } else {
                out.push(finding(
                    RuleCode::E04,
                    format!("/activities/{i}/objective_ids/{j}"),
                    format!("there is no objective \"{objective_id}\""),
                ));
            }
        }
    }
    for (i, objective) in unit.objectives.iter().enumerate() {
        if !served.contains(objective.id.as_str()) {
            out.push(finding(
                RuleCode::E04,
                format!("/objectives/{i}"),
                format!("no activity serves objective \"{}\"", objective.id),
            ));
        }
    }
}

fn check_activity_links(unit: &Unit, out: &mut Vec<Finding>) {
    let dialogues: HashSet<&str> = unit.dialogues.iter().map(|d| d.id.as_str()).collect();
    let grammar: HashSet<&str> = unit.targets.grammar.iter().map(|g| g.id.as_str()).collect();
    let vocab: HashSet<&str> = unit
        .targets
        .vocabulary
        .iter()
        .map(|v| v.id.as_str())
        .collect();
    for (i, activity) in unit.activities.iter().enumerate() {
        let path = format!("/activities/{i}");
        match activity {
            Activity::Shadowing(a) if !dialogues.contains(a.dialogue_id.as_str()) => {
                out.push(finding(
                    RuleCode::E08,
                    format!("{path}/dialogue_id"),
                    format!("there is no dialogue \"{}\"", a.dialogue_id),
                ));
            }
            Activity::Roleplay(a) => {
                for (j, id) in a.target_grammar_ids.iter().enumerate() {
                    if !grammar.contains(id.as_str()) {
                        out.push(finding(
                            RuleCode::E10,
                            format!("{path}/target_grammar_ids/{j}"),
                            format!("there is no grammar point \"{id}\" in this unit"),
                        ));
                    }
                }
                for (j, id) in a.target_vocab_ids.iter().enumerate() {
                    if !vocab.contains(id.as_str()) {
                        out.push(finding(
                            RuleCode::E10,
                            format!("{path}/target_vocab_ids/{j}"),
                            format!("there is no vocabulary item \"{id}\" in this unit"),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

fn check_checkpoint(unit: &Unit, out: &mut Vec<Finding>) {
    let by_id: HashMap<&str, &Activity> = unit.activities.iter().map(|a| (a.id(), a)).collect();
    for (j, id) in unit.checkpoint.activity_ids.iter().enumerate() {
        let path = format!("/checkpoint/activity_ids/{j}");
        match by_id.get(id.as_str()) {
            None => out.push(finding(
                RuleCode::E11,
                path,
                format!("there is no activity \"{id}\""),
            )),
            Some(activity) if activity.common().scoring == Scoring::None => out.push(finding(
                RuleCode::E11,
                path,
                format!("activity \"{id}\" has scoring none, so it cannot decide the checkpoint"),
            )),
            Some(_) => {}
        }
    }
}

fn check_generation_and_review(unit: &Unit, out: &mut Vec<Finding>) {
    let grammar: HashSet<&str> = unit.targets.grammar.iter().map(|g| g.id.as_str()).collect();
    for (j, id) in unit
        .generation_policy
        .allowed_grammar_ids
        .iter()
        .enumerate()
    {
        if !grammar.contains(id.as_str()) {
            out.push(finding(
                RuleCode::E12,
                format!("/generation_policy/allowed_grammar_ids/{j}"),
                format!("there is no grammar point \"{id}\" in this unit"),
            ));
        }
    }

    let vocab: HashSet<&str> = unit
        .targets
        .vocabulary
        .iter()
        .map(|v| v.id.as_str())
        .collect();
    let pron: HashSet<&str> = unit
        .targets
        .pronunciation
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    for (j, item) in unit.review_items.iter().enumerate() {
        use crate::model::ReviewKind;
        let (known, what) = match item.kind {
            ReviewKind::Vocab => (&vocab, "vocabulary item"),
            ReviewKind::Grammar => (&grammar, "grammar point"),
            ReviewKind::Pron => (&pron, "pronunciation focus"),
        };
        if !known.contains(item.r#ref.as_str()) {
            out.push(finding(
                RuleCode::E13,
                format!("/review_items/{j}/ref"),
                format!("there is no {what} \"{}\" in this unit", item.r#ref),
            ));
        }
    }
}

/// E14: from A1 to B1 every localized text needs its Indonesian `id` value.
fn check_indonesian_present(unit: &Unit, document: &Value, out: &mut Vec<Finding>) {
    if unit.level > Level::B1 {
        return;
    }
    for text in localized_texts(document) {
        if text.id.is_none_or(|id| id.trim().is_empty()) {
            out.push(finding(
                RuleCode::E14,
                format!("{}/id", text.pointer),
                "the Indonesian text is missing; levels A1 to B1 need it",
            ));
        }
    }
}

/// E15 for dialogues: allowed characters, 200 characters per line, and 12 words at A1.
fn check_dialogues(unit: &Unit, out: &mut Vec<Finding>) {
    for (d, dialogue) in unit.dialogues.iter().enumerate() {
        for (t, turn) in dialogue.turns.iter().enumerate() {
            let path = format!("/dialogues/{d}/turns/{t}/text");
            tts_findings(&turn.text, &path, out);
            let chars = turn.text.chars().count();
            if chars > MAX_DIALOGUE_LINE_CHARS {
                out.push(finding(
                    RuleCode::E15,
                    &path,
                    format!(
                        "the line has {chars} characters, the limit is {MAX_DIALOGUE_LINE_CHARS}"
                    ),
                ));
            }
            let words = word_count(&turn.text);
            if unit.level == Level::A1 && words > MAX_A1_DIALOGUE_LINE_WORDS {
                out.push(finding(
                    RuleCode::E15,
                    &path,
                    format!("the line has {words} words, A1 lines have at most {MAX_A1_DIALOGUE_LINE_WORDS}"),
                ));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CoreSkill {
    Listening,
    Reading,
    Speaking,
    Writing,
}

fn core_skill(skill: Skill) -> Option<CoreSkill> {
    match skill {
        Skill::Listening => Some(CoreSkill::Listening),
        Skill::Reading => Some(CoreSkill::Reading),
        Skill::SpeakingProduction | Skill::SpeakingInteraction => Some(CoreSkill::Speaking),
        Skill::Writing => Some(CoreSkill::Writing),
        Skill::Mediation | Skill::Grammar | Skill::Vocabulary | Skill::Pronunciation => None,
    }
}

/// E16: the minimum content table of spec section 4.
fn check_minimum_content(unit: &Unit, out: &mut Vec<Finding>) {
    let count_type = |t: ActivityType| {
        unit.activities
            .iter()
            .filter(|a| a.activity_type() == t)
            .count()
    };
    let count_skill = |s: Skill| unit.activities.iter().filter(|a| a.skill() == s).count();
    let early = unit.level <= Level::B1;
    let mut need = |ok: bool, what: String| {
        if !ok {
            out.push(finding(RuleCode::E16, "/activities", what));
        }
    };

    let total = unit.activities.len();
    need(
        total >= 14,
        format!("a unit needs 14 or more activities, this one has {total}"),
    );

    let listening = count_skill(Skill::Listening);
    need(
        listening >= 2,
        format!("listening needs 2 activities with skill listening, found {listening}"),
    );
    need(
        count_type(ActivityType::ListeningSet) >= 1,
        "listening needs one listening_set".to_owned(),
    );

    let reading = count_skill(Skill::Reading);
    need(
        reading >= 2,
        format!("reading needs 2 activities with skill reading, found {reading}"),
    );
    need(
        count_type(ActivityType::ReadingSet) >= 1,
        "reading needs one reading_set".to_owned(),
    );

    need(
        count_type(ActivityType::GuidedSpeaking) >= 1,
        "speaking needs a guided_speaking activity".to_owned(),
    );
    need(
        count_type(ActivityType::Roleplay) >= 1,
        "speaking needs a roleplay activity".to_owned(),
    );

    let guided_writing = count_type(ActivityType::GuidedWriting);
    need(
        guided_writing >= 1,
        "writing needs a guided_writing activity".to_owned(),
    );
    need(
        count_type(ActivityType::ErrorCorrection) >= 1 || guided_writing >= 2,
        "writing needs one more task: an error_correction or a second guided_writing".to_owned(),
    );

    let (grammar_min, vocab_min) = if early { (2, 2) } else { (1, 1) };
    let grammar = count_skill(Skill::Grammar);
    need(
        grammar >= grammar_min,
        format!("grammar needs {grammar_min} activities with skill grammar, found {grammar}"),
    );
    let vocabulary = count_skill(Skill::Vocabulary);
    need(
        vocabulary >= vocab_min,
        format!(
            "vocabulary needs {vocab_min} activities with skill vocabulary, found {vocabulary}"
        ),
    );

    // From B2 the drill is required in every second unit (odd sequences) and mediation in
    // every third unit (sequences divisible by three). The spec states the frequency, not
    // which units, so the first unit of a level carries the drill and the third the mediation.
    let drill_required = early || unit.sequence % 2 == 1;
    let drills = count_type(ActivityType::ReadAloud) + count_type(ActivityType::MinimalPairs);
    need(
        !drill_required || drills >= 1,
        "a pronunciation drill (read_aloud or minimal_pairs) is required in this unit".to_owned(),
    );
    let mediation_required = !early && unit.sequence % 3 == 0;
    need(
        !mediation_required || count_type(ActivityType::Mediation) >= 1,
        "a mediation activity is required in this unit".to_owned(),
    );

    check_checkpoint_coverage(unit, out);
}

fn check_checkpoint_coverage(unit: &Unit, out: &mut Vec<Finding>) {
    let path = "/checkpoint/activity_ids";
    let size = unit.checkpoint.activity_ids.len();
    if size < 6 {
        out.push(finding(
            RuleCode::E16,
            path,
            format!("the checkpoint needs 6 or more activities, it has {size}"),
        ));
    }
    let in_checkpoint: Vec<&Activity> = unit
        .checkpoint
        .activity_ids
        .iter()
        .filter_map(|id| unit.activities.iter().find(|a| a.id() == id))
        .collect();
    let has = |skill: CoreSkill, productive: &[ActivityType]| {
        let of_skill: Vec<&&Activity> = in_checkpoint
            .iter()
            .filter(|a| core_skill(a.skill()) == Some(skill))
            .collect();
        (
            !of_skill.is_empty(),
            productive.is_empty()
                || of_skill
                    .iter()
                    .any(|a| productive.contains(&a.activity_type())),
        )
    };
    let rows = [
        ("listening", CoreSkill::Listening, &[][..]),
        ("reading", CoreSkill::Reading, &[][..]),
        (
            "speaking",
            CoreSkill::Speaking,
            &[ActivityType::GuidedSpeaking, ActivityType::Roleplay][..],
        ),
        (
            "writing",
            CoreSkill::Writing,
            &[ActivityType::GuidedWriting][..],
        ),
    ];
    for (name, skill, productive) in rows {
        let (any, productive_ok) = has(skill, productive);
        if !any {
            out.push(finding(
                RuleCode::E16,
                path,
                format!("the checkpoint has no {name} activity"),
            ));
        } else if !productive_ok {
            out.push(finding(
                RuleCode::E16,
                path,
                format!("the checkpoint needs a productive {name} task (guided_speaking or roleplay for speaking, guided_writing for writing)"),
            ));
        }
    }
}

/// E15 is done per activity and per dialogue elsewhere. This covers the checks of E17 and
/// E18 that need the unit: the dialogue of a listening set, the audio source, passage length.
fn check_set_context(unit: &Unit, out: &mut Vec<Finding>) {
    let dialogues: HashSet<&str> = unit.dialogues.iter().map(|d| d.id.as_str()).collect();
    let (min_words, max_words) = reading_range(unit.level);
    for (i, activity) in unit.activities.iter().enumerate() {
        let path = format!("/activities/{i}");
        match activity {
            Activity::ListeningSet(a) => {
                if a.audio_text.is_some() == a.dialogue_id.is_some() {
                    out.push(finding(
                        RuleCode::E17,
                        &path,
                        "a listening_set needs exactly one of audio_text and dialogue_id",
                    ));
                }
                match &a.dialogue_id {
                    Some(id) if !dialogues.contains(id.as_str()) => out.push(finding(
                        RuleCode::E17,
                        format!("{path}/dialogue_id"),
                        format!("there is no dialogue \"{id}\""),
                    )),
                    _ => {}
                }
            }
            Activity::ReadingSet(a) => {
                let words = word_count(&a.passage);
                if words < min_words || words > max_words {
                    out.push(finding(
                        RuleCode::E18,
                        format!("{path}/passage"),
                        format!(
                            "the passage has {words} words, {} passages have {min_words} to {max_words}",
                            unit.level
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
}
