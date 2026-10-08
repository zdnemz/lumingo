use std::collections::HashMap;

use super::*;

const KEY: &str = "sk-live-0123456789abcdefghij-KEYMATERIAL";

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

fn full_env() -> Vec<(&'static str, &'static str)> {
    vec![
        (ENV_PROTOCOL, "openai_chat"),
        (ENV_BASE_URL, "https://api.openai.com/v1"),
        (ENV_MODEL, "some-model"),
        (ENV_API_KEY, KEY),
    ]
}

#[test]
fn the_four_variables_form_a_read_only_profile_named_env() {
    let profile = EnvProfileLoader::default()
        .load_with(env_of(&full_env()))
        .expect("loads")
        .expect("present");

    assert_eq!(profile.name, "env");
    assert_eq!(profile.source, ProfileSource::Env);
    assert_eq!(profile.protocol, Protocol::OpenAiChat);
    assert_eq!(profile.base_url.as_str(), "https://api.openai.com/v1");
    assert_eq!(profile.model, "some-model");
    assert!(profile.key.is_some());
}

#[test]
fn no_variables_means_no_profile() {
    assert!(
        EnvProfileLoader::default()
            .load_with(env_of(&[]))
            .expect("loads")
            .is_none()
    );
    // The shipped `.env.example` has all four names with empty values.
    let empty = [
        (ENV_PROTOCOL, ""),
        (ENV_BASE_URL, ""),
        (ENV_MODEL, ""),
        (ENV_API_KEY, "  "),
    ];
    assert!(
        EnvProfileLoader::default()
            .load_with(env_of(&empty))
            .expect("loads")
            .is_none()
    );
}

#[test]
fn a_partly_set_environment_names_the_missing_variables_and_nothing_else() {
    let error = EnvProfileLoader::default()
        .load_with(env_of(&[
            (ENV_API_KEY, KEY),
            (ENV_PROTOCOL, "anthropic_messages"),
        ]))
        .expect_err("incomplete");

    assert_eq!(
        error,
        ProfileError::MissingVariables {
            missing: vec![ENV_BASE_URL, ENV_MODEL]
        }
    );
    assert!(!format!("{error} {error:?}").contains("KEYMATERIAL"));
}

#[test]
fn the_key_is_optional_for_a_local_server() {
    let env = [
        (ENV_PROTOCOL, "openai_chat"),
        (ENV_BASE_URL, "http://localhost:8080/v1"),
        (ENV_MODEL, "local"),
    ];
    let profile = EnvProfileLoader::default()
        .load_with(env_of(&env))
        .expect("loads")
        .expect("present");
    assert!(profile.key.is_none());
    assert!(!profile.info().has_key);
}

#[test]
fn a_dotenv_file_fills_what_the_process_environment_does_not_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(".env");
    std::fs::write(
        &path,
        format!(
            "# provider\nTUTOR_LLM_PROTOCOL=anthropic_messages\nexport TUTOR_LLM_BASE_URL=\"https://api.anthropic.com\"\nTUTOR_LLM_MODEL='m-1' # note\nTUTOR_LLM_API_KEY={KEY}\n"
        ),
    )
    .expect("write");
    let loader = EnvProfileLoader::new(vec![path]);

    // The process environment wins for the variables it sets.
    let profile = loader
        .load_with(env_of(&[(ENV_MODEL, "from-process")]))
        .expect("loads")
        .expect("present");

    assert_eq!(profile.protocol, Protocol::AnthropicMessages);
    assert_eq!(profile.base_url.as_str(), "https://api.anthropic.com/");
    assert_eq!(profile.model, "from-process");
    assert_eq!(profile.info().key_last4.as_deref(), Some("RIAL"));
}

