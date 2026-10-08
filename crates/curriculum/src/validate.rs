//! Content validators (CURRICULUM_SPEC section 6). E01 is the schema check done
//! by the loader. These are the rules a schema cannot express, over a loaded
//! `Unit`, plus the cross-unit rules over a set of units.
//!
//! Not implemented yet, because their inputs do not exist in the repository:
//! W01 and W02 (word lists), X03 and X05 (syllabus files), X06 (rubric catalog),
//! and the "every second unit" and "every third unit" cadence for B2 and above,
//! which is a property of a whole level and is reported by neither function.
//! W03 runs only when the caller links a [`GrammarCheck`]; without one the report
//! says `skipped` (see [`Severity::Skipped`]), never that the answers are clean.

use crate::{Activity, Level, Localized, ModelBand, Scoring, Skill, Unit};
use assessment_engine::{
    deterministic::normalize,
    text_metrics::{tokenize, word_count},
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    /// A rule that needs input the caller did not link (W03 without a grammar
    /// checker). A skipped rule is never a pass, and the report must say so.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub unit: String,
    /// A JSON pointer into the unit, or empty for the unit as a whole.
    pub path: String,
    pub message: String,
}

struct Sink<'a> {
    unit: &'a str,
    out: Vec<Diagnostic>,
}

impl Sink<'_> {
    fn error(&mut self, code: &'static str, path: impl Into<String>, message: impl Into<String>) {
        self.out.push(Diagnostic {
            code,
            severity: Severity::Error,
            unit: self.unit.to_owned(),
            path: path.into(),
            message: message.into(),
        });
    }

    fn warning(&mut self, code: &'static str, path: impl Into<String>, message: impl Into<String>) {
        self.out.push(Diagnostic {
            code,
            severity: Severity::Warning,
            unit: self.unit.to_owned(),
            path: path.into(),
            message: message.into(),
        });
    }

    fn skipped(&mut self, code: &'static str, message: impl Into<String>) {
        self.out.push(Diagnostic {
            code,
            severity: Severity::Skipped,
            unit: self.unit.to_owned(),
            path: String::new(),
            message: message.into(),
        });
    }
}

/// The rule-based grammar checker W03 runs against `at` model answers. The
/// caller links one (`assessment-engine` wraps harper-core behind its `grammar`
/// feature); this crate does not depend on it, so a build without a checker
/// reports W03 as skipped instead of reading an empty list as "clean". `&mut`
/// because building a checker loads a dictionary and linting takes it mutably.
pub trait GrammarCheck {
    /// One string per finding, in the checker's own words.
    fn findings(&mut self, text: &str) -> Vec<String>;
}

/// Inclusive word ranges per level (section 4). Starting values, tuned with the pilot units.
fn range(table: [(usize, usize); 6], level: Level) -> (usize, usize) {
    table[level as usize]
}

const READING_WORDS: [(usize, usize); 6] = [
    (60, 100),
    (100, 180),
    (180, 300),
    (300, 450),
    (450, 650),
    (600, 800),
];
const LISTENING_WORDS: [(usize, usize); 6] = [
    (25, 60),
    (50, 100),
    (90, 160),
    (140, 220),
    (180, 280),
    (220, 320),
];
const WRITING_WORDS: [(usize, usize); 6] = [
    (8, 40),
    (35, 80),
    (80, 150),
    (150, 250),
    (220, 320),
    (280, 400),
];

/// The error categories of `contracts/turn_analysis.schema.json`, read at start-up from the bundled contract.
fn error_categories() -> Vec<String> {
    const CONTRACT: &str = include_str!("../../../contracts/turn_analysis.schema.json");
    serde_json::from_str::<serde_json::Value>(CONTRACT)
        .ok()
        .and_then(|v| {
            v.pointer("/properties/turns/items/properties/errors/items/properties/category/enum")
                .cloned()
        })
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn duplicates<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    ids.filter(|id| !seen.insert(*id)).collect()
}

