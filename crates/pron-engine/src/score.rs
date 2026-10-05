//! From posteriors and a reference to per-phoneme, per-word and per-utterance
//! results (ASSESSMENT_SPEC 7.1 steps 4 to 7).
//!
//! Three rules shape the types here.
//!
//! 1. A phoneme result exists only for a phone that the alignment placed on
//!    frames. Words that were not looked up, and every word of an utterance that
//!    could not be aligned, have no phoneme list at all.
//! 2. "Not scored" is its own state, never a zero. A missing score is `None` and
//!    the report says why.
//! 3. Nothing is invented. Without a validated curve there is no 0..1 score;
//!    without a threshold there is no flag. The measured GOP is always present
//!    for an aligned phone.

use serde::Serialize;

use crate::align::{AlignError, WordSpec, align};
use crate::arpabet::{Arpabet, PhoneClass};
use crate::calibration::{Calibration, CalibrationError};
use crate::gop::{GopError, gop};
use crate::phone_map::BoundPhoneMap;
use crate::posteriors::LogPosteriors;
use crate::reference::{NotChecked, Reference, WordStatus};
use crate::vocab::ModelVocab;

/// ASSESSMENT_SPEC 7.2: at most three highlighted words per free-speech turn.
pub const MAX_HIGHLIGHTED_WORDS: usize = 3;

/// The two modes of ASSESSMENT_SPEC 7.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mode {
    /// Authored reference text, known to be what the learner was asked to say.
    Drill,
    /// The reference is an STT transcript. `transcript_rejected` is set when the
    /// learner rejected or edited it away: nothing is then scored.
    FreeSpeech { transcript_rejected: bool },
}

impl Mode {
    /// Free-speech results never count toward a score (ADR-009).
    pub fn counts_toward_assessment(self) -> bool {
        matches!(self, Mode::Drill)
    }

