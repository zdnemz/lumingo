//! The API key type. It has no `Display`, no `Serialize`, and a `Debug` that
//! prints nothing of the value. The only way out is `expose`, which the adapters
//! call once, to build a sensitive header value.

use std::fmt;

/// A provider key held in memory only.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

/// Why a key was refused. Never carries the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("the key is empty")]
    Empty,
    #[error("the key contains whitespace or characters that cannot be sent in an HTTP header")]
    InvalidCharacters,
}

impl ApiKey {
    /// Accepts visible ASCII only. Surrounding whitespace is trimmed first because
    /// keys pasted into a settings field or a `.env` file often carry a newline.
    pub fn new(raw: &str) -> Result<Self, KeyError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(KeyError::Empty);
        }
        if !trimmed.chars().all(|c| c.is_ascii_graphic()) {
            return Err(KeyError::InvalidCharacters);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The raw value, for building an authentication header and for removing the
    /// key from provider error text. Do not log or format the result.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// The last four characters, for display in settings. Keys shorter than eight
    /// characters return `None`, because four characters would be half the key.
    pub fn last4(&self) -> Option<String> {
        let count = self.0.chars().count();
        if count < 8 {
            return None;
        }
        Some(self.0.chars().skip(count - 4).collect())
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_value() {
        let key = ApiKey::new("sk-test-1234567890abcdef").expect("valid");
        let printed = format!("{key:?} {:#?}", Some(&key));
        assert!(!printed.contains("1234567890"));
        assert!(!printed.contains("cdef"));
        assert!(printed.contains("redacted"));
    }

    #[test]
    fn last4_needs_a_long_enough_key() {
        assert_eq!(
            ApiKey::new("sk-test-1234567890abcdef")
                .expect("valid")
                .last4()
                .as_deref(),
            Some("cdef")
        );
        assert_eq!(ApiKey::new("short").expect("valid").last4(), None);
    }

    #[test]
    fn trims_and_rejects_bad_values() {
        assert_eq!(
            ApiKey::new("  abc12345  \n").expect("valid").expose(),
            "abc12345"
        );
        assert_eq!(ApiKey::new("  ").unwrap_err(), KeyError::Empty);
        assert_eq!(
            ApiKey::new("two words").unwrap_err(),
            KeyError::InvalidCharacters
        );
        assert_eq!(
            ApiKey::new("naïve-key").unwrap_err(),
            KeyError::InvalidCharacters
        );
    }
}
