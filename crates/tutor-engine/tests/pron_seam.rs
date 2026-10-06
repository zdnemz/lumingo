#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The pronunciation seam with the real adapter over `pron-engine` and a scripted
//! phoneme model: no ONNX model and no audio hardware are involved. What a real
//! model produces on a real recording is UNVERIFIED.

use pron_engine::{
    Arpabet, Calibration, Lexicon, LogPosteriors, LogisticCurve, ModelError, ModelVocab, PhoneMap,
    PosteriorModel, Thresholds, UnavailableModel, WordResult,
};
use speech::{CancelFlag, EngineInfo};
use tutor_engine::{DrillError, DrillRequest, DrillScorer, PronEngineDrill};

const DOMINANT: f32 = 0.9;

fn vocab() -> (ModelVocab, PhoneMap) {
    let map = PhoneMap::bundled_candidate().unwrap();
    let mut labels = vec!["<pad>".to_owned()];
    for symbol in Arpabet::ALL {
        let first = map.labels_of(symbol)[0].clone();
        if !labels.contains(&first) {
            labels.push(first);
        }
    }
    (ModelVocab::from_labels(labels, "<pad>").unwrap(), map)
}

/// One run of `frames` frames dominated by the column of `symbol`.
fn segment(vocab: &ModelVocab, map: &PhoneMap, symbol: Option<Arpabet>, frames: usize) -> Vec<f32> {
    let column = match symbol {
        Some(symbol) => vocab.index_of(&map.labels_of(symbol)[0]).unwrap(),
        None => vocab.blank(),
    };
    let rest = (1.0 - DOMINANT) / (vocab.len() - 1) as f32;
    let mut out = Vec::new();
    for _ in 0..frames {
        for c in 0..vocab.len() {
            out.push(if c == column { DOMINANT } else { rest }.ln());
        }
    }
    out
}

struct ScriptedModel {
    vocab: ModelVocab,
    matrix: LogPosteriors,
}

impl PosteriorModel for ScriptedModel {
    fn vocab(&self) -> &ModelVocab {
        &self.vocab
    }

    fn log_posteriors(
        &mut self,
        _samples: &[f32],
        cancel: &CancelFlag,
    ) -> Result<LogPosteriors, ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        Ok(self.matrix.clone())
    }
}

fn calibration() -> Calibration {
    Calibration {
        curve: Some(LogisticCurve {
            midpoint: -2.0,
            slope: 1.5,
        }),
        thresholds: Thresholds {
            vowel: Some(-3.0),
            stop: Some(-3.0),
            fricative: Some(-3.0),
            affricate: Some(-3.0),
            nasal: Some(-3.0),
            liquid: Some(-3.0),
            glide: Some(-3.0),
        },
        free_speech_margin: None,
        validated_by: None,
    }
}

fn lexicon() -> Lexicon {
    Lexicon::parse("THANK  TH AE1 NG K\nYOU  Y UW1\n").unwrap()
}

/// A drill over "thank you" where the learner said `first` for the first sound.
fn drill(first: Arpabet) -> PronEngineDrill<ScriptedModel> {
    let (vocab, map) = vocab();
    let mut data = segment(&vocab, &map, None, 2);
    for symbol in [
        first,
        Arpabet::AE,
        Arpabet::NG,
        Arpabet::K,
        Arpabet::Y,
        Arpabet::UW,
    ] {
        data.extend(segment(&vocab, &map, Some(symbol), 4));
        data.extend(segment(&vocab, &map, None, 1));
    }
    let frames = data.len() / vocab.len();
    let matrix = LogPosteriors::new(data, frames, vocab.len()).unwrap();
    let bound = map.bind(&vocab).unwrap();
    PronEngineDrill::new(
        ScriptedModel {
            vocab: vocab.clone(),
            matrix,
        },
        lexicon(),
        bound,
        calibration(),
        EngineInfo::new("scripted-phoneme-model", "0").with_model_checksum("none"),
        "test-thresholds/1",
        20,
    )
}

