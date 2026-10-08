//! The file of scripted responses for `tutor-cli unit run --script`.
//!
//! One JSON object, `{ "responses": { "<activity id>": { ... } } }`. Each entry
//! uses the fields that fit its activity, and the words are the learner's, not
//! positions on a screen: a match is given by the phrase and its meaning, a
//! listening minimal pair by the word that was heard. Option indexes count from 0
//! like `answer_index` in the unit file.
//!
//! | Activity | Fields |
//! |---|---|
//! | `mcq` | `choice` |
//! | `gap_fill` | `gaps` |
//! | `reorder` | `order` |
//! | `match` | `pairs`: left text to right text |
//! | `dictation`, `error_correction`, `guided_writing`, written `mediation` | `text` |
//! | `minimal_pairs` (listen) | `heard`: the word heard per pair, `null` to leave blank |
//! | `reading_set`, `listening_set` | `answers`: option index per question, `null` to leave blank |
//! | `guided_speaking`, spoken `mediation` | `spoken`: the transcript, and `spans`: `[start_ms, end_ms]` pairs |
//! | `roleplay` | `turns`: the learner's lines |
//! | `read_aloud`, `minimal_pairs` (say), `shadowing` | `wav`: one file, `clips`: a file per clip, or `done` |
//!
//! `plays` is how many times an audio item is played before it is answered
//! (default 1). A relative `wav` path is relative to the script file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use assessment_engine::VoicedSpan;
use serde::Deserialize;
use tutor_engine::{Body, Clip, Presentation, Response, SpokenResponse};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptFile {
    pub responses: BTreeMap<String, ScriptedResponse>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScriptedResponse {
    pub choice: Option<usize>,
    pub gaps: Option<Vec<String>>,
    pub order: Option<Vec<String>>,
    pub pairs: Option<BTreeMap<String, String>>,
    pub heard: Option<Vec<Option<String>>>,
    pub answers: Option<Vec<Option<usize>>>,
    pub text: Option<String>,
    pub spoken: Option<String>,
    pub spans: Option<Vec<[u32; 2]>>,
    pub turns: Option<Vec<String>>,
    pub wav: Option<PathBuf>,
    pub clips: Option<Vec<PathBuf>>,
    pub done: Option<bool>,
    pub plays: Option<u8>,
}

/// What a responder hands the driver for one activity.
#[derive(Debug)]
pub struct Answer {
    pub response: Response,
    /// Plays of the audio before answering.
    pub plays: u8,
}

pub fn parse(text: &str) -> Result<ScriptFile> {
    serde_json::from_str(text).context("the script is not a valid responses file")
}

pub fn load(path: &Path) -> Result<ScriptFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the script {}", path.display()))?;
    parse(&text)
}

fn need<T>(value: Option<T>, id: &str, field: &str) -> Result<T> {
    value.with_context(|| format!("the script's entry for {id} needs `{field}`"))
}

fn clip_from(path: &Path, base: &Path) -> Result<Clip> {
    let full = if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    };
    let bytes = std::fs::read(&full)
        .with_context(|| format!("cannot read the recording {}", full.display()))?;
    let samples = crate::wav::parse(&bytes)
        .with_context(|| format!("{} is not a usable recording", full.display()))?;
    Ok(Clip {
        samples,
        says: None,
    })
}

impl ScriptedResponse {
    /// The learner's lines of a roleplay.
    pub fn roleplay_turns(&self) -> Vec<String> {
        self.turns.clone().unwrap_or_default()
    }

