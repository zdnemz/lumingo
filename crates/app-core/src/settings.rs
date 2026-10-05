//! Reading, checking and storing [`Settings`].
//!
//! The first four fields live on the learner profile row. The two switches are
//! rows of `app_settings`. Neither holds a key or any learner text beyond the
//! display name the learner typed.

use storage::{Database, Profile, Timestamp};

use crate::api::{AdaptiveTiming, Settings};
use crate::error::{CoreError, CoreResult};

const KEY_ADAPTIVE_TIMING: &str = "pron.adaptive_timing";
const KEY_KEEP_RECORDINGS: &str = "audio.keep_recordings";
const MAX_DISPLAY_NAME_CHARS: usize = 64;

impl AdaptiveTiming {
    fn as_stored(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::AlwaysWait => "always_wait",
            Self::NeverWait => "never_wait",
        }
    }

    fn from_stored(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "always_wait" => Some(Self::AlwaysWait),
            "never_wait" => Some(Self::NeverWait),
            _ => None,
        }
    }
}

impl Settings {
    /// Refuses values the profile and the rest of the program cannot use, and
    /// returns the settings with the display name trimmed.
    pub fn validated(mut self) -> CoreResult<Self> {
        let name = self.display_name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_DISPLAY_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(CoreError::InvalidInput(
                "the display name must have 1 to 64 characters and no control characters"
                    .to_owned(),
            ));
        }
        self.display_name = name.to_owned();
        if self.l1.len() != 2 || !self.l1.bytes().all(|b| b.is_ascii_lowercase()) {
            return Err(CoreError::InvalidInput(
                "the first language is a two-letter lower-case ISO 639-1 code".to_owned(),
            ));
        }
        Ok(self)
    }
}

/// Reads the settings of `profile`. A stored switch this build does not
/// understand falls back to its default instead of failing the start-up.
pub(crate) async fn load(db: &Database, profile: &Profile) -> CoreResult<Settings> {
    let adaptive_timing = match db.settings().get(KEY_ADAPTIVE_TIMING).await? {
        Some(text) => AdaptiveTiming::from_stored(&text).unwrap_or_else(|| {
            tracing::warn!("a stored timing setting is not understood; using the default");
            AdaptiveTiming::default()
        }),
        None => AdaptiveTiming::default(),
    };
    let keep_recordings = db
        .settings()
        .get(KEY_KEEP_RECORDINGS)
        .await?
        .is_some_and(|text| text == "true");
    Ok(Settings {
        display_name: profile.display_name.clone(),
        ui_language: profile.ui_language.into(),
        l1: profile.l1.clone(),
        l1_help_mode: profile.l1_help_mode.into(),
        adaptive_timing,
        keep_recordings,
    })
}

/// Stores validated settings. Two writes, profile first: if the second fails,
/// the switches keep their old values and the call reports the error.
pub(crate) async fn save(
    db: &Database,
    profile: &Profile,
    settings: &Settings,
    now: &Timestamp,
) -> CoreResult<()> {
    let updated = Profile {
        display_name: settings.display_name.clone(),
        ui_language: settings.ui_language.into(),
        l1: settings.l1.clone(),
        l1_help_mode: settings.l1_help_mode.into(),
        ..profile.clone()
    };
    db.profiles().update(&updated).await?;
    db.settings()
        .set(
            KEY_ADAPTIVE_TIMING,
            settings.adaptive_timing.as_stored(),
            now,
        )
        .await?;
    db.settings()
        .set(
            KEY_KEEP_RECORDINGS,
            if settings.keep_recordings {
                "true"
            } else {
                "false"
            },
            now,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{L1HelpMode, UiLanguage};

    fn valid() -> Settings {
        Settings {
            display_name: "  Sari  ".to_owned(),
            ui_language: UiLanguage::Id,
            l1: "id".to_owned(),
            l1_help_mode: L1HelpMode::Auto,
            adaptive_timing: AdaptiveTiming::Auto,
            keep_recordings: false,
        }
    }

    #[test]
    fn a_valid_value_is_kept_with_the_name_trimmed() {
        assert_eq!(valid().validated().expect("valid").display_name, "Sari");
    }

    #[test]
    fn bad_values_are_refused() {
        type Change = fn(&mut Settings);
        let cases: [(&str, Change); 6] = [
            ("empty name", |s| s.display_name = "   ".to_owned()),
            ("long name", |s| s.display_name = "x".repeat(65)),
            ("control character", |s| s.display_name = "a\nb".to_owned()),
            ("three letter language", |s| s.l1 = "ind".to_owned()),
            ("upper case language", |s| s.l1 = "ID".to_owned()),
            ("digit language", |s| s.l1 = "i1".to_owned()),
        ];
        for (label, change) in cases {
            let mut settings = valid();
            change(&mut settings);
            assert!(
                matches!(settings.validated(), Err(CoreError::InvalidInput(_))),
                "{label}"
            );
        }
    }

    #[test]
    fn the_timing_names_round_trip() {
        for value in [
            AdaptiveTiming::Auto,
            AdaptiveTiming::AlwaysWait,
            AdaptiveTiming::NeverWait,
        ] {
            assert_eq!(AdaptiveTiming::from_stored(value.as_stored()), Some(value));
        }
        assert_eq!(AdaptiveTiming::from_stored("sometimes"), None);
    }
}