fn request() -> DrillRequest {
    DrillRequest {
        reference_text: "Thank you.".to_owned(),
        focus: vec!["TH".to_owned()],
        samples: vec![0.0; 16_000],
    }
}

#[test]
fn a_clean_reading_is_scored_in_drill_mode_with_the_focus_reported() {
    let scorer = drill(Arpabet::TH);
    let done = scorer.score(&request(), &CancelFlag::new()).unwrap();
    assert_eq!(done.frame_ms, 20);
    let report = done.report;
    assert!(report.experimental);
    assert!(report.counts_toward_assessment);
    assert!(report.utterance_score.is_some());
    assert_eq!(report.words_scored, 2);
    assert_eq!(report.focus.len(), 1);
    assert_eq!(report.focus[0].symbol, Arpabet::TH);
    assert_eq!(report.focus[0].occurrences, 1);
    assert_eq!(report.focus[0].flagged, 0);
    assert_eq!(scorer.engine().id, "scripted-phoneme-model");
    assert_eq!(scorer.threshold_set_version(), "test-thresholds/1");
}

#[test]
fn a_substituted_sound_is_flagged_and_the_heard_sound_is_named() {
    let scorer = drill(Arpabet::T);
    let report = scorer.score(&request(), &CancelFlag::new()).unwrap().report;
    assert_eq!(report.focus[0].flagged, 1);
    let WordResult::Scored { phonemes, .. } = &report.words[0] else {
        panic!("the first word is scored");
    };
    let th = &phonemes[0];
    assert_eq!(th.symbol, Arpabet::TH);
    assert_eq!(th.flagged, Some(true));
    let heard = th.heard.as_ref().expect("a heard sound");
    assert_eq!(heard.arpabet, Some(Arpabet::T));
}

#[test]
fn a_cancelled_analysis_and_an_unknown_focus_symbol_are_reported_as_such() {
    let scorer = drill(Arpabet::TH);
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert!(matches!(
        scorer.score(&request(), &cancel),
        Err(DrillError::Cancelled)
    ));
    let mut bad = request();
    bad.focus = vec!["QQ".to_owned()];
    assert!(matches!(
        scorer.score(&bad, &CancelFlag::new()),
        Err(DrillError::Failed(message)) if message.contains("QQ")
    ));
}

#[test]
fn a_missing_model_is_unavailable_and_never_a_score() {
    let (vocab, map) = vocab();
    let scorer = PronEngineDrill::new(
        UnavailableModel::new(vocab.clone(), "built without the ort-backend feature"),
        lexicon(),
        map.bind(&vocab).unwrap(),
        calibration(),
        EngineInfo::new("unavailable", "0"),
        "none",
        20,
    );
    let error = scorer.score(&request(), &CancelFlag::new()).unwrap_err();
    assert!(
        matches!(&error, DrillError::Unavailable(reason) if reason.contains("ort-backend")),
        "{error}"
    );
}

#[test]
fn words_the_lexicon_lacks_are_not_checked_and_an_unvalidated_calibration_gives_no_score() {
    let (vocab, map) = vocab();
    let matrix_data = segment(&vocab, &map, Some(Arpabet::TH), 4);
    let matrix = LogPosteriors::new(matrix_data, 4, vocab.len()).unwrap();
    let scorer = PronEngineDrill::new(
        ScriptedModel {
            vocab: vocab.clone(),
            matrix,
        },
        lexicon(),
        map.bind(&vocab).unwrap(),
        Calibration::unvalidated(),
        EngineInfo::new("scripted-phoneme-model", "0"),
        "unvalidated",
        20,
    );
    let mut unknown = request();
    unknown.reference_text = "Zzyzx quux".to_owned();
    let report = scorer.score(&unknown, &CancelFlag::new()).unwrap().report;
    assert_eq!(report.utterance_score, None);
    assert_eq!(report.words_not_checked, 2);
    // Without a curve there is no 0 to 1 score even for known words.
    let known = scorer.score(&request(), &CancelFlag::new()).unwrap().report;
    assert!(!known.scores_calibrated);
}
