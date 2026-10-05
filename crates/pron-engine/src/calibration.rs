//! Thresholds and the score curve, kept as configuration.
//!
//! No value here is validated. ASSESSMENT_SPEC 7.1 steps 6 and 7 say the
//! thresholds per class and the logistic curve are chosen in validation
//! (ROADMAP S3-10, which needs the owner's recordings). Until then the shipped
//! configuration is [`Calibration::unvalidated`]: it has no threshold and no
//! curve, so the engine reports measured GOP and aligned spans but no 0..1
//! score and no flag. There is deliberately no `Default` with numbers in it.

use serde::{Deserialize, Serialize};

use crate::arpabet::PhoneClass;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CalibrationError {
    #[error("the curve slope must be a positive finite number, got {0}")]
    BadSlope(f32),
    #[error("the curve midpoint must be finite, got {0}")]
    BadMidpoint(f32),
    #[error("the threshold for {class:?} must be a finite GOP value at most 0, got {value}")]
    BadThreshold { class: PhoneClass, value: f32 },
    #[error("the free-speech margin must be a finite number at least 0, got {0}")]
    BadMargin(f32),
    #[error("the calibration file is not valid: {0}")]
    Toml(String),
}

/// Maps GOP (at most 0) to 0..1: `1 / (1 + exp(-slope * (gop - midpoint)))`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogisticCurve {
    pub midpoint: f32,
    pub slope: f32,
}

impl LogisticCurve {
    pub fn apply(&self, gop: f32) -> f32 {
        1.0 / (1.0 + (-self.slope * (gop - self.midpoint)).exp())
    }
}

/// One GOP threshold per phone class. `None` means "no threshold chosen", and
/// a phone of that class is then not flagged either way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    pub vowel: Option<f32>,
    pub stop: Option<f32>,
    pub fricative: Option<f32>,
    pub affricate: Option<f32>,
    pub nasal: Option<f32>,
    pub liquid: Option<f32>,
    pub glide: Option<f32>,
}