/// Every localized text in the unit with its path, for rules E14 and W05.
fn localized_texts(u: &Unit) -> Vec<(String, &Localized)> {
    let mut v: Vec<(String, &Localized)> = vec![("/title".into(), &u.title)];
    v.extend(
        u.objectives
            .iter()
            .enumerate()
            .map(|(i, o)| (format!("/objectives/{i}/can_do"), &o.can_do)),
    );
    v.extend(
        u.targets
            .grammar
            .iter()
            .enumerate()
            .map(|(i, g)| (format!("/targets/grammar/{i}/note"), &g.note)),
    );
    for (i, b) in u.presentation.iter().enumerate() {
        v.push((format!("/presentation/{i}/text"), &b.text));
        v.extend(
            b.examples
                .iter()
                .enumerate()
                .map(|(j, e)| (format!("/presentation/{i}/examples/{j}"), e)),
        );
    }
    v.extend(
        u.dialogues
            .iter()
            .enumerate()
            .map(|(i, d)| (format!("/dialogues/{i}/context"), &d.context)),
    );
    for (i, a) in u.activities.iter().enumerate() {
        let p = format!("/activities/{i}");
        v.push((format!("{p}/instructions"), &a.common().instructions));
        match a {
            Activity::Mcq { explanation, .. }
            | Activity::GapFill { explanation, .. }
            | Activity::ErrorCorrection { explanation, .. } => {
                v.push((format!("{p}/explanation"), explanation))
            }
            Activity::Reorder {
                explanation: Some(e),
                ..
            } => v.push((format!("{p}/explanation"), e)),
            Activity::GuidedSpeaking(g) | Activity::GuidedWriting(g) => {
                v.push((format!("{p}/prompt"), &g.prompt))
            }
            Activity::Roleplay { scenario, .. } => v.push((format!("{p}/scenario"), scenario)),
            Activity::Mediation { task, .. } => v.push((format!("{p}/task"), task)),
            Activity::ReadingSet { questions, .. } | Activity::ListeningSet { questions, .. } => {
                v.extend(
                    questions
                        .iter()
                        .enumerate()
                        .map(|(j, q)| (format!("{p}/questions/{j}/explanation"), &q.explanation)),
                );
            }
            _ => {}
        }
    }
    v
}

/// Text that the speech engine will read aloud, with its path (rule 14).
fn spoken_texts(u: &Unit) -> Vec<(String, &str)> {
    let mut v = Vec::new();
    for (i, d) in u.dialogues.iter().enumerate() {
        v.extend(
            d.turns
                .iter()
                .enumerate()
                .map(|(j, t)| (format!("/dialogues/{i}/turns/{j}/text"), t.text.as_str())),
        );
    }
    for (i, a) in u.activities.iter().enumerate() {
        let p = format!("/activities/{i}");
        match a {
            Activity::Mcq {
                audio_text: Some(t),
                ..
            }
            | Activity::ListeningSet {
                audio_text: Some(t),
                ..
            } => v.push((format!("{p}/audio_text"), t)),
            Activity::Dictation { audio_text, .. } => {
                v.push((format!("{p}/audio_text"), audio_text))
            }
            Activity::ReadAloud { text, .. } => v.push((format!("{p}/text"), text)),
            _ => {}
        }
    }
    v
}

fn is_tts_safe(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_alphanumeric() || c == ' ' || matches!(c, '.' | ',' | '?' | '!' | '\'' | '-'))
}

fn skill_group(s: Skill) -> Option<&'static str> {
    match s {
        Skill::Listening => Some("listening"),
        Skill::Reading => Some("reading"),
        Skill::SpeakingProduction | Skill::SpeakingInteraction => Some("speaking"),
        Skill::Writing => Some("writing"),
        _ => None,
    }
}

/// What the per-unit rules may use besides the unit itself.
#[derive(Default)]
pub struct UnitOptions<'a> {
    /// The rule-based checker for W03. Without it, W03 is reported as skipped.
    pub grammar_check: Option<&'a mut dyn GrammarCheck>,
}

/// Runs the per-unit rules E02 to E20 and the warnings W03 to W07. W03 is
/// skipped because no grammar checker is linked; use [`validate_unit_with`] to
/// link one.
pub fn validate_unit(u: &Unit) -> Vec<Diagnostic> {
    validate_unit_with(u, UnitOptions::default())
}

