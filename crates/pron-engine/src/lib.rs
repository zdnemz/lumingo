//! Pronunciation engine: reference text to phonemes, forced alignment of a
//! phoneme recogniser's posteriors, goodness-of-pronunciation scores, honest
//! "not scored" reporting, and the blocking-or-deferred timing policy.
//!
//! Everything except the ONNX adapter is a pure function over plain data, so it
//! is tested without a model. See `docs/ASSESSMENT_SPEC.md` section 7 and
//! ADR-008 and ADR-009.
#![forbid(unsafe_code)]

pub mod align;
pub mod arpabet;
pub mod calibration;
pub mod gop;
pub mod lexicon;
pub mod perf;
pub mod phone_map;
pub mod posteriors;
pub mod reference;
pub mod score;
pub mod vocab;

#[cfg(test)]
mod testing;

pub use align::{AlignError, AlignedPhone, AlignedWord, Alignment, WordSpec, align};
pub use arpabet::{Arpabet, Phone, PhoneClass, UnknownArpabet};
pub use calibration::{Calibration, CalibrationError, LogisticCurve, Thresholds};
pub use gop::{GopError, HeardColumn, PhoneGop, gop};
pub use lexicon::{Lexicon, LexiconError};
pub use perf::{
    DecisionSource, Measurement, ModeDecision, ModeSetting, PerfError, PerformanceTier,
    PolicyConfig, Prediction, PronMode, SpeedProfile, Workload, Workloads, decide, headroom_ms,
    measure,
};
pub use phone_map::{BoundPhoneMap, PhoneMap, PhoneMapError};
pub use posteriors::{LogPosteriors, PosteriorsError};
pub use reference::{NotChecked, Reference, ReferenceWord, WordStatus};
pub use score::{
    FocusResult, Heard, MAX_HIGHLIGHTED_WORDS, Mode, NotScoredReason, Outcome, PhonemeResult,
    ScoreError, ScoreRequest, UtteranceReport, WordResult, score_utterance,
};
pub use vocab::{ModelVocab, VocabError};