    fn is_free_speech(self) -> bool {
        matches!(self, Mode::FreeSpeech { .. })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScoreError {
    #[error(transparent)]
    Calibration(#[from] CalibrationError),
    #[error(
        "label counts disagree: posteriors have {posteriors}, the vocabulary {vocab}, the bound phone map {map}"
    )]
    LabelCount {
        posteriors: usize,
        vocab: usize,
        map: usize,
    },
    #[error("alignment failed: {0}")]
    Align(AlignError),
    #[error("GOP failed on an aligned phone: {0}")]
    Gop(#[from] GopError),
}

/// Everything `score_utterance` reads.
pub struct ScoreRequest<'a> {
    pub reference: &'a Reference,
    pub posteriors: &'a LogPosteriors,
    pub map: &'a BoundPhoneMap,
    pub vocab: &'a ModelVocab,
    pub calibration: &'a Calibration,
    pub mode: Mode,
    /// The phonemes a drill targets. They are reported separately.
    pub focus: &'a [Arpabet],
}

/// What the recogniser heard instead of the expected sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Heard {
    /// The model's own label.
    pub label: String,
    /// The ARPAbet symbol the label stands for, only when exactly one does.
    pub arpabet: Option<Arpabet>,
    pub frames_won: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PhonemeResult {
    pub symbol: Arpabet,
    pub class: PhoneClass,
    /// Aligned frames `[start_frame, end_frame)`.
    pub start_frame: usize,
    pub end_frame: usize,
    /// Measured GOP, at most 0.
    pub gop: f32,
    /// 0..1 from the logistic curve. `None` while no curve is configured.
    pub score: Option<f32>,
    /// `Some(true)` when GOP is below the class threshold, `Some(false)` when
    /// not, `None` when no threshold applies (nothing was decided).
    pub flagged: Option<bool>,
    pub heard: Option<Heard>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WordResult {
    Scored {
        text: String,
        /// The pronunciation the alignment preferred.
        pronunciation: Vec<Arpabet>,
        start_frame: usize,
        end_frame: usize,
        phonemes: Vec<PhonemeResult>,
        /// Mean of the phoneme scores, `None` while no curve is configured.
        score: Option<f32>,
        /// Phones whose flag is `Some(true)`.
        flagged_phonemes: usize,
        /// A neighbouring word was not checked. Its audio sits between the
        /// aligned words and can pull the nearest phones' GOP down, so treat
        /// this word's numbers with more caution.
        adjacent_to_unchecked: bool,
    },
    /// The word was not looked up or could not be pronounced from the lexicon.
    NotChecked { text: String, reason: NotChecked },
    /// The word is in the lexicon but the whole utterance was not scored.
    NotScored { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum NotScoredReason {
    /// Free speech whose transcript the learner rejected.
    TranscriptRejected,
    /// Not one word of the reference was in the lexicon.
    NoWordsChecked,
    NoAudio,
    /// The audio is too short for the reference, or the posteriors allow no
    /// path through it. Nothing is forced.
    AlignmentInfeasible {
        frames: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    /// Phones were aligned. Scores may still be absent: see
    /// `UtteranceReport::scores_calibrated`.
    Aligned,
    NotScored(NotScoredReason),
}

/// A drill's focus phoneme, summarised over every place it occurs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FocusResult {
    pub symbol: Arpabet,
    /// How many aligned occurrences there were. 0 means the phoneme does not
    /// occur in the checked words, which is not the same as a bad score.
    pub occurrences: usize,
    pub mean_gop: Option<f32>,
    pub mean_score: Option<f32>,
    pub flagged: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UtteranceReport {
    /// Always true: the UI must label pronunciation feedback experimental
    /// (ASSESSMENT_SPEC section 12).
    pub experimental: bool,
    pub mode: Mode,
    pub counts_toward_assessment: bool,
    pub outcome: Outcome,
    /// One entry per word of the reference, in order.
    pub words: Vec<WordResult>,
    /// Mean of the scored words' scores. `None` when nothing was scored or no
    /// curve is configured.
    pub utterance_score: Option<f32>,
    pub words_scored: usize,
    pub words_not_checked: usize,
    /// A logistic curve was configured, so 0..1 scores exist.
    pub scores_calibrated: bool,
    /// The configuration says where it was validated. False until S3-10.
    pub calibration_validated: bool,
    /// Drill focus phonemes. Empty when the utterance was not aligned.
    pub focus: Vec<FocusResult>,
    /// Free speech only: indices into `words` of at most three words to show,
    /// worst first.
    pub highlighted_words: Vec<usize>,
    pub frames: usize,
}

fn mean(values: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, n) = values.fold((0.0f64, 0usize), |(s, n), v| (s + f64::from(v), n + 1));
    (n > 0).then(|| (sum / n as f64) as f32)
}

/// Scores one utterance.
///
/// Errors are for a caller or configuration mistake (mismatched label counts,
/// an impossible calibration). Audio that cannot be scored is not an error: it
/// comes back as `Outcome::NotScored` with the reason.
pub fn score_utterance(req: &ScoreRequest<'_>) -> Result<UtteranceReport, ScoreError> {
    req.calibration.validate()?;
    if req.posteriors.labels() != req.vocab.len() || req.map.vocab_len() != req.vocab.len() {
        return Err(ScoreError::LabelCount {
            posteriors: req.posteriors.labels(),
            vocab: req.vocab.len(),
            map: req.map.vocab_len(),
        });
    }
    let frames = req.posteriors.frames();
    let not_scored = |reason: NotScoredReason| UtteranceReport {
        experimental: true,
        mode: req.mode,
        counts_toward_assessment: req.mode.counts_toward_assessment(),
        outcome: Outcome::NotScored(reason),
        words: req
            .reference
            .words
            .iter()
            .map(|w| match &w.status {
                WordStatus::Checked { .. } => WordResult::NotScored {
                    text: w.text.clone(),
                },
                WordStatus::NotChecked { reason } => WordResult::NotChecked {
                    text: w.text.clone(),
                    reason: *reason,
                },
            })
            .collect(),
        utterance_score: None,
        words_scored: 0,
        words_not_checked: req.reference.words.len() - req.reference.checked_count(),
        scores_calibrated: req.calibration.curve.is_some(),
        calibration_validated: req.calibration.is_validated(),
        focus: Vec::new(),
        highlighted_words: Vec::new(),
        frames,
    };

    if matches!(
        req.mode,
        Mode::FreeSpeech {
            transcript_rejected: true
        }
    ) {
        return Ok(not_scored(NotScoredReason::TranscriptRejected));
    }

    // Only words with a pronunciation enter the alignment. Variants that differ
    // only in stress are one candidate: stress is not assessed.
    let mut checked_indices = Vec::new();
    let mut specs = Vec::new();
    for (i, word) in req.reference.words.iter().enumerate() {
        if let WordStatus::Checked { pronunciations } = &word.status {
            let mut variants: Vec<Vec<Arpabet>> = Vec::new();
            for p in pronunciations {
                let symbols: Vec<Arpabet> = p.iter().map(|ph| ph.symbol).collect();
                if !variants.contains(&symbols) {
                    variants.push(symbols);
                }
            }
            checked_indices.push(i);
            specs.push(WordSpec { variants });
        }
    }
    if specs.is_empty() {
        return Ok(not_scored(NotScoredReason::NoWordsChecked));
    }
    if frames == 0 {
        return Ok(not_scored(NotScoredReason::NoAudio));
    }
    let alignment = match align(req.posteriors, req.map, &specs) {
        Ok(a) => a,
        Err(AlignError::NoFeasiblePath { frames }) => {
            return Ok(not_scored(NotScoredReason::AlignmentInfeasible { frames }));
        }
        Err(other) => return Err(ScoreError::Align(other)),
    };

    let free = req.mode.is_free_speech();
    let blank = req.map.blank();
    let words_ref = &req.reference.words;
    let mut words: Vec<WordResult> = words_ref
        .iter()
        .map(|w| match &w.status {
            WordStatus::NotChecked { reason } => WordResult::NotChecked {
                text: w.text.clone(),
                reason: *reason,
            },
            // Replaced below for every checked word.
            WordStatus::Checked { .. } => WordResult::NotScored {
                text: w.text.clone(),
            },
        })
        .collect();

    for (k, aligned) in alignment.words.iter().enumerate() {
        let ref_index = checked_indices[k];
        let pronunciation: Vec<Arpabet> = specs[k].variants[aligned.variant].clone();
        let mut phonemes = Vec::with_capacity(aligned.phones.len());
        for phone in &aligned.phones {
            let measured = gop(
                req.posteriors,
                req.map.columns_of(phone.symbol),
                blank,
                phone.frames(),
            )?;
            let class = phone.symbol.class();
            let heard = measured.heard.map(|h| Heard {
                label: req.vocab.label(h.column).unwrap_or_default().to_owned(),
                arpabet: match req.map.symbols_of_column(h.column) {
                    [only] => Some(*only),
                    _ => None,
                },
                frames_won: h.frames_won,
            });
            phonemes.push(PhonemeResult {
                symbol: phone.symbol,
                class,
                start_frame: phone.start,
                end_frame: phone.end,
                gop: measured.gop,
                score: req.calibration.curve.map(|c| c.apply(measured.gop)),
                flagged: req
                    .calibration
                    .threshold_for(class, free)
                    .map(|t| measured.gop < t),
                heard,
            });
        }
        let score = if phonemes.iter().all(|p| p.score.is_some()) {
            mean(phonemes.iter().filter_map(|p| p.score))
        } else {
            None
        };
        let flagged_phonemes = phonemes.iter().filter(|p| p.flagged == Some(true)).count();
        let adjacent_to_unchecked = (ref_index > 0
            && matches!(
                words_ref[ref_index - 1].status,
                WordStatus::NotChecked { .. }
            ))
            || matches!(
                words_ref.get(ref_index + 1).map(|w| &w.status),
                Some(WordStatus::NotChecked { .. })
            );
        let span = aligned
            .span()
            .ok_or(ScoreError::Align(AlignError::Internal(
                "an aligned word has no phones",
            )))?;
        words[ref_index] = WordResult::Scored {
            text: words_ref[ref_index].text.clone(),
            pronunciation,
            start_frame: span.start,
            end_frame: span.end,
            phonemes,
            score,
            flagged_phonemes,
            adjacent_to_unchecked,
        };
    }

    let utterance_score = mean(words.iter().filter_map(|w| match w {
        WordResult::Scored { score, .. } => *score,
        _ => None,
    }));
    let words_scored = words
        .iter()
        .filter(|w| matches!(w, WordResult::Scored { .. }))
        .count();

    let all_phones = || {
        words.iter().flat_map(|w| match w {
            WordResult::Scored { phonemes, .. } => phonemes.as_slice(),
            _ => &[],
        })
    };
    let mut focus = Vec::new();
    for symbol in req.focus {
        if focus.iter().any(|f: &FocusResult| f.symbol == *symbol) {
            continue;
        }
        let hits: Vec<&PhonemeResult> = all_phones().filter(|p| p.symbol == *symbol).collect();
        focus.push(FocusResult {
            symbol: *symbol,
            occurrences: hits.len(),
            mean_gop: mean(hits.iter().map(|p| p.gop)),
            mean_score: if hits.iter().all(|p| p.score.is_some()) {
                mean(hits.iter().filter_map(|p| p.score))
            } else {
                None
            },
            flagged: hits.iter().filter(|p| p.flagged == Some(true)).count(),
        });
    }

    let mut highlighted_words = Vec::new();
    if free {
        let mut candidates: Vec<(usize, f32)> = words
            .iter()
            .enumerate()
            .filter_map(|(i, w)| match w {
                WordResult::Scored {
                    phonemes,
                    flagged_phonemes,
                    ..
                } if *flagged_phonemes > 0 => Some((
                    i,
                    phonemes.iter().map(|p| p.gop).fold(f32::INFINITY, f32::min),
                )),
                _ => None,
            })
            .collect();
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        highlighted_words = candidates
            .into_iter()
            .take(MAX_HIGHLIGHTED_WORDS)
            .map(|(i, _)| i)
            .collect();
    }

    Ok(UtteranceReport {
        experimental: true,
        mode: req.mode,
        counts_toward_assessment: req.mode.counts_toward_assessment(),
        outcome: Outcome::Aligned,
        words,
        utterance_score,
        words_scored,
        words_not_checked: words_ref.len() - req.reference.checked_count(),
        scores_calibrated: req.calibration.curve.is_some(),
        calibration_validated: req.calibration.is_validated(),
        focus,
        highlighted_words,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arpabet::Arpabet::*;
    use crate::calibration::{LogisticCurve, Thresholds};
    use crate::lexicon::Lexicon;
    use crate::testing::{Synthetic, bound_map};

    fn lexicon() -> Lexicon {
        Lexicon::parse(
            "cat K AE1 T\n\
             sat S AE1 T\n\
             read R EH1 D\n\
             read(2) R IY1 D\n\
             sheep SH IY1 P\n\
             ship SH IH1 P\n\
             bad B AE1 D\n\
             zoo Z UW1\n\
             go G OW1\n",
        )
        .expect("lexicon")
    }

    /// TEST VALUES ONLY. They have no validation behind them and exist so the
    /// flag and score logic can be exercised.
    fn test_calibration() -> Calibration {
        let t = Some(-3.0);
        Calibration {
            curve: Some(LogisticCurve {
                midpoint: -3.0,
                slope: 1.0,
            }),
            thresholds: Thresholds {
                vowel: t,
                stop: t,
                fricative: t,
                affricate: t,
                nasal: t,
                liquid: t,
                glide: t,
            },
            free_speech_margin: Some(2.0),
            validated_by: None,
        }
    }

    struct Fixture {
        map: BoundPhoneMap,
        vocab: ModelVocab,
    }

    fn fixture() -> Fixture {
        let (map, vocab) = bound_map();
        Fixture { map, vocab }
    }

    fn run(
        f: &Fixture,
        text: &str,
        s: &Synthetic,
        calibration: &Calibration,
        mode: Mode,
        focus: &[Arpabet],
    ) -> UtteranceReport {
        let reference = Reference::from_text(text, &lexicon());
        let m = s.matrix();
        score_utterance(&ScoreRequest {
            reference: &reference,
            posteriors: &m,
            map: &f.map,
            vocab: &f.vocab,
            calibration,
            mode,
            focus,
        })
        .expect("scores")
    }

    fn scored(w: &WordResult) -> (&Vec<PhonemeResult>, Option<f32>, usize) {
        match w {
            WordResult::Scored {
                phonemes,
                score,
                flagged_phonemes,
                ..
            } => (phonemes, *score, *flagged_phonemes),
            other => panic!("expected a scored word, got {other:?}"),
        }
    }

    #[test]
    fn unvalidated_calibration_gives_measured_gop_but_no_score_and_no_flag() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(K, 2).phone(AE, 2).phone(T, 2);
        let r = run(&f, "cat", &s, &Calibration::unvalidated(), Mode::Drill, &[]);
        assert_eq!(r.outcome, Outcome::Aligned);
        assert!(r.experimental);
        assert!(!r.scores_calibrated);
        assert!(!r.calibration_validated);
        assert_eq!(r.utterance_score, None);
        let (phonemes, word_score, flagged) = scored(&r.words[0]);
        assert_eq!(word_score, None);
        assert_eq!(flagged, 0);
        assert_eq!(phonemes.len(), 3);
        for p in phonemes {
            assert!(p.gop <= 0.0 && p.gop > -0.01, "{}", p.gop);
            assert_eq!(p.score, None);
            assert_eq!(p.flagged, None);
            assert_eq!(p.heard, None);
        }
    }

    #[test]
    fn a_substituted_sound_is_flagged_and_the_heard_label_is_reported() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        // "cat" said as "gat", then a clean "sat".
        s.phone(G, 2).phone(AE, 2).phone(T, 2).blank(2);
        s.phone(S, 2).phone(AE, 2).phone(T, 2);
        let r = run(&f, "cat sat", &s, &test_calibration(), Mode::Drill, &[]);
        let (cat, cat_score, cat_flags) = scored(&r.words[0]);
        let (sat, sat_score, sat_flags) = scored(&r.words[1]);
        assert_eq!(cat[0].symbol, K);
        assert_eq!(cat[0].flagged, Some(true));
        let heard = cat[0].heard.as_ref().expect("heard G");
        assert_eq!(heard.arpabet, Some(G));
        assert_eq!(heard.label, "ɡ");
        assert!(heard.frames_won >= 1);
        assert_eq!(cat_flags, 1);
        assert_eq!(sat_flags, 0);
        assert!(
            sat.iter()
                .all(|p| p.flagged == Some(false) && p.heard.is_none())
        );
        assert!(cat_score.expect("score") < sat_score.expect("score"));
        assert!(r.scores_calibrated);
        assert!(!r.calibration_validated);
    }

    #[test]
    fn word_score_is_the_mean_of_phone_scores_and_utterance_the_mean_of_word_scores() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        // A two-phone word and a three-phone word with different quality, so a
        // mean over all phones would differ from a mean of word means.
        s.phone(Z, 2).phone(UW, 2).blank(1);
        s.phone(G, 2).phone(AE, 2).phone(T, 2); // "cat" with K said as G
        let r = run(&f, "zoo cat", &s, &test_calibration(), Mode::Drill, &[]);
        let (zoo, zoo_score, _) = scored(&r.words[0]);
        let (cat, cat_score, _) = scored(&r.words[1]);
        let zoo_mean = zoo.iter().map(|p| p.score.expect("s")).sum::<f32>() / 2.0;
        let cat_mean = cat.iter().map(|p| p.score.expect("s")).sum::<f32>() / 3.0;
        assert!((zoo_score.expect("s") - zoo_mean).abs() < 1e-6);
        assert!((cat_score.expect("s") - cat_mean).abs() < 1e-6);
        let expected = (zoo_mean + cat_mean) / 2.0;
        assert!((r.utterance_score.expect("s") - expected).abs() < 1e-6);
        let all_phones_mean = (zoo
            .iter()
            .chain(cat)
            .map(|p| p.score.expect("s"))
            .sum::<f32>())
            / 5.0;
        assert!((expected - all_phones_mean).abs() > 1e-4);
    }

    #[test]
    fn the_matching_variant_is_chosen_and_nothing_is_flagged() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(R, 2).phone(IY, 3).phone(D, 2);
        let r = run(&f, "read", &s, &test_calibration(), Mode::Drill, &[]);
        match &r.words[0] {
            WordResult::Scored {
                pronunciation,
                flagged_phonemes,
                ..
            } => {
                assert_eq!(pronunciation, &[R, IY, D]);
                assert_eq!(*flagged_phonemes, 0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn words_outside_the_lexicon_are_not_checked_and_get_no_phonemes() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(K, 2).phone(AE, 2).phone(T, 2).blank(6);
        s.phone(S, 2).phone(AE, 2).phone(T, 2);
        let r = run(
            &f,
            "cat Jakarta sat",
            &s,
            &test_calibration(),
            Mode::Drill,
            &[],
        );
        assert_eq!(r.words.len(), 3);
        assert_eq!(
            r.words[1],
            WordResult::NotChecked {
                text: "jakarta".to_owned(),
                reason: NotChecked::NotInLexicon
            }
        );
        assert_eq!(r.words_not_checked, 1);
        assert_eq!(r.words_scored, 2);
        for i in [0, 2] {
            match &r.words[i] {
                WordResult::Scored {
                    adjacent_to_unchecked,
                    ..
                } => assert!(*adjacent_to_unchecked),
                other => panic!("{other:?}"),
            }
        }
        // The utterance score covers the two checked words only.
        assert!(r.utterance_score.is_some());
    }

    #[test]
    fn a_word_far_from_an_unchecked_one_is_not_marked_adjacent() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(K, 2).phone(AE, 2).phone(T, 2).blank(1);
        s.phone(S, 2).phone(AE, 2).phone(T, 2).blank(4);
        let r = run(
            &f,
            "cat sat Jakarta",
            &s,
            &test_calibration(),
            Mode::Drill,
            &[],
        );
        let adjacent: Vec<bool> = r
            .words
            .iter()
            .filter_map(|w| match w {
                WordResult::Scored {
                    adjacent_to_unchecked,
                    ..
                } => Some(*adjacent_to_unchecked),
                _ => None,
            })
            .collect();
        assert_eq!(adjacent, [false, true]);
    }

    #[test]
    fn unscorable_audio_is_reported_not_scored_and_carries_no_numbers() {
        let f = fixture();
        let cal = test_calibration();

        let mut short = Synthetic::new(&f.vocab);
        short.phone(K, 1);
        let r = run(&f, "cat", &short, &cal, Mode::Drill, &[K]);
        assert_eq!(
            r.outcome,
            Outcome::NotScored(NotScoredReason::AlignmentInfeasible { frames: 1 })
        );
        assert!(matches!(r.words[0], WordResult::NotScored { .. }));
        assert_eq!(r.utterance_score, None);
        assert_eq!(r.words_scored, 0);
        assert!(r.focus.is_empty());

        let silence = Synthetic::new(&f.vocab);
        let r = run(&f, "cat", &silence, &cal, Mode::Drill, &[]);
        assert_eq!(r.outcome, Outcome::NotScored(NotScoredReason::NoAudio));

        let mut audio = Synthetic::new(&f.vocab);
        audio.phone(K, 3);
        let r = run(&f, "Jakarta 911", &audio, &cal, Mode::Drill, &[]);
        assert_eq!(
            r.outcome,
            Outcome::NotScored(NotScoredReason::NoWordsChecked)
        );
        assert_eq!(r.words_not_checked, 2);
        assert!(
            r.words
                .iter()
                .all(|w| matches!(w, WordResult::NotChecked { .. }))
        );

        let r = run(&f, "", &audio, &cal, Mode::Drill, &[]);
        assert_eq!(
            r.outcome,
            Outcome::NotScored(NotScoredReason::NoWordsChecked)
        );
    }

    #[test]
    fn a_rejected_free_speech_transcript_scores_nothing() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(K, 2).phone(AE, 2).phone(T, 2);
        let r = run(
            &f,
            "cat",
            &s,
            &test_calibration(),
            Mode::FreeSpeech {
                transcript_rejected: true,
            },
            &[],
        );
        assert_eq!(
            r.outcome,
            Outcome::NotScored(NotScoredReason::TranscriptRejected)
        );
        assert!(r.highlighted_words.is_empty());
        assert!(!r.counts_toward_assessment);
    }

    #[test]
    fn free_speech_is_stricter_and_never_counts_toward_assessment() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        // K at 0.009 against G at 0.9 gives GOP = ln(0.01) = -4.6. That is below
        // the drill threshold of -3 but above the free-speech threshold of
        // -3 - 2 = -5, so only the drill flags it.
        let k = s.column(K);
        let g = s.column(G);
        s.mixed(&[(k, 0.009), (g, 0.9)], 2); // GOP = ln(0.01) = -4.6
        s.phone(AE, 2).phone(T, 2);
        let cal = test_calibration();
        let drill = run(&f, "cat", &s, &cal, Mode::Drill, &[]);
        let free = run(
            &f,
            "cat",
            &s,
            &cal,
            Mode::FreeSpeech {
                transcript_rejected: false,
            },
            &[],
        );
        let (drill_phones, _, drill_flags) = scored(&drill.words[0]);
        let (free_phones, _, free_flags) = scored(&free.words[0]);
        assert!(drill_phones[0].gop < -4.0 && drill_phones[0].gop > -5.0);
        assert_eq!((drill_flags, drill_phones[0].flagged), (1, Some(true)));
        assert_eq!((free_flags, free_phones[0].flagged), (0, Some(false)));
        assert!(drill.counts_toward_assessment);
        assert!(!free.counts_toward_assessment);
        assert!(free.highlighted_words.is_empty());
    }

    #[test]
    fn free_speech_without_a_margin_raises_no_flag() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(G, 2).phone(AE, 2).phone(T, 2);
        let cal = Calibration {
            free_speech_margin: None,
            ..test_calibration()
        };
        let r = run(
            &f,
            "cat",
            &s,
            &cal,
            Mode::FreeSpeech {
                transcript_rejected: false,
            },
            &[],
        );
        let (phones, _, flags) = scored(&r.words[0]);
        assert_eq!(flags, 0);
        assert!(phones.iter().all(|p| p.flagged.is_none()));
        assert!(r.highlighted_words.is_empty());
    }

    #[test]
    fn at_most_three_words_are_highlighted_worst_first() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        let g = s.column(G);
        let k = s.column(K);
        // Five "cat"s whose K is increasingly wrong: GOP = ln(kp / 0.9) runs
        // from -5.10 down to -5.70, all below the free-speech threshold of -5.
        // Each kp stays above the filler probability (about 0.0025), so the
        // aligner keeps K on these frames rather than on a cheaper neighbour.
        let shortfalls = [0.0055, 0.0045, 0.0040, 0.0035, 0.0030];
        for (i, kp) in shortfalls.iter().enumerate() {
            s.mixed(&[(k, *kp), (g, 0.9)], 2);
            s.phone(AE, 2).phone(T, 2);
            if i + 1 < shortfalls.len() {
                s.blank(1);
            }
        }
        let r = run(
            &f,
            "cat cat cat cat cat",
            &s,
            &test_calibration(),
            Mode::FreeSpeech {
                transcript_rejected: false,
            },
            &[],
        );
        assert_eq!(r.highlighted_words, [4, 3, 2]);
        assert_eq!(r.highlighted_words.len(), MAX_HIGHLIGHTED_WORDS);
    }

