//! What the learner is shown for an activity, with every answer left out.
//!
//! The presentation is plain data. The command line prints it, the UI renders it,
//! and the speech side turns each [`AudioLine`] into sound. Spoken text is never
//! part of what is displayed: a listening item shows its questions, and the text
//! of the audio travels in a separate field for the synthesiser.

use curriculum::{
    Activity, ActivityType, Dialogue, GuidedProduction, Localized, MinimalPairsMode, Skill, Unit,
};
use serde::Serialize;

use super::error::ActivityError;

/// One line for the synthesiser. A dialogue has a speaker per line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioLine {
    pub speaker: Option<String>,
    pub text: String,
}

/// A question of a reading or listening set, without its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionView {
    pub stem: String,
    pub options: Vec<String>,
}

/// One item of a listening minimal-pairs activity: the two words to choose from
/// and the word that is played.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairItem {
    pub options: [String; 2],
    pub audio: AudioLine,
}

/// One pair of a say-mode minimal-pairs drill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairView {
    pub a: String,
    pub b: String,
    pub focus: String,
}

/// The content of an activity, by type.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Mcq {
        passage: Option<String>,
        /// Present for a listening item: played, not shown.
        audio: Option<Vec<AudioLine>>,
        stem: String,
        options: Vec<String>,
    },
    GapFill {
        text: String,
        gaps: usize,
    },
    Reorder {
        tokens: Vec<String>,
    },
    Match {
        left: Vec<String>,
        /// The right-hand column in the order it is shown. A response names the
        /// position in this list.
        right: Vec<String>,
    },
    Dictation {
        audio: Vec<AudioLine>,
    },
    MinimalPairsListen {
        items: Vec<PairItem>,
    },
    MinimalPairsSay {
        pairs: Vec<PairView>,
    },
    ReadingSet {
        passage: String,
        questions: Vec<QuestionView>,
    },
    ListeningSet {
        audio: Vec<AudioLine>,
        /// How many times the audio may be played again after the first time.
        replays_allowed: u8,
        questions: Vec<QuestionView>,
    },
    ErrorCorrection {
        sentence: String,
    },
    ReadAloud {
        text: String,
        focus_phonemes: Vec<String>,
    },
    Shadowing {
        title: String,
        lines: Vec<AudioLine>,
    },
    Production {
        prompt: Localized,
        content_points: Vec<String>,
        min_words: u32,
        max_words: u32,
    },
    Mediation {
        source_text: String,
        task: Localized,
    },
    Roleplay {
        scenario: Localized,
        tutor_role: String,
        learner_role: String,
        goals: Vec<String>,
        max_turns: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Presentation {
    pub id: String,
    pub activity_type: ActivityType,
    pub skill: Skill,
    pub instructions: Localized,
    pub body: Body,
}

/// FNV-1a over the id, the seed of the layouts below.
fn seed(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The next value of a splitmix64 stream.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// For a `match` of `n` pairs: `order[j]` is the pair whose right-hand text is
/// shown at position `j`. The order is a fixed function of the activity id, so a
/// presentation and the scoring of its response agree without any stored state,
/// and it is never the identity when there are two pairs or more, because a
/// column that is already in order is answered by lining the items up.
pub fn match_display_order(id: &str, n: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut state = seed(id);
    for i in (1..n).rev() {
        let j = usize::try_from(next(&mut state) % (i as u64 + 1)).unwrap_or(0);
        order.swap(i, j);
    }
    if n >= 2
        && order
            .iter()
            .enumerate()
            .all(|(position, pair)| position == *pair)
    {
        order.rotate_left(1);
    }
    order
}

/// For a listening minimal-pairs activity: which word of pair `index` is played,
/// 0 for the first and 1 for the second. A fixed function of the activity id and
/// the position, so the audio and the scoring agree.
pub fn minimal_pair_spoken(id: &str, index: usize) -> usize {
    let mut state = seed(id) ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    usize::try_from(next(&mut state) & 1).unwrap_or(0)
}

fn plain(text: &str) -> Vec<AudioLine> {
    vec![AudioLine {
        speaker: None,
        text: text.to_owned(),
    }]
}

fn dialogue_lines(dialogue: &Dialogue) -> Vec<AudioLine> {
    dialogue
        .turns
        .iter()
        .map(|turn| AudioLine {
            speaker: Some(turn.speaker.clone()),
            text: turn.text.clone(),
        })
        .collect()
}

fn question_views(questions: &[curriculum::Question]) -> Vec<QuestionView> {
    questions
        .iter()
        .map(|q| QuestionView {
            stem: q.stem.clone(),
            options: q.options.clone(),
        })
        .collect()
}

fn production(a: &GuidedProduction) -> Body {
    Body::Production {
        prompt: a.prompt.clone(),
        content_points: a.content_points.clone(),
        min_words: a.min_words,
        max_words: a.max_words,
    }
}

fn find_dialogue<'a>(unit: &'a Unit, id: &str) -> Result<&'a Dialogue, ActivityError> {
    unit.dialogues
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ActivityError::NoAudio(id.to_owned()))
}

