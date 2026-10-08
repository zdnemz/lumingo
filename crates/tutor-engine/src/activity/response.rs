//! What a learner hands in for an activity.
//!
//! The shapes are plain data, so the command line, the server and the tests all
//! build them the same way. Sizes are bounded: text is cut off at
//! [`MAX_TEXT_CHARS`] and audio at [`MAX_CLIP_SAMPLES`] per clip and
//! [`MAX_TOTAL_SAMPLES`] per response. Longer input is refused, not truncated,
//! so a learner is never scored on a part of what they said or wrote.

use assessment_engine::VoicedSpan;
use serde::{Deserialize, Serialize};
use speech::EngineInfo;

use super::error::ActivityError;

/// Longest text response, in characters. A 400-word C2 mediation answer is about
/// 2,500 characters; the cap leaves room above that and keeps a pasted wall of
/// text out of a prompt.
pub const MAX_TEXT_CHARS: usize = 6_000;

/// Longest audio clip, in samples at 16 kHz mono: one minute.
pub const MAX_CLIP_SAMPLES: usize = 16_000 * 60;

/// Most audio samples in one response: three minutes at 16 kHz. A drill has a
/// handful of short clips; the cap keeps memory bounded.
pub const MAX_TOTAL_SAMPLES: usize = 16_000 * 180;

/// Most clips in one response.
pub const MAX_CLIPS: usize = 64;

/// A spoken answer: the transcript of what the learner said, with the voiced
/// spans from the voice activity detector and the recogniser that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpokenResponse {
    /// The transcript after any correction by the learner.
    pub transcript: String,
    /// Voiced stretches of the recording, for the timing figures. May be empty.
    pub spans: Vec<VoicedSpan>,
    /// The recogniser, recorded as evidence that a speech engine took part.
    pub stt: Option<EngineInfo>,
}

/// One recorded clip, 16 kHz mono, for the pronunciation engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub samples: Vec<f32>,
    /// The words the clip is supposed to hold, when the response names them
    /// (a minimal pair says which word). `None` uses the activity's text.
    pub says: Option<String>,
}

/// What a learner hands in.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// The index of the chosen option of an `mcq`.
    Choice(usize),
    /// One answer per gap of a `gap_fill`.
    Gaps(Vec<String>),
    /// The tokens of a `reorder` in the order the learner put them.
    Order(Vec<String>),
    /// A choice per item, `None` for an item left blank: the questions of a
    /// reading or listening set, the pairs of a `match` (the index in the
    /// displayed right-hand column) and the words of a listening minimal pair
    /// (0 for the first word, 1 for the second).
    Picks(Vec<Option<usize>>),
    /// Typed text: a dictation, a corrected sentence, a written production.
    Text(String),
    /// A spoken production, as a transcript.
    Spoken(SpokenResponse),
    /// Recorded clips for a pronunciation drill.
    Clips(Vec<Clip>),
    /// The learner went through the activity and there is nothing to score
    /// (a shadowing run whose scoring is `none`).
    Done,
}

impl Response {
    /// A short name of the shape, for error messages and logs. It never holds
    /// what the learner wrote.
    pub fn shape(&self) -> &'static str {
        match self {
            Self::Choice(_) => "a choice",
            Self::Gaps(_) => "gap answers",
            Self::Order(_) => "an order of tokens",
            Self::Picks(_) => "a list of picks",
            Self::Text(_) => "text",
            Self::Spoken(_) => "a spoken transcript",
            Self::Clips(_) => "recorded clips",
            Self::Done => "a completion",
        }
    }
}

/// Refuses text that is too long. Empty text is the caller's business: a blank
/// dictation is a wrong answer, a blank essay is nothing to score.
pub fn check_text(text: &str) -> Result<(), ActivityError> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(ActivityError::TooLong {
            max: MAX_TEXT_CHARS,
            unit: "characters",
        });
    }
    Ok(())
}

/// Refuses clips beyond the bounds above.
pub fn check_clips(clips: &[Clip]) -> Result<(), ActivityError> {
    if clips.len() > MAX_CLIPS {
        return Err(ActivityError::TooLong {
            max: MAX_CLIPS,
            unit: "clips",
        });
    }
    let mut total = 0_usize;
    for clip in clips {
        if clip.samples.len() > MAX_CLIP_SAMPLES {
            return Err(ActivityError::TooLong {
                max: MAX_CLIP_SAMPLES,
                unit: "samples in one clip",
            });
        }
        total = total.saturating_add(clip.samples.len());
    }
    if total > MAX_TOTAL_SAMPLES {
        return Err(ActivityError::TooLong {
            max: MAX_TOTAL_SAMPLES,
            unit: "samples in one response",
        });
    }
    Ok(())
}
