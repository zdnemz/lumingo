//! Collecting the text of a unit for the rules that read it as a whole.

use serde_json::Value;

use crate::model::{Activity, Localized, Unit};

/// One localized text found in a document: where it is, its English text, its Indonesian text.
pub struct LocalizedAt<'a> {
    pub pointer: String,
    pub en: &'a str,
    pub id: Option<&'a str>,
}

/// Every localized object of `document`, sorted by pointer. An object is localized when it
/// has a string `en` and no key other than `en` and `id`. Walking the JSON instead of the
/// typed unit means a localized field added to the schema later is covered at once.
pub fn localized_texts(document: &Value) -> Vec<LocalizedAt<'_>> {
    let mut found = Vec::new();
    let mut stack = vec![(String::new(), document)];
    while let Some((pointer, value)) = stack.pop() {
        match value {
            Value::Object(map) => {
                let english = map.get("en").and_then(Value::as_str);
                match english {
                    Some(en) if map.keys().all(|k| k == "en" || k == "id") => {
                        found.push(LocalizedAt {
                            pointer,
                            en,
                            id: map.get("id").and_then(Value::as_str),
                        });
                    }
                    _ => {
                        for (key, child) in map {
                            let escaped = key.replace('~', "~0").replace('/', "~1");
                            stack.push((format!("{pointer}/{escaped}"), child));
                        }
                    }
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    stack.push((format!("{pointer}/{i}"), child));
                }
            }
            _ => {}
        }
    }
    found.sort_by(|a, b| a.pointer.cmp(&b.pointer));
    found
}

/// Which English text of a unit to collect.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextScope {
    /// Everything a learner reads or hears, including instructions and explanations.
    Everything,
    /// Only content that could be copied between units: dialogues, passages, audio texts,
    /// stems, sentences, examples and model answers. Instructions and explanations are
    /// boilerplate that units legitimately share.
    Content,
}

/// English strings of the unit with their JSON pointers.
pub fn unit_texts(unit: &Unit, scope: TextScope) -> Vec<(String, &str)> {
    let everything = scope == TextScope::Everything;
    let mut out: Vec<(String, &str)> = Vec::new();

    for (d, dialogue) in unit.dialogues.iter().enumerate() {
        for (t, turn) in dialogue.turns.iter().enumerate() {
            out.push((format!("/dialogues/{d}/turns/{t}/text"), &turn.text));
        }
    }
    for (p, block) in unit.presentation.iter().enumerate() {
        if everything {
            out.push((format!("/presentation/{p}/text/en"), &block.text.en));
        }
        for (e, example) in block.examples.iter().flatten().enumerate() {
            out.push((format!("/presentation/{p}/examples/{e}/en"), &example.en));
        }
    }
    for (v, item) in unit.targets.vocabulary.iter().enumerate() {
        out.push((format!("/targets/vocabulary/{v}/example"), &item.example));
    }

    for (i, activity) in unit.activities.iter().enumerate() {
        collect_activity(activity, &format!("/activities/{i}"), everything, &mut out);
    }
    out
}

fn collect_activity<'a>(
    activity: &'a Activity,
    base: &str,
    everything: bool,
    out: &mut Vec<(String, &'a str)>,
) {
    let mut push = |field: String, text: &'a str| out.push((format!("{base}/{field}"), text));
    let localized = |push: &mut dyn FnMut(String, &'a str), field: &str, text: &'a Localized| {
        push(format!("{field}/en"), &text.en);
    };

    if everything {
        let common = activity.common();
        localized(&mut push, "instructions", common.instructions);
    }
    match activity {
        Activity::Mcq(a) => {
            if let Some(passage) = &a.passage {
                push("passage".into(), passage);
            }
            if let Some(audio) = &a.audio_text {
                push("audio_text".into(), audio);
            }
            push("stem".into(), &a.stem);
            if everything {
                for (o, option) in a.options.iter().enumerate() {
                    push(format!("options/{o}"), option);
                }
                localized(&mut push, "explanation", &a.explanation);
            }
        }
        Activity::GapFill(a) => {
            push("text".into(), &a.text);
            if everything {
                localized(&mut push, "explanation", &a.explanation);
            }
        }
        Activity::Reorder(a) => push("answer".into(), &a.answer),
        Activity::Match(a) => {
            if everything {
                for (p, pair) in a.pairs.iter().enumerate() {
                    push(format!("pairs/{p}/left"), &pair.left);
                }
            }
        }
        Activity::Dictation(a) => push("audio_text".into(), &a.audio_text),
        Activity::ReadAloud(a) => push("text".into(), &a.text),
        Activity::MinimalPairs(a) => {
            if everything {
                for (p, pair) in a.pairs.iter().enumerate() {
                    push(format!("pairs/{p}/a"), &pair.a);
                    push(format!("pairs/{p}/b"), &pair.b);
                }
            }
        }
        Activity::Shadowing(_) => {}
        Activity::GuidedSpeaking(a) | Activity::GuidedWriting(a) => {
            if everything {
                localized(&mut push, "prompt", &a.prompt);
            }
            for (m, answer) in a.model_answers.iter().enumerate() {
                push(format!("model_answers/{m}/text"), &answer.text);
            }
        }
        Activity::Roleplay(a) => {
            if everything {
                localized(&mut push, "scenario", &a.scenario);
                for (g, goal) in a.goals.iter().enumerate() {
                    push(format!("goals/{g}"), goal);
                }
            }
        }
        Activity::Mediation(a) => {
            push("source_text".into(), &a.source_text);
            if everything {
                localized(&mut push, "task", &a.task);
            }
            for (m, answer) in a.model_answers.iter().enumerate() {
                push(format!("model_answers/{m}/text"), &answer.text);
            }
        }
        Activity::ReadingSet(a) => {
            push("passage".into(), &a.passage);
            for (q, question) in a.questions.iter().enumerate() {
                push(format!("questions/{q}/stem"), &question.stem);
                if everything {
                    for (o, option) in question.options.iter().enumerate() {
                        push(format!("questions/{q}/options/{o}"), option);
                    }
                    localized(
                        &mut push,
                        &format!("questions/{q}/explanation"),
                        &question.explanation,
                    );
                }
            }
        }
        Activity::ListeningSet(a) => {
            if let Some(audio) = &a.audio_text {
                push("audio_text".into(), audio);
            }
            for (q, question) in a.questions.iter().enumerate() {
                push(format!("questions/{q}/stem"), &question.stem);
                if everything {
                    for (o, option) in question.options.iter().enumerate() {
                        push(format!("questions/{q}/options/{o}"), option);
                    }
                    localized(
                        &mut push,
                        &format!("questions/{q}/explanation"),
                        &question.explanation,
                    );
                }
            }
        }
        Activity::ErrorCorrection(a) => {
            push("sentence".into(), &a.sentence);
            if everything {
                localized(&mut push, "explanation", &a.explanation);
            }
        }
    }
}
