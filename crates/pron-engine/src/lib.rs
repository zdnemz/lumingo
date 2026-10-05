//! Pronunciation engine: reference text to phonemes, forced alignment of a
//! phoneme recogniser's posteriors, goodness-of-pronunciation scores, honest
//! "not scored" reporting, and the blocking-or-deferred timing policy.
//!
//! Everything except the ONNX adapter is a pure function over plain data, so it
//! is tested without a model. See `docs/ASSESSMENT_SPEC.md` section 7 and
//! ADR-008 and ADR-009.
#![forbid(unsafe_code)]

pub mod arpabet;
pub mod lexicon;
pub mod phone_map;
pub mod reference;
pub mod vocab;

pub use arpabet::{Arpabet, Phone, PhoneClass, UnknownArpabet};
pub use lexicon::{Lexicon, LexiconError};
pub use phone_map::{BoundPhoneMap, PhoneMap, PhoneMapError};
pub use reference::{NotChecked, Reference, ReferenceWord, WordStatus};
pub use vocab::{ModelVocab, VocabError};
