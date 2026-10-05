//! Removes anything that looks like a credential from provider error text before
//! it is stored in an error value or logged. Providers sometimes echo part of the
//! key ("Incorrect API key provided: sk-abc***xyz"), so the exact key is removed
//! first and a shape-based pass follows.

const MAX_MESSAGE_CHARS: usize = 300;
const REDACTED: &str = "[redacted]";

/// Known key prefixes. A token that starts with one of these and is at least
/// eight characters long is treated as a key.
const KEY_PREFIXES: [&str; 11] = [
    "sk-", "sk_", "pk-", "rk-", "AIza", "gsk_", "xai-", "ghp_", "hf_", "AKIA", "key-",
];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '*' | '+' | '/' | '=')
}

fn looks_like_secret(token: &str) -> bool {
    if token.contains("**") {
        return true;
    }
    if token.len() >= 8 && KEY_PREFIXES.iter().any(|p| token.starts_with(p)) {
        return true;
    }
    // Long mixed tokens: base64 or hex blobs. Plain words and identifiers such as
    // `max_completion_tokens` have no digit and pass through.
    token.len() >= 20
        && token.chars().any(|c| c.is_ascii_digit())
        && token.chars().any(|c| c.is_ascii_alphabetic())
}

/// Returns a one-line, length-limited copy of `raw` with the key and key-like
/// tokens replaced by `[redacted]`.
pub fn sanitize_message(raw: &str, key: Option<&str>) -> String {
    let mut text = raw.to_owned();
    if let Some(key) = key.filter(|k| k.len() >= 4) {
        text = text.replace(key, REDACTED);
    }

    let mut out = String::with_capacity(text.len().min(MAX_MESSAGE_CHARS * 2));
    let mut token = String::new();
    let mut after_bearer = false;

    let flush = |token: &mut String, out: &mut String, after_bearer: &mut bool| {
        if token.is_empty() {
            return;
        }
        if *after_bearer || looks_like_secret(token) {
            out.push_str(REDACTED);
        } else {
            out.push_str(token);
        }
        *after_bearer = token.eq_ignore_ascii_case("bearer");
        token.clear();
    };

    for c in text.chars() {
        if is_token_char(c) {
            token.push(c);
        } else {
            flush(&mut token, &mut out, &mut after_bearer);
            if c.is_whitespace() {
                // Collapse runs of whitespace and newlines so one log line stays one line.
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            } else {
                out.push(c);
                after_bearer = false;
            }
        }
    }
    flush(&mut token, &mut out, &mut after_bearer);

    let trimmed = out.trim();
    if trimmed.chars().count() > MAX_MESSAGE_CHARS {
        let cut: String = trimmed.chars().take(MAX_MESSAGE_CHARS).collect();
        format!("{cut}...")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_the_exact_key() {
        let out = sanitize_message("bad key abcd-efgh-1234 given", Some("abcd-efgh-1234"));
        assert_eq!(out, "bad key [redacted] given");
    }

    #[test]
    fn removes_echoed_partial_keys() {
        let out = sanitize_message(
            "Incorrect API key provided: sk-proj-****abcd. Find yours at https://example.com/keys",
            None,
        );
        assert!(!out.contains("abcd"), "{out}");
        assert!(out.contains("Incorrect API key provided"));
    }

    #[test]
    fn removes_bearer_tokens_and_long_blobs() {
        let out = sanitize_message("Authorization: Bearer abc.def", None);
        assert!(!out.contains("abc.def"), "{out}");
        let out = sanitize_message("token QWxhZGRpbjpvcGVuIHNlc2FtZTEyMw is invalid", None);
        assert!(!out.contains("QWxhZGRpbjpvcGVuIHNlc2FtZTEyMw"), "{out}");
    }

    #[test]
    fn keeps_parameter_names_that_quirk_detection_needs() {
        let out = sanitize_message(
            "Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.",
            None,
        );
        assert!(out.contains("max_tokens"));
        assert!(out.contains("max_completion_tokens"));
    }

    #[test]
    fn collapses_whitespace_and_limits_length() {
        let out = sanitize_message("line one\n\n   line two", None);
        assert_eq!(out, "line one line two");
        let long = "word ".repeat(200);
        assert!(sanitize_message(&long, None).chars().count() <= MAX_MESSAGE_CHARS + 3);
    }
}
