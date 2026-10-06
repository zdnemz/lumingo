/// An API key held in memory only. `Debug` hides it and there is no `Display`,
/// no `Serialize` and no accessor outside this crate.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// Last four characters, for the settings screen (`has_key` and a hint).
    pub fn last_four(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        chars[chars.len().saturating_sub(4)..].iter().collect()
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

const MAX_MESSAGE_CHARS: usize = 200;

/// Shortens a server message and removes the key and key-like words (`sk-...`)
/// before it can be stored or shown.
pub(crate) fn redact(text: &str, key: &ApiKey) -> String {
    let mut out = if key.0.is_empty() {
        text.to_owned()
    } else {
        text.replace(&key.0, "[key]")
    };
    out = out
        .split(' ')
        .map(|w| {
            let bare = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
            if bare.starts_with("sk-") && bare.len() > 8 {
                w.replace(bare, "[key]")
            } else {
                w.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    out.chars().take(MAX_MESSAGE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_key_and_last_four_shows_a_hint() {
        let k = ApiKey::new("sk-abcdef123456");
        assert!(!format!("{k:?}").contains("abcdef"));
        assert_eq!(k.last_four(), "3456");
        assert_eq!(ApiKey::new("ab").last_four(), "ab");
    }

    #[test]
    fn redact_removes_the_key_and_key_like_words_and_truncates() {
        let k = ApiKey::new("secret-value-123");
        let msg = redact(
            "bad key secret-value-123, also sk-proj-ABCDEFGH1234 was seen.",
            &k,
        );
        assert!(
            !msg.contains("secret-value") && !msg.contains("ABCDEFGH"),
            "{msg}"
        );
        assert_eq!(redact(&"x".repeat(500), &k).chars().count(), 200);
    }
}