    /// Turns the entry into the response the activity takes. `base` is the folder
    /// `wav` paths are relative to.
    pub fn into_answer(&self, shown: &Presentation, base: &Path) -> Result<Answer> {
        let id = shown.id.as_str();
        let response = match &shown.body {
            Body::Mcq { .. } => Response::Choice(need(self.choice, id, "choice")?),
            Body::GapFill { .. } => Response::Gaps(need(self.gaps.clone(), id, "gaps")?),
            Body::Reorder { .. } => Response::Order(need(self.order.clone(), id, "order")?),
            Body::Match { left, right } => {
                let pairs = need(self.pairs.as_ref(), id, "pairs")?;
                let mut picks = Vec::with_capacity(left.len());
                for phrase in left {
                    picks.push(match pairs.get(phrase) {
                        None => None,
                        Some(meaning) => {
                            Some(right.iter().position(|r| r == meaning).ok_or_else(|| {
                                anyhow!("{id}: \"{meaning}\" is not in the right-hand column")
                            })?)
                        }
                    });
                }
                Response::Picks(picks)
            }
            Body::Dictation { .. } | Body::ErrorCorrection { .. } => {
                Response::Text(need(self.text.clone(), id, "text")?)
            }
            Body::MinimalPairsListen { items } => {
                let heard = need(self.heard.as_ref(), id, "heard")?;
                if heard.len() != items.len() {
                    bail!(
                        "{id}: `heard` has {} entries and there are {} pairs",
                        heard.len(),
                        items.len()
                    );
                }
                let mut picks = Vec::with_capacity(items.len());
                for (item, word) in items.iter().zip(heard) {
                    picks.push(match word {
                        None => None,
                        Some(word) => {
                            Some(item.options.iter().position(|o| o == word).ok_or_else(|| {
                                anyhow!("{id}: \"{word}\" is not one of {:?}", item.options)
                            })?)
                        }
                    });
                }
                Response::Picks(picks)
            }
            Body::ReadingSet { .. } | Body::ListeningSet { .. } => {
                Response::Picks(need(self.answers.clone(), id, "answers")?)
            }
            Body::Production { .. } | Body::Mediation { .. } => match (&self.spoken, &self.text) {
                (Some(transcript), _) => Response::Spoken(SpokenResponse {
                    transcript: transcript.clone(),
                    spans: self
                        .spans
                        .iter()
                        .flatten()
                        .map(|[start_ms, end_ms]| VoicedSpan {
                            start_ms: *start_ms,
                            end_ms: *end_ms,
                        })
                        .collect(),
                    stt: None,
                }),
                (None, Some(text)) => Response::Text(text.clone()),
                (None, None) => bail!("the script's entry for {id} needs `spoken` or `text`"),
            },
            Body::ReadAloud { .. } | Body::MinimalPairsSay { .. } | Body::Shadowing { .. } => {
                if let Some(clips) = &self.clips {
                    Response::Clips(
                        clips
                            .iter()
                            .map(|p| clip_from(p, base))
                            .collect::<Result<_>>()?,
                    )
                } else if let Some(path) = &self.wav {
                    Response::Clips(vec![clip_from(path, base)?])
                } else if self.done == Some(true) {
                    Response::Done
                } else {
                    bail!("the script's entry for {id} needs `wav`, `clips` or `done`");
                }
            }
            Body::Roleplay { .. } => bail!("{id} is a roleplay: it takes `turns`, not a response"),
        };
        Ok(Answer {
            response,
            plays: self.plays.unwrap_or(1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tutor_engine::{AudioLine, PairItem};

    fn shown(id: &str, body: Body) -> Presentation {
        Presentation {
            id: id.to_owned(),
            activity_type: curriculum::ActivityType::Mcq,
            skill: curriculum::Skill::Listening,
            instructions: curriculum::Localized {
                en: "x".into(),
                id: None,
            },
            body,
        }
    }

    fn entry(json: &str) -> ScriptedResponse {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_match_is_given_by_phrase_and_meaning_and_mapped_to_the_displayed_column() {
        let body = Body::Match {
            left: vec!["Good morning".into(), "Thank you".into()],
            right: vec!["Terima kasih".into(), "Selamat pagi".into()],
        };
        let answer =
            entry(r#"{"pairs": {"Good morning": "Selamat pagi", "Thank you": "Terima kasih"}}"#)
                .into_answer(&shown("m", body), Path::new("."))
                .unwrap();
        assert_eq!(answer.response, Response::Picks(vec![Some(1), Some(0)]));
        assert_eq!(answer.plays, 1);
    }

    #[test]
    fn a_missing_pair_is_left_blank_and_an_unknown_meaning_is_an_error() {
        let body = || Body::Match {
            left: vec!["a".into(), "b".into()],
            right: vec!["x".into(), "y".into()],
        };
        let answer = entry(r#"{"pairs": {"a": "y"}}"#)
            .into_answer(&shown("m", body()), Path::new("."))
            .unwrap();
        assert_eq!(answer.response, Response::Picks(vec![Some(1), None]));
        let error = entry(r#"{"pairs": {"a": "zzz"}}"#)
            .into_answer(&shown("m", body()), Path::new("."))
            .unwrap_err();
        assert!(error.to_string().contains("zzz"));
    }

    #[test]
    fn minimal_pairs_are_answered_with_the_word_heard() {
        let item = |a: &str, b: &str| PairItem {
            options: [a.to_owned(), b.to_owned()],
            audio: AudioLine {
                speaker: None,
                text: a.to_owned(),
            },
        };
        let body = Body::MinimalPairsListen {
            items: vec![item("thank", "tank"), item("three", "tree")],
        };
        let answer = entry(r#"{"heard": ["tank", null]}"#)
            .into_answer(&shown("p", body), Path::new("."))
            .unwrap();
        assert_eq!(answer.response, Response::Picks(vec![Some(1), None]));
    }

    #[test]
    fn an_entry_without_the_field_its_activity_needs_says_which() {
        let body = Body::Mcq {
            passage: None,
            audio: None,
            stem: "s".into(),
            options: vec!["a".into(), "b".into()],
        };
        let error = entry("{}")
            .into_answer(&shown("a01", body), Path::new("."))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "the script's entry for a01 needs `choice`"
        );
    }

    #[test]
    fn production_takes_a_transcript_with_spans_or_text() {
        let body = || Body::Production {
            prompt: curriculum::Localized {
                en: "p".into(),
                id: None,
            },
            content_points: Vec::new(),
            min_words: 1,
            max_words: 9,
        };
        let spoken =
            entry(r#"{"spoken": "hello there", "spans": [[0, 500], [900, 1400]], "plays": 2}"#)
                .into_answer(&shown("s", body()), Path::new("."))
                .unwrap();
        let Response::Spoken(s) = spoken.response else {
            panic!("spoken");
        };
        assert_eq!(s.transcript, "hello there");
        assert_eq!(s.spans.len(), 2);
        assert_eq!(s.spans[1].start_ms, 900);
        assert_eq!(spoken.plays, 2);
        let written = entry(r#"{"text": "hello"}"#)
            .into_answer(&shown("w", body()), Path::new("."))
            .unwrap();
        assert_eq!(written.response, Response::Text("hello".into()));
        assert!(
            entry("{}")
                .into_answer(&shown("w", body()), Path::new("."))
                .is_err()
        );
    }

    #[test]
    fn a_drill_takes_recordings_or_a_completion_and_a_roleplay_takes_turns() {
        let body = || Body::ReadAloud {
            text: "t".into(),
            focus_phonemes: Vec::new(),
        };
        let done = entry(r#"{"done": true}"#)
            .into_answer(&shown("d", body()), Path::new("."))
            .unwrap();
        assert_eq!(done.response, Response::Done);
        assert!(
            entry("{}")
                .into_answer(&shown("d", body()), Path::new("."))
                .is_err()
        );
        let missing = entry(r#"{"wav": "nowhere.wav"}"#)
            .into_answer(&shown("d", body()), Path::new("/no/such/folder"))
            .unwrap_err();
        assert!(missing.to_string().contains("nowhere.wav"));

        let roleplay = Body::Roleplay {
            scenario: curriculum::Localized {
                en: "s".into(),
                id: None,
            },
            tutor_role: "t".into(),
            learner_role: "l".into(),
            goals: Vec::new(),
            max_turns: 3,
        };
        assert!(
            entry(r#"{"turns": ["hi"]}"#)
                .into_answer(&shown("r", roleplay), Path::new("."))
                .is_err()
        );
        assert_eq!(
            entry(r#"{"turns": ["hi", "bye"]}"#).roleplay_turns(),
            ["hi", "bye"]
        );
    }

    #[test]
    fn unknown_fields_and_bad_json_are_refused() {
        assert!(parse(r#"{"responses": {"a": {"chioce": 1}}}"#).is_err());
        assert!(parse("[]").is_err());
        let ok = parse(r#"{"responses": {"a": {"choice": 1}, "b": {"answers": [0, null, 2]}}}"#)
            .unwrap();
        assert_eq!(ok.responses.len(), 2);
        assert_eq!(
            ok.responses["b"].answers,
            Some(vec![Some(0), None, Some(2)])
        );
    }
}
