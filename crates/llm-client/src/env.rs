//! The read-only `env` profile (context_pack.md section 10, "Keys", source one):
//! `TUTOR_LLM_PROTOCOL`, `TUTOR_LLM_BASE_URL`, `TUTOR_LLM_MODEL` and
//! `TUTOR_LLM_API_KEY`, from the process environment or a `.env` file. The
//! process environment wins. Errors name variables, never values.

use crate::{ApiKey, LlmError, Protocol, ProviderConfig, policy};
use std::{collections::HashMap, path::Path};

const PROTOCOL: &str = "TUTOR_LLM_PROTOCOL";
const BASE_URL: &str = "TUTOR_LLM_BASE_URL";
const MODEL: &str = "TUTOR_LLM_MODEL";
const API_KEY: &str = "TUTOR_LLM_API_KEY";

#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("these variables are missing or empty: {}", .0.join(", "))]
    Missing(Vec<&'static str>),
    #[error("{PROTOCOL} must be `openai_chat` or `anthropic_messages`")]
    UnknownProtocol,
    #[error(transparent)]
    BaseUrl(#[from] LlmError),
}

/// Parses `KEY=VALUE` lines. Blank lines and `#` comments are skipped, a leading
/// `export ` is allowed, and one pair of matching quotes around a value is removed.
pub fn parse_dotenv(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let unquoted = ['"', '\'']
                .iter()
                .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
                .unwrap_or(value);
            Some((key.trim().to_owned(), unquoted.to_owned()))
        })
        .collect()
}

/// Builds the `env` profile. `Ok(None)` when none of the four variables is set,
/// so a missing profile is normal. A partly filled one is an error.
pub fn provider_from_vars(
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<ProviderConfig>, EnvError> {
    let get = |name| {
        var(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let (protocol, base_url, model, key) = (get(PROTOCOL), get(BASE_URL), get(MODEL), get(API_KEY));
    if [&protocol, &base_url, &model, &key]
        .iter()
        .all(|v| v.is_none())
    {
        return Ok(None);
    }
    let missing: Vec<&'static str> = [
        (PROTOCOL, &protocol),
        (BASE_URL, &base_url),
        (MODEL, &model),
        (API_KEY, &key),
    ]
    .into_iter()
    .filter(|(_, v)| v.is_none())
    .map(|(n, _)| n)
    .collect();
    let (Some(protocol), Some(base_url), Some(model), Some(key)) = (protocol, base_url, model, key)
    else {
        return Err(EnvError::Missing(missing));
    };
    let protocol = Protocol::from_name(&protocol).ok_or(EnvError::UnknownProtocol)?;
    policy::parse_base_url(&base_url)?;
    Ok(Some(ProviderConfig {
        protocol,
        base_url,
        model,
        api_key: ApiKey::new(key),
    }))
}

/// Reads the process environment, filling gaps from `.env_file` when it exists.
pub fn load_env_profile(env_file: Option<&Path>) -> Result<Option<ProviderConfig>, EnvError> {
    let file_vars = env_file
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_dotenv(&t))
        .unwrap_or_default();
    provider_from_vars(|name| {
        std::env::var(name)
            .ok()
            .or_else(|| file_vars.get(name).cloned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    const FULL: [(&str, &str); 4] = [
        (PROTOCOL, "openai_chat"),
        (BASE_URL, "https://api.example.com/v1"),
        (MODEL, "some-model"),
        (API_KEY, "fake-key-0000"),
    ];

    #[test]
    fn dotenv_handles_comments_export_quotes_and_blank_lines() {
        let m = parse_dotenv("# c\n\nA=1\nexport B = \"two words\"\nC='x'\nnot a pair\nD=a=b\n");
        assert_eq!(m.get("A").map(String::as_str), Some("1"));
        assert_eq!(m.get("B").map(String::as_str), Some("two words"));
        assert_eq!(m.get("C").map(String::as_str), Some("x"));
        assert_eq!(m.get("D").map(String::as_str), Some("a=b"));
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn a_full_set_makes_a_profile_and_none_set_makes_none() {
        let p = provider_from_vars(vars(&FULL)).unwrap().unwrap();
        assert_eq!(
            (p.protocol, p.model.as_str()),
            (Protocol::OpenaiChat, "some-model")
        );
        assert_eq!(p.api_key.last_four(), "0000");
        assert!(
            !format!("{p:?}").contains("fake-key-0000"),
            "Debug must not print the key"
        );
        assert!(provider_from_vars(vars(&[])).unwrap().is_none());
        assert!(
            provider_from_vars(vars(&[(API_KEY, "  ")]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_partial_set_names_the_missing_variables_and_never_a_value() {
        let err = provider_from_vars(vars(&[
            (PROTOCOL, "openai_chat"),
            (API_KEY, "fake-key-0000"),
        ]))
        .unwrap_err();
        let shown = format!("{err} {err:?}");
        assert!(shown.contains(BASE_URL) && shown.contains(MODEL));
        assert!(!shown.contains("fake-key-0000"));
    }

    #[test]
    fn an_unknown_protocol_and_an_insecure_url_are_refused() {
        let mut v = FULL;
        v[0].1 = "gemini";
        assert!(matches!(
            provider_from_vars(vars(&v)),
            Err(EnvError::UnknownProtocol)
        ));
        let mut v = FULL;
        v[1].1 = "http://api.example.com";
        assert!(matches!(
            provider_from_vars(vars(&v)),
            Err(EnvError::BaseUrl(_))
        ));
    }

    #[test]
    fn the_file_fills_gaps_and_the_process_environment_wins() {
        let dir = std::env::temp_dir().join(format!("llm-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".env");
        std::fs::write(&file, "TUTOR_LLM_PROTOCOL=anthropic_messages\nTUTOR_LLM_BASE_URL=https://a.example.com\nTUTOR_LLM_MODEL=m\nTUTOR_LLM_API_KEY=from-file-0001\n").unwrap();
        // The test process sets none of these, so the file supplies all four.
        let p = load_env_profile(Some(&file)).unwrap().unwrap();
        assert_eq!(p.protocol, Protocol::AnthropicMessages);
        assert_eq!(p.api_key.last_four(), "0001");
        assert!(
            load_env_profile(Some(&dir.join("missing.env")))
                .unwrap()
                .is_none()
        );
    }
}
