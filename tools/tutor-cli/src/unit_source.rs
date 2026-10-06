//! Where the learner's responses come from in `tutor-cli unit run`: a script
//! file, or lines typed at the terminal.
//!
//! Typed input is read asynchronously, so a run that waits for a person never
//! blocks the runtime. At the end of the input every remaining activity is
//! skipped. A line that does not parse is refused with a reason and asked again.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use curriculum::ActivityType;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tutor_engine::{Body, Clip, Presentation, Response, SpokenResponse};

use crate::unit_script::{Answer, ScriptFile};

pub enum Source<R> {
    /// Responses from a script. `base` is the folder `wav` paths are relative to.
    Script {
        file: ScriptFile,
        base: PathBuf,
        /// How many of each roleplay's scripted lines have been used.
        used: HashMap<String, usize>,
    },
    /// Lines typed by a person.
    Typed { reader: R },
}

impl<R> Source<R> {
    pub fn script(file: ScriptFile, base: &Path) -> Self {
        Self::Script {
            file,
            base: base.to_owned(),
            used: HashMap::new(),
        }
    }

    pub fn typed(reader: R) -> Self {
        Self::Typed { reader }
    }
}

/// `1`-based numbers, as a person counts, into 0-based indexes. `-` is blank.
fn parse_picks(line: &str, count: usize, options: &[usize]) -> Result<Vec<Option<usize>>> {
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() != count {
        bail!("give {count} answer(s), one per item, separated by spaces");
    }
    tokens
        .iter()
        .zip(options.iter().cycle())
        .map(|(token, max)| {
            if *token == "-" {
                return Ok(None);
            }
            let n: usize = token
                .parse()
                .map_err(|_| anyhow!("\"{token}\" is not a number"))?;
            if n == 0 || n > *max {
                bail!("{n} is not between 1 and {max}");
            }
            Ok(Some(n - 1))
        })
        .collect()
}

fn parse_gaps(line: &str, gaps: usize) -> Result<Vec<String>> {
    let answers: Vec<String> = line.split('|').map(|a| a.trim().to_owned()).collect();
    if answers.len() != gaps {
        bail!("give {gaps} answer(s) separated by `|`");
    }
    Ok(answers)
}

fn parse_wav(line: &str) -> Result<PathBuf> {
    let path = line
        .trim()
        .strip_prefix("wav:")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| anyhow!("type `wav: <file>`"))?;
    Ok(PathBuf::from(path))
}

fn load_clip(path: &Path) -> Result<Clip> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read the recording {}", path.display()))?;
    let samples = crate::wav::parse(&bytes)
        .with_context(|| format!("{} is not a usable recording", path.display()))?;
    Ok(Clip {
        samples,
        says: None,
    })
}

fn option_counts(shown: &Presentation) -> Vec<usize> {
    match &shown.body {
        Body::ReadingSet { questions, .. } | Body::ListeningSet { questions, .. } => {
            questions.iter().map(|q| q.options.len()).collect()
        }
        Body::Match { right, .. } => vec![right.len()],
        Body::MinimalPairsListen { .. } => vec![2],
        _ => Vec::new(),
    }
}

impl<R: AsyncBufRead + Unpin> Source<R> {
    async fn read_line(reader: &mut R) -> Result<Option<String>> {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        Ok((read > 0).then(|| line.trim_end_matches(['\r', '\n']).to_owned()))
    }

    /// The answer to one activity, or `None` to skip it. A roleplay is not
    /// answered here: see [`Source::roleplay_turn`].
    pub async fn answer(
        &mut self,
        shown: &Presentation,
        out: &mut impl Write,
    ) -> Result<Option<Answer>> {
        match self {
            Self::Script { file, base, .. } => match file.responses.get(&shown.id) {
                None => Ok(None),
                Some(entry) => entry.into_answer(shown, base).map(Some),
            },
            Self::Typed { reader } => loop {
                write!(out, "> ")?;
                out.flush()?;
                let Some(line) = Self::read_line(reader).await? else {
                    return Ok(None);
                };
                if line.trim() == "/skip" {
                    return Ok(None);
                }
                match Self::parse_typed(shown, &line, reader).await {
                    Ok(response) => {
                        return Ok(Some(Answer { response, plays: 1 }));
                    }
                    Err(error) => writeln!(out, "  {error:#}")?,
                }
            },
        }
    }