/// Runs the per-unit rules E02 to E20 and the warnings W03 to W07, with the
/// optional inputs the caller can link (CURRICULUM_SPEC section 6). A rule whose
/// input is not linked is reported as `Skipped`, never as a pass.
pub fn validate_unit_with(u: &Unit, mut options: UnitOptions<'_>) -> Vec<Diagnostic> {
    let mut s = Sink {
        unit: &u.id,
        out: Vec::new(),
    };
    let early = u.level <= Level::B1;

    // E02
    let expected = format!(
        "{}-u{:02}",
        format!("{:?}", u.level).to_lowercase(),
        u.sequence
    );
    if u.id != expected {
        s.error(
            "E02",
            "/id",
            format!(
                "the id should be `{expected}` for level {:?} sequence {}",
                u.level, u.sequence
            ),
        );
    }

    // E03
    let groups: [(&str, Vec<&str>); 6] = [
        (
            "objectives",
            u.objectives.iter().map(|o| o.id.as_str()).collect(),
        ),
        (
            "targets/vocabulary",
            u.targets.vocabulary.iter().map(|o| o.id.as_str()).collect(),
        ),
        (
            "targets/grammar",
            u.targets.grammar.iter().map(|o| o.id.as_str()).collect(),
        ),
        (
            "targets/pronunciation",
            u.targets
                .pronunciation
                .iter()
                .map(|o| o.id.as_str())
                .collect(),
        ),
        (
            "dialogues",
            u.dialogues.iter().map(|o| o.id.as_str()).collect(),
        ),
        (
            "activities",
            u.activities
                .iter()
                .map(|a| a.common().id.as_str())
                .collect(),
        ),
    ];
    for (name, ids) in &groups {
        for dup in duplicates(ids.iter().copied()) {
            s.error(
                "E03",
                format!("/{name}"),
                format!("the id `{dup}` is used twice"),
            );
        }
    }
    let objective_ids: HashSet<&str> = groups[0].1.iter().copied().collect();
    let vocab_ids: HashSet<&str> = groups[1].1.iter().copied().collect();
    let grammar_ids: HashSet<&str> = groups[2].1.iter().copied().collect();
    let pron_ids: HashSet<&str> = groups[3].1.iter().copied().collect();
    let dialogue_ids: HashSet<&str> = groups[4].1.iter().copied().collect();
    let activity_ids: HashSet<&str> = groups[5].1.iter().copied().collect();

    // E04
    let mut served = HashSet::new();
    for (i, a) in u.activities.iter().enumerate() {
        for o in &a.common().objective_ids {
            if objective_ids.contains(o.as_str()) {
                served.insert(o.as_str());
            } else {
                s.error(
                    "E04",
                    format!("/activities/{i}/objective_ids"),
                    format!("the objective `{o}` does not exist"),
                );
            }
        }
    }
    for (i, o) in u.objectives.iter().enumerate() {
        if !served.contains(o.id.as_str()) {
            s.error(
                "E04",
                format!("/objectives/{i}"),
                format!("no activity serves the objective `{}`", o.id),
            );
        }
    }

    let categories = error_categories();
    let (read_min, read_max) = range(READING_WORDS, u.level);
    let (listen_min, listen_max) = range(LISTENING_WORDS, u.level);
    let (write_min, write_max) = range(WRITING_WORDS, u.level);
    for (i, a) in u.activities.iter().enumerate() {
        let p = format!("/activities/{i}");
        match a {
            // E05
            Activity::Mcq {
                options,
                answer_index,
                passage,
                audio_text,
                ..
            } => {
                if usize::from(*answer_index) >= options.len() {
                    s.error(
                        "E05",
                        format!("{p}/answer_index"),
                        "answer_index is outside the options",
                    );
                }
                if passage.is_some() && audio_text.is_some() {
                    s.error(
                        "E05",
                        &p,
                        "an mcq has either a passage or audio_text, not both",
                    );
                }
            }
            // E06
            Activity::GapFill { text, answers, .. } => {
                let gaps = text.matches("___").count();
                if gaps != answers.len() {
                    s.error(
                        "E06",
                        &p,
                        format!(
                            "the text has {gaps} gaps but {} answer lists",
                            answers.len()
                        ),
                    );
                }
            }
            // E07
            Activity::Reorder { tokens, answer, .. } => {
                let mut given: Vec<&str> = tokens.iter().map(String::as_str).collect();
                let mut wanted: Vec<&str> = answer.split_whitespace().collect();
                let in_order = tokens.join(" ") == *answer;
                given.sort_unstable();
                wanted.sort_unstable();
                if given != wanted {
                    s.error("E07", &p, "the answer does not use exactly the tokens");
                }
                if in_order {
                    s.error("E07", &p, "the tokens are already in answer order");
                }
            }
            // E08
            Activity::Shadowing { dialogue_id, .. }
                if !dialogue_ids.contains(dialogue_id.as_str()) =>
            {
                s.error(
                    "E08",
                    format!("{p}/dialogue_id"),
                    format!("the dialogue `{dialogue_id}` does not exist"),
                );
            }
            // E09
            Activity::GuidedSpeaking(g) | Activity::GuidedWriting(g) => {
                check_bands(&mut s, &p, g.model_answers.iter().map(|m| m.band));
                if g.min_words >= g.max_words {
                    s.error("E09", &p, "min_words must be below max_words");
                }
                if let Some(at) = g.model_answers.iter().find(|m| m.band == ModelBand::At) {
                    let n = word_count(&at.text);
                    if (n as u32) < g.min_words || (n as u32) > g.max_words {
                        s.error(
                            "E09",
                            &p,
                            format!(
                                "the `at` answer has {n} words, outside {} to {}",
                                g.min_words, g.max_words
                            ),
                        );
                    }
                    // W06
                    if matches!(a, Activity::GuidedWriting(_)) && (n < write_min || n > write_max) {
                        s.warning("W06", &p, format!("the `at` answer has {n} words, the range for {:?} is {write_min} to {write_max}", u.level));
                    }
                }
            }
            Activity::Mediation { model_answers, .. } => {
                check_bands(&mut s, &p, model_answers.iter().map(|m| m.band))
            }
            // E10
            Activity::Roleplay {
                target_grammar_ids,
                target_vocab_ids,
                ..
            } => {
                for g in target_grammar_ids
                    .iter()
                    .filter(|g| !grammar_ids.contains(g.as_str()))
                {
                    s.error(
                        "E10",
                        format!("{p}/target_grammar_ids"),
                        format!("the grammar id `{g}` does not exist in the unit"),
                    );
                }
                for v in target_vocab_ids
                    .iter()
                    .filter(|v| !vocab_ids.contains(v.as_str()))
                {
                    s.error(
                        "E10",
                        format!("{p}/target_vocab_ids"),
                        format!("the vocabulary id `{v}` does not exist in the unit"),
                    );
                }
            }
            // E17, E18
            Activity::ReadingSet {
                passage, questions, ..
            } => {
                check_questions(
                    &mut s,
                    &p,
                    questions
                        .iter()
                        .map(|q| (q.stem.as_str(), q.options.len(), q.answer_index)),
                );
                let n = word_count(passage);
                if n < read_min || n > read_max {
                    s.error("E18", format!("{p}/passage"), format!("the passage has {n} words, the range for {:?} is {read_min} to {read_max}", u.level));
                }
            }
            Activity::ListeningSet {
                audio_text,
                dialogue_id,
                questions,
                ..
            } => {
                check_questions(
                    &mut s,
                    &p,
                    questions
                        .iter()
                        .map(|q| (q.stem.as_str(), q.options.len(), q.answer_index)),
                );
                match (audio_text, dialogue_id) {
                    (Some(_), Some(_)) | (None, None) => s.error(
                        "E17",
                        &p,
                        "a listening_set has exactly one of audio_text and dialogue_id",
                    ),
                    (None, Some(d)) if !dialogue_ids.contains(d.as_str()) => s.error(
                        "E17",
                        format!("{p}/dialogue_id"),
                        format!("the dialogue `{d}` does not exist"),
                    ),
                    _ => {}
                }
                // W07
                let words = match (audio_text, dialogue_id) {
                    (Some(t), None) => Some(word_count(t)),
                    (None, Some(d)) => u
                        .dialogues
                        .iter()
                        .find(|x| x.id == *d)
                        .map(|x| x.turns.iter().map(|t| word_count(&t.text)).sum()),
                    _ => None,
                };
                if let Some(n) = words.filter(|n| *n < listen_min || *n > listen_max) {
                    s.warning("W07", &p, format!("the audio text has {n} words, the range for {:?} is {listen_min} to {listen_max}", u.level));
                }
            }
            // E19, E20
            Activity::ErrorCorrection {
                sentence,
                accepted_answers,
                error_category,
                ..
            } => {
                if !categories.iter().any(|c| c == error_category) {
                    s.error(
                        "E19",
                        format!("{p}/error_category"),
                        format!(
                            "`{error_category}` is not a category of the turn_analysis contract"
                        ),
                    );
                }
                let given = normalize(sentence);
                if accepted_answers.iter().any(|a| normalize(a) == given) {
                    s.error(
                        "E20",
                        &p,
                        "the sentence equals an accepted answer after normalisation",
                    );
                }
            }
            _ => {}
        }
    }

    // E11
    for id in &u.checkpoint.activity_ids {
        match u.activities.iter().find(|a| a.common().id == *id) {
            None => s.error(
                "E11",
                "/checkpoint/activity_ids",
                format!("the activity `{id}` does not exist"),
            ),
            Some(a) if a.common().scoring == Some(Scoring::None) => s.error(
                "E11",
                "/checkpoint/activity_ids",
                format!("the activity `{id}` has scoring none"),
            ),
            Some(_) => {}
        }
    }
    // E12
    for g in u
        .generation_policy
        .allowed_grammar_ids
        .iter()
        .filter(|g| !grammar_ids.contains(g.as_str()))
    {
        s.error(
            "E12",
            "/generation_policy/allowed_grammar_ids",
            format!("the grammar id `{g}` does not exist in the unit"),
        );
    }
    // E13
    for (i, r) in u.review_items.iter().enumerate() {
        let known = match r.kind {
            crate::ReviewKind::Vocab => &vocab_ids,
            crate::ReviewKind::Grammar => &grammar_ids,
            crate::ReviewKind::Pron => &pron_ids,
        };
        if !known.contains(r.target.as_str()) {
            s.error(
                "E13",
                format!("/review_items/{i}"),
                format!("`{}` is not a {:?} target of the unit", r.target, r.kind),
            );
        }
    }
    // E14 and W05
    for (path, l) in localized_texts(u) {
        match &l.id {
            None if early => s.error("E14", &path, "an Indonesian text is required at A1 to B1"),
            Some(id) if early && id.trim().is_empty() => {
                s.error("E14", &path, "the Indonesian text is empty")
            }
            Some(id) if l.en.chars().count() >= 20 => {
                let ratio = id.chars().count() as f64 / l.en.chars().count() as f64;
                if !(0.5..=2.0).contains(&ratio) {
                    s.warning(
                        "W05",
                        &path,
                        format!(
                            "the Indonesian text is {ratio:.1} times the length of the English"
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    // E15
    for (path, text) in spoken_texts(u) {
        if !is_tts_safe(text) {
            s.error(
                "E15",
                &path,
                "spoken text may only use letters, digits and . , ? ! ' - (rule 14)",
            );
        }
    }
    for (i, d) in u.dialogues.iter().enumerate() {
        for (j, t) in d.turns.iter().enumerate() {
            let path = format!("/dialogues/{i}/turns/{j}/text");
            if t.text.chars().count() > 200 {
                s.error(
                    "E15",
                    &path,
                    "a dialogue line is at most 200 characters (rule 15)",
                );
            }
            if u.level == Level::A1 && word_count(&t.text) > 12 {
                s.error(
                    "E15",
                    &path,
                    "an A1 dialogue line is at most 12 words (rule 15)",
                );
            }
        }
    }
    check_minimum_content(&mut s, u, early, &activity_ids);
    check_at_answers(&mut s, u, options.grammar_check.take());
    check_near_duplicates(&mut s, u);
    s.out
}

/// E09: exactly one model answer per band.
fn check_bands(s: &mut Sink, path: &str, bands: impl Iterator<Item = ModelBand>) {
    let mut count: HashMap<ModelBand, usize> = HashMap::new();
    for b in bands {
        *count.entry(b).or_default() += 1;
    }
    for band in [ModelBand::Below, ModelBand::At, ModelBand::Above] {
        if count.get(&band) != Some(&1) {
            s.error(
                "E09",
                path,
                format!("there must be exactly one `{band:?}` model answer"),
            );
        }
    }
}

/// E17: answer indexes inside the options, and no two questions sharing a stem.
fn check_questions<'a>(
    s: &mut Sink,
    path: &str,
    questions: impl Iterator<Item = (&'a str, usize, u8)>,
) {
    let mut stems = HashSet::new();
    for (i, (stem, options, answer)) in questions.enumerate() {
        if usize::from(answer) >= options {
            s.error(
                "E17",
                format!("{path}/questions/{i}/answer_index"),
                "answer_index is outside the options",
            );
        }
        if !stems.insert(normalize(stem)) {
            s.error(
                "E17",
                format!("{path}/questions/{i}/stem"),
                "two questions share a stem",
            );
        }
    }
}

/// E16: the minimum content table of section 4, and one activity per skill in the checkpoint.
fn check_minimum_content(s: &mut Sink, u: &Unit, early: bool, activity_ids: &HashSet<&str>) {
    let acts = &u.activities;
    let count = |f: &dyn Fn(&Activity) -> bool| acts.iter().filter(|a| f(a)).count();
    let by_skill = |skill: Skill| count(&|a| a.common().skill == skill);
    let mut need = |ok: bool, what: &str| {
        if !ok {
            s.error(
                "E16",
                "/activities",
                format!("minimum content not met: {what}"),
            );
        }
    };
    need(acts.len() >= 14, "14 or more activities");
    need(
        by_skill(Skill::Listening) >= 2
            && count(&|a| matches!(a, Activity::ListeningSet { .. })) >= 1,
        "2 listening activities including a listening_set",
    );
    need(
        by_skill(Skill::Reading) >= 2 && count(&|a| matches!(a, Activity::ReadingSet { .. })) >= 1,
        "2 reading activities including a reading_set",
    );
    need(
        count(&|a| matches!(a, Activity::GuidedSpeaking(_))) >= 1
            && count(&|a| matches!(a, Activity::Roleplay { .. })) >= 1,
        "a guided_speaking and a roleplay",
    );
    let writing_tasks = count(&|a| matches!(a, Activity::GuidedWriting(_)));
    need(
        writing_tasks >= 1
            && writing_tasks + count(&|a| matches!(a, Activity::ErrorCorrection { .. })) >= 2,
        "a guided_writing and one more writing task",
    );
    let (min_items, _) = if early { (2, 2) } else { (1, 1) };
    need(by_skill(Skill::Grammar) >= min_items, "grammar items");
    need(by_skill(Skill::Vocabulary) >= min_items, "vocabulary items");
    if early {
        need(
            count(&|a| {
                matches!(
                    a,
                    Activity::ReadAloud { .. } | Activity::MinimalPairs { .. }
                )
            }) >= 1,
            "a pronunciation drill",
        );
    }
    // Checkpoint: 6 or more activities, at least one per skill, with productive speaking and writing.
    let cp: Vec<&Activity> = acts
        .iter()
        .filter(|a| u.checkpoint.activity_ids.contains(&a.common().id))
        .collect();
    if u.checkpoint
        .activity_ids
        .iter()
        .all(|id| activity_ids.contains(id.as_str()))
    {
        let mut need_cp = |ok: bool, what: &str| {
            if !ok {
                s.error("E16", "/checkpoint", format!("the checkpoint needs {what}"));
            }
        };
        need_cp(cp.len() >= 6, "6 or more activities");
        for group in ["listening", "reading", "speaking", "writing"] {
            need_cp(
                cp.iter()
                    .any(|a| skill_group(a.common().skill) == Some(group)),
                &format!("an activity for {group}"),
            );
        }
        need_cp(
            cp.iter()
                .any(|a| matches!(a, Activity::GuidedSpeaking(_) | Activity::Roleplay { .. })),
            "a productive speaking task",
        );
        need_cp(
            cp.iter()
                .any(|a| matches!(a, Activity::GuidedWriting(_) | Activity::Mediation { .. })),
            "a productive writing task",
        );
    }
}

/// W03: every `at` model answer of the productive tasks runs through the
/// rule-based grammar checker (CURRICULUM_SPEC section 6). Spelling is left out:
/// the unit is full of names and place names the dictionary does not know, and
/// the rule names the grammar checker, not the speller. Without a checker the
/// rule is reported as skipped, never as a pass.
fn check_at_answers(s: &mut Sink, u: &Unit, checker: Option<&mut dyn GrammarCheck>) {
    let Some(checker) = checker else {
        s.skipped(
            "W03",
            "no rule-based grammar checker is linked into this build",
        );
        return;
    };
    for (i, a) in u.activities.iter().enumerate() {
        let answers = match a {
            Activity::GuidedSpeaking(g) | Activity::GuidedWriting(g) => &g.model_answers,
            Activity::Mediation { model_answers, .. } => model_answers,
            _ => continue,
        };
        for (m, answer) in answers.iter().enumerate() {
            if answer.band != ModelBand::At {
                continue;
            }
            let findings = checker.findings(&answer.text);
            if !findings.is_empty() {
                s.warning(
                    "W03",
                    format!("/activities/{i}/model_answers/{m}/text"),
                    format!(
                        "the grammar checker reports {} finding(s) in the `at` answer: {}",
                        findings.len(),
                        findings.join("; ")
                    ),
                );
            }
        }
    }
}

/// The texts of one activity that W04 compares, with the field each came from.
/// Empty for types with no free text of their own (`match`, `minimal_pairs`)
/// and for a `listening_set` that plays a dialogue: the dialogue's text belongs
/// to the `shadowing` activities that point at it, not to the set.
fn comparison_texts(a: &Activity) -> Vec<(&'static str, &str)> {
    match a {
        Activity::Mcq {
            stem,
            passage,
            audio_text,
            ..
        } => {
            let mut v = vec![("stem", stem.as_str())];
            v.extend(passage.as_deref().map(|p| ("passage", p)));
            v.extend(audio_text.as_deref().map(|t| ("audio_text", t)));
            v
        }
        Activity::GapFill { text, .. } => vec![("text", text)],
        Activity::Reorder { answer, .. } => vec![("answer", answer)],
        Activity::Dictation { audio_text, .. } => vec![("audio_text", audio_text)],
        Activity::ReadAloud { text, .. } => vec![("text", text)],
        Activity::GuidedSpeaking(g) | Activity::GuidedWriting(g) => vec![("prompt", &g.prompt.en)],
        Activity::Roleplay { scenario, .. } => vec![("scenario", &scenario.en)],
        Activity::Mediation { source_text, .. } => vec![("source_text", source_text)],
        Activity::ReadingSet { passage, .. } => vec![("passage", passage)],
        Activity::ListeningSet {
            audio_text: Some(t),
            ..
        } => vec![("audio_text", t)],
        Activity::ErrorCorrection { sentence, .. } => vec![("sentence", sentence)],
        _ => Vec::new(),
    }
}

/// Two texts count as nearly equal for W04 when their word sets agree in at
/// least this share (Jaccard). Texts with fewer than four distinct words are
/// compared only by equality: on two or three words, any overlap is noise.
const NEAR_DUPLICATE_SIMILARITY: f64 = 0.9;

/// W04: two activities must not share nearly the same stem or text
/// (CURRICULUM_SPEC section 6). Words are compared in the `norm/1` form, so
/// contractions and punctuation do not hide a copy.
fn check_near_duplicates(s: &mut Sink, u: &Unit) {
    let texts: Vec<(usize, &'static str, &str)> = u
        .activities
        .iter()
        .enumerate()
        .flat_map(|(i, a)| comparison_texts(a).into_iter().map(move |(f, t)| (i, f, t)))
        .collect();
    for (n, (i, field, text)) in texts.iter().enumerate() {
        for (j, other_field, other) in &texts[n + 1..] {
            if *j == *i {
                continue;
            }
            let Some(duplicate) = nearly_equal(text, other) else {
                continue;
            };
            let earlier = format!(
                "`/activities/{i}/{field}` (`{}`)",
                u.activities[*i].common().id
            );
            let message = match duplicate {
                Duplicate::Same => format!("the text duplicates {earlier}"),
                Duplicate::Nearly(similarity) => format!(
                    "the text nearly duplicates {earlier}, word-set overlap {similarity:.2}"
                ),
            };
            s.warning("W04", format!("/activities/{j}/{other_field}"), message);
        }
    }
}

/// How two texts are the same for W04.
enum Duplicate {
    /// Equal after `norm/1`.
    Same,
    /// The share of the combined word sets they have in common.
    Nearly(f64),
}

/// How `a` and `b` duplicate each other, or `None`. The words are compared
/// after `norm/1`, then as token sets.
fn nearly_equal(a: &str, b: &str) -> Option<Duplicate> {
    let wa = normalize(a);
    let wb = normalize(b);
    if wa == wb {
        return Some(Duplicate::Same);
    }
    let ta: HashSet<String> = tokenize(&wa).into_iter().collect();
    let tb: HashSet<String> = tokenize(&wb).into_iter().collect();
    if ta.len() < 4 || tb.len() < 4 {
        return None;
    }
    let shared = ta.intersection(&tb).count();
    let similarity = shared as f64 / ta.union(&tb).count() as f64;
    (similarity >= NEAR_DUPLICATE_SIMILARITY).then_some(Duplicate::Nearly(similarity))
}

/// Which cross-unit rules to run.
#[derive(Debug, Clone, Copy, Default)]
pub struct SetOptions {
    /// X01 is only meaningful for a finished curriculum: each level must then have units 1 to 30.
    pub require_complete: bool,
}

/// Cross-unit rules: X01 (with `require_complete`), X02 and X04.
pub fn validate_set(units: &[Unit], options: SetOptions) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut err = |code: &'static str, unit: &str, message: String| {
        out.push(Diagnostic {
            code,
            severity: Severity::Error,
            unit: unit.to_owned(),
            path: String::new(),
            message,
        });
    };
    let by_id: HashMap<&str, &Unit> = units.iter().map(|u| (u.id.as_str(), u)).collect();
    // X01
    if options.require_complete {
        let mut seqs: BTreeMap<Level, Vec<u32>> = BTreeMap::new();
        for u in units {
            seqs.entry(u.level).or_default().push(u.sequence);
        }
        for level in [
            Level::A1,
            Level::A2,
            Level::B1,
            Level::B2,
            Level::C1,
            Level::C2,
        ] {
            let have = seqs.get(&level).cloned().unwrap_or_default();
            for n in 1..=30u32 {
                match have.iter().filter(|s| **s == n).count() {
                    1 => {}
                    0 => err(
                        "X01",
                        "",
                        format!("{level:?} has no unit with sequence {n}"),
                    ),
                    _ => err(
                        "X01",
                        "",
                        format!("{level:?} has more than one unit with sequence {n}"),
                    ),
                }
            }
        }
    }
    // X02
    for u in units {
        for p in &u.prerequisites {
            match by_id.get(p.as_str()) {
                None => err(
                    "X02",
                    &u.id,
                    format!("the prerequisite `{p}` does not exist"),
                ),
                Some(q) if (q.level, q.sequence) >= (u.level, u.sequence) => err(
                    "X02",
                    &u.id,
                    format!("the prerequisite `{p}` is not an earlier unit"),
                ),
                Some(_) => {}
            }
        }
    }
    // X04: a sentence of more than six words must not appear in two different units
    let mut seen: HashMap<String, &str> = HashMap::new();
    let mut reported: HashSet<(String, String)> = HashSet::new();
    for u in units {
        let mut mine = HashSet::new();
        for text in long_sentences(u) {
            mine.insert(text);
        }
        for sentence in mine {
            match seen.get(&sentence) {
                Some(first)
                    if *first != u.id && reported.insert((first.to_string(), u.id.clone())) =>
                {
                    err(
                        "X04",
                        &u.id,
                        format!(
                            "a sentence of more than six words also appears in `{first}`: \"{}\"",
                            sentence.chars().take(60).collect::<String>()
                        ),
                    );
                }
                None => {
                    seen.insert(sentence, &u.id);
                }
                _ => {}
            }
        }
    }
    out
}

/// Sentences of more than six words from the unit's English texts, normalised for comparison.
fn long_sentences(u: &Unit) -> Vec<String> {
    let mut texts: Vec<&str> = spoken_texts(u).into_iter().map(|(_, t)| t).collect();
    for a in &u.activities {
        match a {
            Activity::ReadingSet { passage, .. } => texts.push(passage),
            Activity::Mcq {
                passage: Some(p), ..
            } => texts.push(p),
            _ => {}
        }
    }
    texts.extend(u.targets.vocabulary.iter().map(|v| v.example.as_str()));
    texts
        .into_iter()
        .flat_map(|t| {
            t.split(['.', '!', '?'])
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .map(|s| tokenize(&s))
        .filter(|w| w.len() > 6)
        .map(|w| w.join(" "))
        .collect()
}