    #[test]
    fn focus_phonemes_are_summarised_separately() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(SH, 2).phone(IY, 2).phone(P, 2).blank(1);
        s.phone(S, 2).phone(IH, 2).phone(P, 2); // "ship" said with S for SH
        let r = run(
            &f,
            "sheep ship",
            &s,
            &test_calibration(),
            Mode::Drill,
            &[SH, IY, SH, Z],
        );
        assert_eq!(r.focus.len(), 3, "a repeated focus symbol is reported once");
        let sh = &r.focus[0];
        assert_eq!((sh.symbol, sh.occurrences, sh.flagged), (SH, 2, 1));
        assert!(sh.mean_gop.expect("gop") < -1.0);
        let iy = &r.focus[1];
        assert_eq!((iy.symbol, iy.occurrences, iy.flagged), (IY, 1, 0));
        let z = &r.focus[2];
        assert_eq!((z.symbol, z.occurrences), (Z, 0));
        assert_eq!((z.mean_gop, z.mean_score), (None, None));
    }

    #[test]
    fn mismatched_label_counts_and_bad_calibration_are_errors() {
        let f = fixture();
        let reference = Reference::from_text("cat", &lexicon());
        let narrow = LogPosteriors::new(vec![-1.0; 6], 2, 3).expect("matrix");
        let cal = Calibration::unvalidated();
        let err = score_utterance(&ScoreRequest {
            reference: &reference,
            posteriors: &narrow,
            map: &f.map,
            vocab: &f.vocab,
            calibration: &cal,
            mode: Mode::Drill,
            focus: &[],
        });
        assert!(matches!(
            err,
            Err(ScoreError::LabelCount { posteriors: 3, .. })
        ));

        let mut s = Synthetic::new(&f.vocab);
        s.phone(K, 3);
        let m = s.matrix();
        let bad = Calibration {
            free_speech_margin: Some(-1.0),
            ..Calibration::default()
        };
        let err = score_utterance(&ScoreRequest {
            reference: &reference,
            posteriors: &m,
            map: &f.map,
            vocab: &f.vocab,
            calibration: &bad,
            mode: Mode::Drill,
            focus: &[],
        });
        assert!(matches!(err, Err(ScoreError::Calibration(_))));
    }

    #[test]
    fn the_report_serialises_with_null_for_missing_scores() {
        let f = fixture();
        let mut s = Synthetic::new(&f.vocab);
        s.phone(K, 2).phone(AE, 2).phone(T, 2);
        let r = run(&f, "cat", &s, &Calibration::unvalidated(), Mode::Drill, &[]);
        let json = serde_json::to_value(&r).expect("serialises");
        assert_eq!(json["experimental"], true);
        assert_eq!(json["utterance_score"], serde_json::Value::Null);
        assert_eq!(json["outcome"]["status"], "aligned");
        assert_eq!(json["words"][0]["kind"], "scored");
        assert_eq!(
            json["words"][0]["phonemes"][0]["score"],
            serde_json::Value::Null
        );
        assert_eq!(json["words"][0]["phonemes"][0]["symbol"], "K");
    }
}
