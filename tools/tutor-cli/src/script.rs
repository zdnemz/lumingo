//! A script of turns for a run without a person.
//!
//! One turn per line. A line is either the text the learner types, or
//! `wav: <path>` for a recorded utterance that is fed through the VAD and the
//! recogniser as if it came from the microphone. A relative path is relative to
//! the script file. Blank lines and lines starting with `#` are ignored.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptTurn {
    Text(String),
    Audio(PathBuf),
}

impl ScriptTurn {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Audio(_) => "audio",
        }
    }
}

/// Parses the text of a script. `base` is the folder `wav:` paths are relative to.
pub fn parse(text: &str, base: &Path) -> Result<Vec<ScriptTurn>> {
    let mut turns = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.strip_prefix("wav:") {
            Some(path) => {
                let path = path.trim();
                if path.is_empty() {
                    bail!("line {}: `wav:` needs a path", number + 1);
                }
                let path = Path::new(path);
                turns.push(ScriptTurn::Audio(if path.is_absolute() {
                    path.to_owned()
                } else {
                    base.join(path)
                }));
            }
            None => turns.push(ScriptTurn::Text(line.to_owned())),
        }
    }
    if turns.is_empty() {
        bail!("the script has no turns");
    }
    Ok(turns)
}

pub fn load(path: &Path) -> Result<Vec<ScriptTurn>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the script {}", path.display()))?;
    parse(&text, path.parent().unwrap_or_else(|| Path::new(".")))
}

/// The turns of a run: the script in order, repeated when it is shorter than
/// `wanted`, cut when it is longer. `None` runs the script once.
pub fn plan(script: &[ScriptTurn], wanted: Option<u32>) -> Vec<ScriptTurn> {
    let count = wanted.map_or(script.len(), |n| n as usize);
    script.iter().cycle().take(count).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_become_text_or_audio_and_comments_are_skipped() {
        let turns = parse(
            "# a comment\n\nHello, my name is Dewi.\nwav: clips/one.wav\n  I am from Bandung.  \n",
            Path::new("/scripts"),
        )
        .unwrap();
        assert_eq!(
            turns,
            [
                ScriptTurn::Text("Hello, my name is Dewi.".to_owned()),
                ScriptTurn::Audio(PathBuf::from("/scripts/clips/one.wav")),
                ScriptTurn::Text("I am from Bandung.".to_owned()),
            ]
        );
    }

    #[test]
    fn an_absolute_wav_path_is_kept_and_a_bare_prefix_is_refused() {
        let turns = parse("wav: /data/a.wav", Path::new("/scripts")).unwrap();
        assert_eq!(turns, [ScriptTurn::Audio(PathBuf::from("/data/a.wav"))]);
        assert!(parse("wav:", Path::new(".")).is_err());
        assert!(parse("# nothing but a comment\n\n", Path::new(".")).is_err());
    }

    #[test]
    fn a_short_script_repeats_to_the_wanted_turns_and_a_long_one_is_cut() {
        let script = parse("one\ntwo\nthree", Path::new(".")).unwrap();
        let names = |turns: Vec<ScriptTurn>| -> Vec<String> {
            turns
                .into_iter()
                .map(|t| match t {
                    ScriptTurn::Text(t) => t,
                    ScriptTurn::Audio(p) => p.display().to_string(),
                })
                .collect()
        };
        assert_eq!(names(plan(&script, None)), ["one", "two", "three"]);
        assert_eq!(names(plan(&script, Some(2))), ["one", "two"]);
        assert_eq!(
            names(plan(&script, Some(7))),
            ["one", "two", "three", "one", "two", "three", "one"]
        );
        assert!(plan(&script, Some(0)).is_empty());
    }

    #[test]
    fn the_shipped_script_has_fifty_typed_turns() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/a1-u01-50-turns.txt");
        let turns = load(&path).unwrap();
        assert_eq!(turns.len(), 50);
        assert!(turns.iter().all(|t| matches!(t, ScriptTurn::Text(_))));
        // No line repeats, so a 50-turn run does not feed the model the same sentence twice.
        let mut unique: Vec<&ScriptTurn> = turns.iter().collect();
        unique.sort_by_key(|t| format!("{t:?}"));
        unique.dedup();
        assert_eq!(unique.len(), 50);
    }
}