#[test]
fn dotenv_parsing_handles_quotes_comments_and_junk() {
    let parsed = parse_dotenv(
        "A=1\n  B = two words \nC=\"quoted # not a comment\"\nD=plain # comment\n=nokey\nnoequals\n#E=3\nexport F=6\nG='single'\n",
    );
    assert_eq!(parsed["A"], "1");
    assert_eq!(parsed["B"], "two words");
    assert_eq!(parsed["C"], "quoted # not a comment");
    assert_eq!(parsed["D"], "plain");
    assert_eq!(parsed["F"], "6");
    assert_eq!(parsed["G"], "single");
    assert_eq!(parsed.len(), 6);
}

#[test]
fn base_urls_are_checked_without_echoing_them() {
    let ok = |url: &str| {
        ProviderProfile::new(
            "p",
            Protocol::OpenAiChat,
            url,
            "m",
            None,
            ProfileSource::File,
        )
    };
    for good in [
        "https://api.example.com/v1",
        "http://localhost:8080/v1",
        "http://127.0.0.1:9/v1",
        "http://[::1]:1/v1",
    ] {
        assert!(ok(good).is_ok(), "{good}");
    }
    for bad in [
        "http://api.example.com/v1",
        "ftp://example.com",
        "not a url",
        "https://user:secret-pass@example.com/v1",
        "https://example.com/v1?key=secret-in-query",
        "https://example.com/v1#frag",
    ] {
        let error = ok(bad).expect_err(bad);
        assert!(matches!(error, ProfileError::InvalidBaseUrl(_)), "{bad}");
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }
}

#[test]
fn names_models_and_keys_are_validated() {
    let make = |name: &str, model: &str, key: Option<&str>| {
        ProviderProfile::new(
            name,
            Protocol::OpenAiChat,
            "https://example.com/v1",
            model,
            key,
            ProfileSource::File,
        )
    };
    assert_eq!(
        make("", "m", None).expect_err("name"),
        ProfileError::InvalidName
    );
    assert_eq!(
        make(&"x".repeat(65), "m", None).expect_err("name"),
        ProfileError::InvalidName
    );
    assert_eq!(
        make("a\nb", "m", None).expect_err("name"),
        ProfileError::InvalidName
    );
    assert_eq!(
        make("p", "  ", None).expect_err("model"),
        ProfileError::EmptyModel
    );
    assert!(matches!(
        make("p", "m", Some("two words")),
        Err(ProfileError::Key(_))
    ));
    assert!(
        make("p", "m", Some("  "))
            .expect("blank key is no key")
            .key
            .is_none()
    );
}

#[test]
fn the_ui_type_has_has_key_and_the_last_four_characters_only() {
    let profile = ProviderProfile::new(
        "work",
        Protocol::AnthropicMessages,
        "https://api.anthropic.com",
        "m",
        Some(KEY),
        ProfileSource::File,
    )
    .expect("valid");

    let info = profile.info();
    let json = serde_json::to_string(&info).expect("json");

    assert!(info.has_key);
    assert_eq!(info.key_last4.as_deref(), Some("RIAL"));
    assert!(
        !json.contains("KEYMATERIAL") && !json.contains("0123456789"),
        "{json}"
    );
    assert!(json.contains("\"source\":\"file\""));
    assert!(json.contains("\"protocol\":\"anthropic_messages\""));
    let debug = format!("{profile:?}");
    assert!(
        !debug.contains("KEYMATERIAL") && !debug.contains("RIAL"),
        "{debug}"
    );
}

#[test]
fn providers_toml_round_trips_and_keeps_the_env_profile_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("providers.toml");
    let env = EnvProfileLoader::default()
        .load_with(env_of(&full_env()))
        .expect("loads");
    let mut set = ProfileSet::load(env, &path).expect("a missing file is an empty set");
    assert_eq!(set.list().len(), 1);

    let saved = ProviderProfile::new(
        "mine",
        Protocol::AnthropicMessages,
        "https://api.anthropic.com",
        "m-2",
        Some(KEY),
        ProfileSource::File,
    )
    .expect("valid");
    set.upsert(saved).expect("upsert");
    set.save(&path).expect("save");

    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("[[profile]]") && text.contains("name = \"mine\""));
    assert!(
        !text.contains("env"),
        "the environment profile is never written: {text}"
    );
    let reloaded = ProfileSet::load(None, &path).expect("reload");
    let profile = reloaded.get("mine").expect("saved profile");
    assert_eq!(profile.model, "m-2");
    assert_eq!(profile.source, ProfileSource::File);
    assert_eq!(profile.info().key_last4.as_deref(), Some("RIAL"));
}