    async fn parse_typed(shown: &Presentation, line: &str, reader: &mut R) -> Result<Response> {
        let counts = option_counts(shown);
        Ok(match &shown.body {
            Body::Mcq { options, .. } => {
                let picks = parse_picks(line, 1, &[options.len()])?;
                Response::Choice(picks[0].ok_or_else(|| anyhow!("choose an option"))?)
            }
            Body::GapFill { gaps, .. } => Response::Gaps(parse_gaps(line, *gaps)?),
            Body::Reorder { tokens } => {
                let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
                if words.len() != tokens.len() {
                    bail!("type all {} words", tokens.len());
                }
                Response::Order(words)
            }
            Body::Match { left, right } => {
                Response::Picks(parse_picks(line, left.len(), &[right.len()])?)
            }
            Body::MinimalPairsListen { items } => {
                Response::Picks(parse_picks(line, items.len(), &[2])?)
            }
            Body::ReadingSet { questions, .. } | Body::ListeningSet { questions, .. } => {
                Response::Picks(parse_picks(line, questions.len(), &counts)?)
            }
            Body::Dictation { .. } | Body::ErrorCorrection { .. } => {
                Response::Text(line.to_owned())
            }
            Body::Production { .. } | Body::Mediation { .. } => {
                if line.trim().is_empty() {
                    bail!("type your answer, or `/skip`");
                }
                if shown.activity_type == ActivityType::GuidedSpeaking {
                    // A typed answer to a speaking task stands for its transcript.
                    Response::Spoken(SpokenResponse {
                        transcript: line.to_owned(),
                        spans: Vec::new(),
                        stt: None,
                    })
                } else {
                    Response::Text(line.to_owned())
                }
            }
            Body::ReadAloud { .. } => Self::typed_clips(line, 1, reader).await?,
            Body::MinimalPairsSay { pairs } => Self::typed_clips(line, pairs.len(), reader).await?,
            Body::Shadowing { lines, .. } => Self::typed_clips(line, lines.len(), reader).await?,
            Body::Roleplay { .. } => bail!("a roleplay is played turn by turn"),
        })
    }

    /// An empty first line is a completion; otherwise `count` lines of
    /// `wav: <file>`, the first of which is already read.
    async fn typed_clips(first: &str, count: usize, reader: &mut R) -> Result<Response> {
        if first.trim().is_empty() {
            return Ok(Response::Done);
        }
        let mut clips = vec![load_clip(&parse_wav(first)?)?];
        while clips.len() < count {
            let Some(next) = Self::read_line(reader).await? else {
                bail!("the input ended before {count} recording(s)");
            };
            clips.push(load_clip(&parse_wav(&next)?)?);
        }
        Ok(Response::Clips(clips))
    }

    /// The learner's next line of a roleplay, or `None` when they have no more.
    pub async fn roleplay_turn(
        &mut self,
        id: &str,
        out: &mut impl Write,
    ) -> Result<Option<String>> {
        match self {
            Self::Script { file, used, .. } => {
                let Some(entry) = file.responses.get(id) else {
                    return Ok(None);
                };
                let turns = entry.roleplay_turns();
                let next = used.entry(id.to_owned()).or_insert(0);
                let line = turns.get(*next).cloned();
                if line.is_some() {
                    *next += 1;
                }
                Ok(line)
            }
            Self::Typed { reader } => {
                write!(out, "you> ")?;
                out.flush()?;
                match Self::read_line(reader).await? {
                    None => Ok(None),
                    Some(line) if line.trim() == "/end" || line.trim() == "/skip" => Ok(None),
                    Some(line) if line.trim().is_empty() => Ok(None),
                    Some(line) => Ok(Some(line)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_count_from_one_and_a_dash_is_blank() {
        assert_eq!(
            parse_picks("2 - 1", 3, &[3]).unwrap(),
            [Some(1), None, Some(0)]
        );
        assert_eq!(parse_picks("1,2", 2, &[2]).unwrap(), [Some(0), Some(1)]);
        assert!(parse_picks("1 2", 3, &[3]).is_err(), "too few");
        assert!(parse_picks("4", 1, &[3]).is_err(), "out of range");
        assert!(parse_picks("0", 1, &[3]).is_err(), "counting starts at 1");
        assert!(parse_picks("x", 1, &[3]).is_err());
        // Each question has its own number of options.
        assert!(parse_picks("3 3", 2, &[3, 2]).is_err());
        assert!(parse_picks("3 2", 2, &[3, 2]).is_ok());
    }

    #[test]
    fn gaps_are_split_on_a_bar_and_must_match_the_count() {
        assert_eq!(parse_gaps(" am | from ", 2).unwrap(), ["am", "from"]);
        assert!(parse_gaps("am", 2).is_err());
        assert_eq!(parse_gaps("", 1).unwrap(), [""]);
    }

    #[test]
    fn a_recording_is_named_with_wav_and_a_path() {
        assert_eq!(parse_wav("wav: a/b.wav").unwrap(), PathBuf::from("a/b.wav"));
        assert!(parse_wav("a/b.wav").is_err());
        assert!(parse_wav("wav:").is_err());
    }
}