/// The audio of a listening set or shadowing activity, by dialogue or by text.
pub fn audio_of(unit: &Unit, activity: &Activity) -> Result<Vec<AudioLine>, ActivityError> {
    match activity {
        Activity::Mcq(a) => a
            .audio_text
            .as_deref()
            .map(plain)
            .ok_or_else(|| ActivityError::NoAudio(a.id.clone())),
        Activity::Dictation(a) => Ok(plain(&a.audio_text)),
        Activity::ListeningSet(a) => match (&a.audio_text, &a.dialogue_id) {
            (Some(text), _) => Ok(plain(text)),
            (None, Some(dialogue)) => Ok(dialogue_lines(find_dialogue(unit, dialogue)?)),
            (None, None) => Err(ActivityError::NoAudio(a.id.clone())),
        },
        Activity::Shadowing(a) => Ok(dialogue_lines(find_dialogue(unit, &a.dialogue_id)?)),
        Activity::MinimalPairs(a) if a.mode == MinimalPairsMode::ListenChoose => Ok(a
            .pairs
            .iter()
            .enumerate()
            .map(|(index, pair)| AudioLine {
                speaker: None,
                text: if minimal_pair_spoken(&a.id, index) == 0 {
                    pair.a.clone()
                } else {
                    pair.b.clone()
                },
            })
            .collect()),
        other => Err(ActivityError::NoAudio(other.id().to_owned())),
    }
}

/// The presentation of `activity`, from the unit it belongs to.
pub fn present(unit: &Unit, activity: &Activity) -> Result<Presentation, ActivityError> {
    let common = activity.common();
    let body = match activity {
        Activity::Mcq(a) => Body::Mcq {
            passage: a.passage.clone(),
            audio: a.audio_text.as_deref().map(plain),
            stem: a.stem.clone(),
            options: a.options.clone(),
        },
        Activity::GapFill(a) => Body::GapFill {
            text: a.text.clone(),
            gaps: a.answers.len(),
        },
        Activity::Reorder(a) => Body::Reorder {
            tokens: a.tokens.clone(),
        },
        Activity::Match(a) => {
            let order = match_display_order(&a.id, a.pairs.len());
            Body::Match {
                left: a.pairs.iter().map(|p| p.left.clone()).collect(),
                right: order.iter().map(|i| a.pairs[*i].right.clone()).collect(),
            }
        }
        Activity::Dictation(a) => Body::Dictation {
            audio: plain(&a.audio_text),
        },
        Activity::MinimalPairs(a) => match a.mode {
            MinimalPairsMode::ListenChoose => {
                let lines = audio_of(unit, activity)?;
                Body::MinimalPairsListen {
                    items: a
                        .pairs
                        .iter()
                        .zip(lines)
                        .map(|(pair, audio)| PairItem {
                            options: [pair.a.clone(), pair.b.clone()],
                            audio,
                        })
                        .collect(),
                }
            }
            MinimalPairsMode::SayBoth => Body::MinimalPairsSay {
                pairs: a
                    .pairs
                    .iter()
                    .map(|p| PairView {
                        a: p.a.clone(),
                        b: p.b.clone(),
                        focus: p.focus.clone(),
                    })
                    .collect(),
            },
        },
        Activity::ReadingSet(a) => Body::ReadingSet {
            passage: a.passage.clone(),
            questions: question_views(&a.questions),
        },
        Activity::ListeningSet(a) => Body::ListeningSet {
            audio: audio_of(unit, activity)?,
            replays_allowed: a.replays_allowed,
            questions: question_views(&a.questions),
        },
        Activity::ErrorCorrection(a) => Body::ErrorCorrection {
            sentence: a.sentence.clone(),
        },
        Activity::ReadAloud(a) => Body::ReadAloud {
            text: a.text.clone(),
            focus_phonemes: a.focus_phonemes.clone(),
        },
        Activity::Shadowing(a) => Body::Shadowing {
            title: find_dialogue(unit, &a.dialogue_id)?.title.clone(),
            lines: audio_of(unit, activity)?,
        },
        Activity::GuidedSpeaking(a) | Activity::GuidedWriting(a) => production(a),
        Activity::Mediation(a) => Body::Mediation {
            source_text: a.source_text.clone(),
            task: a.task.clone(),
        },
        Activity::Roleplay(a) => Body::Roleplay {
            scenario: a.scenario.clone(),
            tutor_role: a.tutor_role.clone(),
            learner_role: a.learner_role.clone(),
            goals: a.goals.clone(),
            max_turns: a.max_turns,
        },
    };
    Ok(Presentation {
        id: common.id.to_owned(),
        activity_type: common.activity_type,
        skill: common.skill,
        instructions: common.instructions.clone(),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_match_order_is_a_fixed_permutation_that_is_never_the_identity() {
        for n in 2..9 {
            for id in ["a05-match-phrases", "other", "x", "a06"] {
                let order = match_display_order(id, n);
                assert_eq!(order, match_display_order(id, n), "stable");
                let mut sorted = order.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "a permutation");
                assert_ne!(order, (0..n).collect::<Vec<_>>(), "{id} {n}");
            }
        }
        assert!(match_display_order("x", 0).is_empty());
        assert_eq!(match_display_order("x", 1), [0]);
    }

    #[test]
    fn different_activities_get_different_match_orders() {
        let orders: std::collections::HashSet<Vec<usize>> = (0..20)
            .map(|i| match_display_order(&format!("act-{i}"), 5))
            .collect();
        assert!(orders.len() > 5, "{} distinct orders", orders.len());
    }

    #[test]
    fn the_spoken_word_of_a_pair_is_fixed_and_both_words_occur() {
        let spoken: Vec<usize> = (0..40).map(|i| minimal_pair_spoken("a08", i)).collect();
        assert_eq!(
            spoken,
            (0..40)
                .map(|i| minimal_pair_spoken("a08", i))
                .collect::<Vec<_>>()
        );
        assert!(spoken.contains(&0) && spoken.contains(&1));
        assert!(spoken.iter().all(|s| *s < 2));
    }
}
