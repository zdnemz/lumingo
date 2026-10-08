//! Rules across units, X01 to X06 of `docs/CURRICULUM_SPEC.md` section 6.
//!
//! Each rule is its own function over the set of loaded units, so a caller can run the ones
//! its inputs allow. Findings that concern one unit carry its index in the input slice.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::report::{Finding, RuleCode};
use super::text::{lowercase_words, sentences};
use super::texts::{TextScope, unit_texts};
use crate::model::{Activity, Level, Unit};
use crate::syllabus::Syllabus;

/// Sentences up to this many words may repeat between units (rule X04).
const MAX_SHARED_SENTENCE_WORDS: usize = 6;

/// One unit of the set, with the file it came from.
#[derive(Clone, Copy)]
pub struct UnitRef<'a> {
    pub file: &'a str,
    pub unit: &'a Unit,
}

/// A finding, tied to the unit it is about when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// Index into the slice of units that was checked, or `None` for a whole-set finding.
    pub unit: Option<usize>,
    pub finding: Finding,
}

fn about_unit(
    index: usize,
    code: RuleCode,
    path: impl Into<String>,
    message: impl Into<String>,
) -> Located {
    Located {
        unit: Some(index),
        finding: Finding::new(code, path, message),
    }
}

fn about_set(code: RuleCode, message: impl Into<String>) -> Located {
    Located {
        unit: None,
        finding: Finding::new(code, "", message),
    }
}

