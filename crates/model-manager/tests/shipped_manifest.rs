#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! The manifest and schema that ship in `models/`.

use std::path::PathBuf;

use model_manager::{Manifest, ModelRole};

fn models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models")
}

fn manifest_text() -> String {
    std::fs::read_to_string(models_dir().join("manifest.toml")).expect("manifest.toml")
}

fn schema() -> serde_json::Value {
    let text = std::fs::read_to_string(models_dir().join("manifest.schema.json"))
        .expect("manifest.schema.json");
    serde_json::from_str(&text).expect("the schema is JSON")
}

#[test]
fn the_shipped_manifest_parses_and_covers_every_role() {
    let manifest = Manifest::parse(&manifest_text()).expect("parses");
    assert!(!manifest.models.is_empty());
    for role in [
        ModelRole::Vad,
        ModelRole::Stt,
        ModelRole::Tts,
        ModelRole::Pron,
    ] {
        assert!(
            manifest.models.iter().any(|m| m.role == role),
            "no candidate for {role:?}"
        );
    }
}

#[test]
fn an_entry_is_downloadable_only_when_every_checksum_is_filled() {
    // True now (all candidates) and after the owner fills the checksums: an
    // entry with a filled checksum for every file must not fail for another
    // reason, and an entry with an empty one must be refused.
    let manifest = Manifest::parse(&manifest_text()).expect("parses");
    for entry in &manifest.models {
        let verdict = entry.check_downloadable();
        let all_filled =
            !entry.files.is_empty() && entry.files.iter().all(|f| !f.sha256.trim().is_empty());
        if all_filled {
            assert_eq!(verdict, Ok(()), "{} has checksums but is refused", entry.id);
        } else {
            assert!(verdict.is_err(), "{} would download unverified", entry.id);
        }
    }
}

#[test]
fn the_shipped_manifest_conforms_to_the_schema() {
    let value: serde_json::Value = toml::from_str(&manifest_text()).expect("toml to json");
    let validator = jsonschema::validator_for(&schema()).expect("schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn the_schema_rejects_what_the_parser_rejects() {
    let validator = jsonschema::validator_for(&schema()).expect("schema compiles");
    let valid = |text: &str| {
        let value: serde_json::Value = toml::from_str(text).expect("toml");
        validator.is_valid(&value)
    };
    let base = r#"
[[model]]
id = "x"
role = "stt"
engine = "e"
version = "1"
license = "MIT"
license_url = "https://example.org/l"
source = "https://example.org/s"
"#;
    assert!(valid(base));
    assert!(!valid(&base.replace("role = \"stt\"", "role = \"llm\"")));
    assert!(!valid(&format!("{base}mirror = \"x\"\n")));
    assert!(!valid(&base.replace("id = \"x\"", "id = \"Bad Id\"")));
    assert!(!valid(&format!(
        "{base}files = [{{ path = \"a\", sha256 = \"abc\" }}]\n"
    )));
    assert!(!valid(&format!(
        "{base}files = [{{ path = \"/abs\", sha256 = \"\" }}]\n"
    )));
    assert!(valid(&format!(
        "{base}files = [{{ path = \"a/b.onnx\", sha256 = \"\" }}]\n"
    )));
}