impl Thresholds {
    pub fn get(&self, class: PhoneClass) -> Option<f32> {
        match class {
            PhoneClass::Vowel => self.vowel,
            PhoneClass::Stop => self.stop,
            PhoneClass::Fricative => self.fricative,
            PhoneClass::Affricate => self.affricate,
            PhoneClass::Nasal => self.nasal,
            PhoneClass::Liquid => self.liquid,
            PhoneClass::Glide => self.glide,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Calibration {
    /// Absent until validation fits it. Without it no 0..1 score exists.
    pub curve: Option<LogisticCurve>,
    pub thresholds: Thresholds,
    /// How much lower (stricter) the threshold is in free speech. Absent until
    /// validation sets it; free speech raises no flag without it.
    pub free_speech_margin: Option<f32>,
    /// Where these values were validated, for example the validation report.
    /// `None` means they were not.
    pub validated_by: Option<String>,
}

impl Calibration {
    /// The shipped state: nothing chosen, nothing validated.
    pub fn unvalidated() -> Calibration {
        Calibration::default()
    }

    pub fn is_validated(&self) -> bool {
        self.validated_by.is_some()
    }

    pub fn from_toml(text: &str) -> Result<Calibration, CalibrationError> {
        let calibration: Calibration =
            toml::from_str(text).map_err(|e| CalibrationError::Toml(e.to_string()))?;
        calibration.validate()?;
        Ok(calibration)
    }

    pub fn validate(&self) -> Result<(), CalibrationError> {
        if let Some(curve) = &self.curve {
            if !(curve.slope.is_finite() && curve.slope > 0.0) {
                return Err(CalibrationError::BadSlope(curve.slope));
            }
            if !curve.midpoint.is_finite() {
                return Err(CalibrationError::BadMidpoint(curve.midpoint));
            }
        }
        for class in PhoneClass::ALL {
            if let Some(value) = self.thresholds.get(class)
                && !(value.is_finite() && value <= 0.0)
            {
                return Err(CalibrationError::BadThreshold { class, value });
            }
        }
        if let Some(margin) = self.free_speech_margin
            && !(margin.is_finite() && margin >= 0.0)
        {
            return Err(CalibrationError::BadMargin(margin));
        }
        Ok(())
    }

    /// The GOP below which a phone of `class` is flagged, or `None` when no
    /// decision can be made. In free speech the threshold is lowered by the
    /// margin so that only clearer errors are raised.
    pub fn threshold_for(&self, class: PhoneClass, free_speech: bool) -> Option<f32> {
        let base = self.thresholds.get(class)?;
        if free_speech {
            Some(base - self.free_speech_margin?)
        } else {
            Some(base)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_state_has_no_numbers_and_is_not_validated() {
        let c = Calibration::unvalidated();
        assert_eq!(c.curve, None);
        assert_eq!(c.free_speech_margin, None);
        assert!(!c.is_validated());
        for class in PhoneClass::ALL {
            assert_eq!(c.thresholds.get(class), None);
            assert_eq!(c.threshold_for(class, false), None);
            assert_eq!(c.threshold_for(class, true), None);
        }
        assert_eq!(c, Calibration::default());
    }

    #[test]
    fn curve_is_logistic() {
        let curve = LogisticCurve {
            midpoint: -2.0,
            slope: 1.5,
        };
        assert!((curve.apply(-2.0) - 0.5).abs() < 1e-6);
        assert!(curve.apply(0.0) > curve.apply(-1.0));
        assert!(curve.apply(-1.0) > curve.apply(-4.0));
        assert!(curve.apply(f32::NEG_INFINITY) == 0.0);
        assert!(curve.apply(0.0) < 1.0);
    }

    #[test]
    fn free_speech_is_stricter_by_the_margin() {
        let c = Calibration {
            thresholds: Thresholds {
                stop: Some(-3.0),
                ..Thresholds::default()
            },
            free_speech_margin: Some(1.0),
            ..Calibration::default()
        };
        assert_eq!(c.threshold_for(PhoneClass::Stop, false), Some(-3.0));
        assert_eq!(c.threshold_for(PhoneClass::Stop, true), Some(-4.0));
        assert_eq!(c.threshold_for(PhoneClass::Vowel, false), None);
        let no_margin = Calibration {
            free_speech_margin: None,
            ..c
        };
        assert_eq!(no_margin.threshold_for(PhoneClass::Stop, true), None);
    }

    #[test]
    fn validation_rejects_impossible_values() {
        let bad_slope = Calibration {
            curve: Some(LogisticCurve {
                midpoint: 0.0,
                slope: 0.0,
            }),
            ..Calibration::default()
        };
        assert_eq!(bad_slope.validate(), Err(CalibrationError::BadSlope(0.0)));
        let bad_mid = Calibration {
            curve: Some(LogisticCurve {
                midpoint: f32::NAN,
                slope: 1.0,
            }),
            ..Calibration::default()
        };
        assert!(matches!(
            bad_mid.validate(),
            Err(CalibrationError::BadMidpoint(_))
        ));
        let positive = Calibration {
            thresholds: Thresholds {
                glide: Some(0.5),
                ..Thresholds::default()
            },
            ..Calibration::default()
        };
        assert!(matches!(
            positive.validate(),
            Err(CalibrationError::BadThreshold {
                class: PhoneClass::Glide,
                ..
            })
        ));
        let bad_margin = Calibration {
            free_speech_margin: Some(-0.1),
            ..Calibration::default()
        };
        assert_eq!(
            bad_margin.validate(),
            Err(CalibrationError::BadMargin(-0.1))
        );
    }

    #[test]
    fn loads_from_toml_and_rejects_unknown_keys() {
        let text = "free_speech_margin = 0.5\n[curve]\nmidpoint = -1.0\nslope = 2.0\n[thresholds]\nvowel = -2.0\n";
        let c = Calibration::from_toml(text).expect("loads");
        assert_eq!(c.thresholds.vowel, Some(-2.0));
        assert_eq!(c.thresholds.stop, None);
        assert!(!c.is_validated());
        assert!(Calibration::from_toml("surprise = 1\n").is_err());
        assert!(Calibration::from_toml("[thresholds]\nvowel = 2.0\n").is_err());
        assert_eq!(
            Calibration::from_toml("").expect("empty file"),
            Calibration::unvalidated()
        );
    }
}