#[cfg(unix)]
#[test]
fn providers_toml_is_created_for_the_user_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("providers.toml");
    let mut set = ProfileSet::default();
    set.upsert(
        ProviderProfile::new(
            "mine",
            Protocol::OpenAiChat,
            "https://example.com/v1",
            "m",
            Some(KEY),
            ProfileSource::File,
        )
        .expect("valid"),
    )
    .expect("upsert");

    set.save(&path).expect("save");
    set.save(&path).expect("save again");

    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!dir.path().join("providers.toml.tmp").exists());
}

#[test]
fn the_env_profile_is_read_only() {
    let env = EnvProfileLoader::default()
        .load_with(env_of(&full_env()))
        .expect("loads");
    let mut set =
        ProfileSet::load(env, std::path::Path::new("/nonexistent/providers.toml")).expect("load");
    let fake = ProviderProfile::new(
        "env",
        Protocol::OpenAiChat,
        "https://example.com/v1",
        "m",
        None,
        ProfileSource::File,
    )
    .expect("valid");

    assert_eq!(
        set.upsert(fake).expect_err("reserved"),
        ProfileError::ReservedName
    );
    assert_eq!(
        set.remove("env").expect_err("reserved"),
        ProfileError::ReservedName
    );
    assert!(!set.remove("nothing").expect("remove"));
}

#[test]
fn a_providers_file_cannot_define_env_or_repeat_a_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("providers.toml");
    let entry = |name: &str| {
        format!(
            "[[profile]]\nname = \"{name}\"\nprotocol = \"openai_chat\"\nbase_url = \"https://example.com/v1\"\nmodel = \"m\"\n"
        )
    };

    std::fs::write(&path, entry("env")).expect("write");
    assert_eq!(
        ProfileSet::load(None, &path).expect_err("reserved"),
        ProfileError::ReservedName
    );
    std::fs::write(&path, format!("{}{}", entry("a"), entry("a"))).expect("write");
    assert_eq!(
        ProfileSet::load(None, &path).expect_err("duplicate"),
        ProfileError::DuplicateName
    );
}

#[test]
fn a_broken_providers_file_gives_an_error_without_the_text_of_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("providers.toml");
    std::fs::write(
        &path,
        format!("[[profile]]\nname = \"a\"\nprotocol = \"{KEY}\"\nbase_url = \"https://example.com/v1\"\nmodel = \"m\"\napi_key = {KEY}\n"),
    )
    .expect("write");

    let error = ProfileSet::load(None, &path).expect_err("invalid");

    let shown = format!("{error} {error:?}");
    assert!(matches!(error, ProfileError::Toml(_)));
    assert!(!shown.contains("KEYMATERIAL"), "{shown}");
}

#[test]
fn the_protocol_serialises_to_its_wire_names() {
    assert_eq!(
        serde_json::to_string(&Protocol::OpenAiChat).expect("json"),
        "\"openai_chat\""
    );
    assert_eq!(
        serde_json::to_string(&Protocol::AnthropicMessages).expect("json"),
        "\"anthropic_messages\""
    );
    for protocol in [Protocol::OpenAiChat, Protocol::AnthropicMessages] {
        assert_eq!(protocol.as_str().parse::<Protocol>(), Ok(protocol));
        let json = serde_json::to_string(&protocol).expect("json");
        assert_eq!(
            serde_json::from_str::<Protocol>(&json).expect("round trip"),
            protocol
        );
    }
    assert!("open_ai_chat".parse::<Protocol>().is_err());
}