/// X01: each level's sequences have no gap and no repeat.
///
/// With `complete` false (units are still being written) a level must hold 1 to its highest
/// sequence without gaps. With `complete` true every level that has a unit must hold all 30.
pub fn check_sequences(units: &[UnitRef<'_>], complete: bool) -> Vec<Located> {
    let mut out = Vec::new();
    let mut by_level: BTreeMap<Level, BTreeMap<u8, usize>> = BTreeMap::new();
    for (index, entry) in units.iter().enumerate() {
        let level = by_level.entry(entry.unit.level).or_default();
        match level.get(&entry.unit.sequence) {
            Some(first) => out.push(about_unit(
                index,
                RuleCode::X01,
                "/sequence",
                format!(
                    "sequence {} of level {} is already used by {}",
                    entry.unit.sequence, entry.unit.level, units[*first].file
                ),
            )),
            None => {
                level.insert(entry.unit.sequence, index);
            }
        }
    }
    for (level, sequences) in &by_level {
        let highest = sequences.keys().next_back().copied().unwrap_or(0);
        let last = if complete { 30 } else { highest };
        let missing: Vec<String> = (1..=last)
            .filter(|n| !sequences.contains_key(n))
            .map(|n| n.to_string())
            .collect();
        if !missing.is_empty() {
            out.push(about_set(
                RuleCode::X01,
                format!(
                    "level {level} has no unit with sequence {}; sequences must run from 1 to {last} without gaps",
                    missing.join(", ")
                ),
            ));
        }
    }
    out
}

/// X02: every prerequisite exists in the set and comes earlier than the unit.
pub fn check_prerequisites(units: &[UnitRef<'_>]) -> Vec<Located> {
    let known: HashSet<&str> = units.iter().map(|u| u.unit.id.as_str()).collect();
    let mut out = Vec::new();
    for (index, entry) in units.iter().enumerate() {
        let unit = entry.unit;
        for (j, prerequisite) in unit.prerequisites.iter().enumerate() {
            let path = format!("/prerequisites/{j}");
            match parse_unit_id(prerequisite) {
                Some(position) if position >= (unit.level, unit.sequence) => out.push(about_unit(
                    index,
                    RuleCode::X02,
                    &path,
                    format!("prerequisite \"{prerequisite}\" does not come before this unit"),
                )),
                _ => {}
            }
            if !known.contains(prerequisite.as_str()) {
                out.push(about_unit(
                    index,
                    RuleCode::X02,
                    &path,
                    format!("prerequisite \"{prerequisite}\" is not among the validated units"),
                ));
            }
        }
    }
    out
}

/// Level and sequence of a unit id such as `a2-u07`.
fn parse_unit_id(id: &str) -> Option<(Level, u8)> {
    let (level, sequence) = id.split_once("-u")?;
    Some((Level::parse(level)?, sequence.parse().ok()?))
}

/// X03: the unit matches its syllabus entry in theme, objective count and grammar ids.
/// Units whose level has no syllabus are left out; the caller reports that as skipped.
pub fn check_against_syllabus(
    units: &[UnitRef<'_>],
    syllabi: &HashMap<Level, Syllabus>,
) -> Vec<Located> {
    let mut out = Vec::new();
    for (index, entry) in units.iter().enumerate() {
        let unit = entry.unit;
        let Some(syllabus) = syllabi.get(&unit.level) else {
            continue;
        };
        let Some(planned) = syllabus.entry(&unit.id) else {
            out.push(about_unit(
                index,
                RuleCode::X03,
                "/id",
                format!(
                    "the {} syllabus has no entry for \"{}\"",
                    unit.level, unit.id
                ),
            ));
            continue;
        };
        if planned.theme != unit.theme {
            out.push(about_unit(
                index,
                RuleCode::X03,
                "/theme",
                format!(
                    "theme is \"{}\" but the syllabus plans \"{}\"",
                    unit.theme, planned.theme
                ),
            ));
        }
        if planned.objectives.len() != unit.objectives.len() {
            out.push(about_unit(
                index,
                RuleCode::X03,
                "/objectives",
                format!(
                    "the unit has {} objectives but the syllabus plans {}",
                    unit.objectives.len(),
                    planned.objectives.len()
                ),
            ));
        }
        let in_unit: BTreeSet<&str> = unit.targets.grammar.iter().map(|g| g.id.as_str()).collect();
        let in_plan: BTreeSet<&str> = planned.grammar_ids.iter().map(String::as_str).collect();
        if in_unit != in_plan {
            let join = |set: &BTreeSet<&str>| set.iter().copied().collect::<Vec<_>>().join(", ");
            out.push(about_unit(
                index,
                RuleCode::X03,
                "/targets/grammar",
                format!(
                    "grammar ids are [{}] but the syllabus plans [{}]",
                    join(&in_unit),
                    join(&in_plan)
                ),
            ));
        }
    }
    out
}

/// X04: no sentence of more than six words appears in two different units.
/// Only content is compared; instructions and explanations are shared boilerplate.
/// The finding is reported on the later unit and names the earlier one.
pub fn check_repeated_sentences(units: &[UnitRef<'_>]) -> Vec<Located> {
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by(|a, b| units[*a].unit.id.cmp(&units[*b].unit.id));
    let mut first_seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for index in order {
        let unit = units[index].unit;
        let mut reported: HashSet<String> = HashSet::new();
        let mut own: HashSet<String> = HashSet::new();
        for (path, text) in unit_texts(unit, TextScope::Content) {
            for sentence in sentences(text) {
                let words = lowercase_words(sentence);
                if words.len() <= MAX_SHARED_SENTENCE_WORDS {
                    continue;
                }
                let key = words.join(" ");
                match first_seen.get(&key) {
                    Some(owner) if *owner != index && reported.insert(key.clone()) => {
                        out.push(about_unit(
                            index,
                            RuleCode::X04,
                            &path,
                            format!(
                                "the sentence \"{}\" also appears in unit {}",
                                sentence.trim(),
                                units[*owner].unit.id
                            ),
                        ));
                    }
                    _ => {}
                }
                own.insert(key);
            }
        }
        for key in own {
            first_seen.entry(key).or_insert(index);
        }
    }
    out
}

/// X05: every grammar id of a level's syllabus is taught in at least one unit of the level and
/// practised in at least two. A unit teaches and practises the ids it lists in
/// `targets.grammar`; rule E10 keeps its roleplay and E12 its generation policy inside that
/// list, so a second unit that lists the id again is the only way to practise it elsewhere.
pub fn check_grammar_coverage(
    units: &[UnitRef<'_>],
    syllabi: &HashMap<Level, Syllabus>,
) -> Vec<Located> {
    let mut out = Vec::new();
    let mut levels: Vec<&Level> = syllabi.keys().collect();
    levels.sort();
    for level in levels {
        let Some(syllabus) = syllabi.get(level) else {
            continue;
        };
        let planned: BTreeSet<&str> = syllabus
            .units
            .iter()
            .flat_map(|entry| entry.grammar_ids.iter().map(String::as_str))
            .collect();
        for id in planned {
            let using = units
                .iter()
                .filter(|u| u.unit.level == *level)
                .filter(|u| u.unit.targets.grammar.iter().any(|g| g.id == id))
                .count();
            if using == 0 {
                out.push(about_set(
                    RuleCode::X05,
                    format!("level {level}: grammar id \"{id}\" is taught in no unit"),
                ));
            } else if using < 2 {
                out.push(about_set(
                    RuleCode::X05,
                    format!("level {level}: grammar id \"{id}\" is practised in only one unit, two are needed"),
                ));
            }
        }
    }
    out
}

/// X06: every `rubric_id` names a rubric of `catalogs/rubrics/`.
pub fn check_rubric_ids(units: &[UnitRef<'_>], rubric_ids: &HashSet<String>) -> Vec<Located> {
    let mut out = Vec::new();
    for (index, entry) in units.iter().enumerate() {
        for (i, activity) in entry.unit.activities.iter().enumerate() {
            let rubric_id = match activity {
                Activity::GuidedSpeaking(a) | Activity::GuidedWriting(a) => &a.rubric_id,
                Activity::Mediation(a) => &a.rubric_id,
                _ => continue,
            };
            if !rubric_ids.contains(rubric_id) {
                out.push(about_unit(
                    index,
                    RuleCode::X06,
                    format!("/activities/{i}/rubric_id"),
                    format!("there is no rubric \"{rubric_id}\" in catalogs/rubrics"),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DialogueTurn;

    /// The example unit with its activities and presentation removed and its dialogue replaced,
    /// so the only English text left is the given lines and the short vocabulary examples.
    fn unit_with_lines(id: &str, sequence: u8, lines: &[&str]) -> Unit {
        let mut unit: Unit = serde_json::from_str(include_str!(
            "../../../../curriculum/examples/a1-u01.example.json"
        ))
        .expect("the example unit is valid");
        unit.id = id.to_owned();
        unit.sequence = sequence;
        unit.activities.clear();
        unit.presentation.clear();
        for item in &mut unit.targets.vocabulary {
            item.example = "Hi.".to_owned();
        }
        unit.dialogues[0].turns = lines
            .iter()
            .map(|line| DialogueTurn {
                speaker: "A".to_owned(),
                text: (*line).to_owned(),
            })
            .collect();
        unit
    }

    fn refs<'a>(a: &'a Unit, b: &'a Unit) -> Vec<UnitRef<'a>> {
        vec![
            UnitRef {
                file: "a.json",
                unit: a,
            },
            UnitRef {
                file: "b.json",
                unit: b,
            },
        ]
    }

    #[test]
    fn a_sentence_of_six_words_may_be_shared() {
        let a = unit_with_lines("a1-u01", 1, &["I am from a small town.", "Hello."]);
        let b = unit_with_lines("a1-u02", 2, &["I am from a small town.", "Goodbye."]);
        assert_eq!(a.dialogues[0].turns[0].text.split_whitespace().count(), 6);
        assert!(check_repeated_sentences(&refs(&a, &b)).is_empty());
    }

    #[test]
    fn a_sentence_of_seven_words_may_not_be_shared() {
        let a = unit_with_lines("a1-u01", 1, &["I am from a very small town."]);
        let b = unit_with_lines("a1-u02", 2, &["Hello.", "i am FROM a very small town!"]);
        let found = check_repeated_sentences(&refs(&a, &b));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].unit, Some(1));
        assert_eq!(found[0].finding.path, "/dialogues/0/turns/1/text");
        assert!(found[0].finding.message.contains("a1-u01"));
    }

    #[test]
    fn the_order_of_the_input_does_not_decide_which_unit_is_blamed() {
        let a = unit_with_lines("a1-u01", 1, &["I am from a very small town."]);
        let b = unit_with_lines("a1-u02", 2, &["I am from a very small town."]);
        let found = check_repeated_sentences(&refs(&b, &a));
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].unit,
            Some(0),
            "index 0 is now the later unit, a1-u02"
        );
    }

    #[test]
    fn repeating_a_sentence_inside_one_unit_is_fine() {
        let a = unit_with_lines(
            "a1-u01",
            1,
            &[
                "I am from a very small town.",
                "I am from a very small town.",
            ],
        );
        let b = unit_with_lines("a1-u02", 2, &["Hello."]);
        assert!(check_repeated_sentences(&refs(&a, &b)).is_empty());
    }
}
