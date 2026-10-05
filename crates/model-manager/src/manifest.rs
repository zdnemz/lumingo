//! `models/manifest.toml`: the only source of model download locations.
//!
//! Parsing and downloading are separate questions. A manifest may list a
//! candidate whose checksums the owner has not filled in yet: it parses, it is
//! shown, but [`ModelEntry::check_downloadable`] refuses it, so nothing is ever
//! fetched without a checksum to verify against.

use std::collections::HashSet;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::{Host, Url};

/// The only schema version this code understands.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("the manifest is not valid TOML for this schema: {0}")]
    Parse(String),
    #[error("unsupported manifest schema_version {found}, this build reads {SCHEMA_VERSION}")]
    SchemaVersion { found: u32 },
    #[error("a model id must be 1 to 64 characters of a-z, 0-9, '-' or '_': {0:?}")]
    BadId(String),
    #[error("model id {0:?} appears twice")]
    DuplicateId(String),
    #[error(
        "model {id}: file path {path:?} must be relative, use '/', and not leave the model folder"
    )]
    BadPath { id: String, path: String },
    #[error("model {id}: file path {path:?} appears twice")]
    DuplicatePath { id: String, path: String },
}

/// Why an entry cannot be downloaded. The checksum rule is the one the
/// roadmap names; the others stop an entry that would fail half way.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NotDownloadable {
    #[error("model {id} lists no files")]
    NoFiles { id: String },
    #[error("model {id}: file {path} has an empty checksum, so it cannot be verified")]
    EmptyChecksum { id: String, path: String },
    #[error("model {id}: file {path} has a checksum that is not 64 hexadecimal characters")]
    BadChecksum { id: String, path: String },
    #[error("model {id} has no source and file {path} has no url")]
    NoLocation { id: String, path: String },
    #[error("model {id} has no license or no license_url")]
    NoLicence { id: String },
    #[error("model {id}: {url} is refused: {reason}")]
    BadUrl {
        id: String,
        url: String,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Vad,
    Stt,
    Tts,
    Pron,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    /// Relative path inside the model folder, with `/` separators.
    pub path: String,
    /// Lowercase or uppercase hexadecimal SHA-256. Empty means "not verified
    /// yet" and makes the entry undownloadable.
    #[serde(default)]
    pub sha256: String,
    /// Full URL of this file. When absent it is `source` plus `path`.
    #[serde(default)]
    pub url: Option<String>,
    /// Expected size. A download that grows beyond it is stopped.
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    pub id: String,
    pub role: ModelRole,
    pub engine: String,
    pub version: String,
    pub license: String,
    pub license_url: String,
    /// Upstream location, never a project-owned mirror. A directory URL: each
    /// file without its own `url` is fetched from `source` plus `path`.
    pub source: String,
    /// Total size, 0 until it is read from a real download.
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub files: Vec<ManifestFile>,
    /// Licence text or the use restrictions in full, shown before download.
    /// When absent the notice shows only the licence name and its URL.
    #[serde(default)]
    pub license_text: Option<String>,
    /// True when the licence carries use restrictions the owner must read in
    /// full (for example OpenRAIL-M). The notice says so.
    #[serde(default)]
    pub license_review: bool,
    /// Free text for the owner: what is still unverified about this entry.
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default, rename = "model")]
    pub models: Vec<ModelEntry>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Manifest {
    /// Parses and checks the structure. Empty checksums are allowed here.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Self =
            toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn entry(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ManifestError::SchemaVersion {
                found: self.schema_version,
            });
        }
        let mut ids = HashSet::new();
        for model in &self.models {
            let id_ok = !model.id.is_empty()
                && model.id.len() <= 64
                && model
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
            if !id_ok {
                return Err(ManifestError::BadId(model.id.clone()));
            }
            if !ids.insert(model.id.as_str()) {
                return Err(ManifestError::DuplicateId(model.id.clone()));
            }
            let mut paths = HashSet::new();
            for file in &model.files {
                if !is_safe_relative_path(&file.path) {
                    return Err(ManifestError::BadPath {
                        id: model.id.clone(),
                        path: file.path.clone(),
                    });
                }
                if !paths.insert(file.path.as_str()) {
                    return Err(ManifestError::DuplicatePath {
                        id: model.id.clone(),
                        path: file.path.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// A path that stays inside the model folder on every OS: no root, no drive,
/// no `..`, no backslash, and not a name the downloader uses for scratch files.
fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return false;
    }
    if path.ends_with(".part") {
        return false;
    }
    path.split('/')
        .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// What the learner reads before a download starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenceNotice {
    pub model_id: String,
    pub license: String,
    pub license_url: String,
    /// The text or use restrictions from the manifest, when it carries them.
    pub text: Option<String>,
    /// The licence has use restrictions that must be read in full.
    pub needs_review: bool,
}

impl LicenceNotice {
    /// Records that this exact notice was shown and accepted. A download only
    /// starts with one, and only for the same model and the same licence.
    pub fn accept(&self) -> LicenceAcceptance {
        LicenceAcceptance {
            model_id: self.model_id.clone(),
            license: self.license.clone(),
            license_url: self.license_url.clone(),
        }
    }
}

/// Proof that a [`LicenceNotice`] was accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenceAcceptance {
    model_id: String,
    license: String,
    license_url: String,
}

impl LicenceAcceptance {
    pub(crate) fn matches(&self, entry: &ModelEntry) -> bool {
        self.model_id == entry.id
            && self.license == entry.license
            && self.license_url == entry.license_url
    }
}

impl ModelEntry {
    pub fn licence_notice(&self) -> LicenceNotice {
        LicenceNotice {
            model_id: self.id.clone(),
            license: self.license.clone(),
            license_url: self.license_url.clone(),
            text: self.license_text.clone(),
            needs_review: self.license_review,
        }
    }

    /// Resolves where `file` is fetched from.
    pub fn file_url(&self, file: &ManifestFile) -> Result<Url, NotDownloadable> {
        let raw = match &file.url {
            Some(url) if !url.trim().is_empty() => url.trim().to_owned(),
            _ => {
                if self.source.trim().is_empty() {
                    return Err(NotDownloadable::NoLocation {
                        id: self.id.clone(),
                        path: file.path.clone(),
                    });
                }
                let mut base = self.source.trim().to_owned();
                if !base.ends_with('/') {
                    base.push('/');
                }
                let base = parse_url(&self.id, &base)?;
                return base
                    .join(&file.path)
                    .map_err(|_| self.bad_url(&file.path, "the path does not join the source"))
                    .and_then(|url| check_url(&self.id, url));
            }
        };
        check_url(&self.id, parse_url(&self.id, &raw)?)
    }

    /// Says whether this entry may be downloaded. Nothing touches the network
    /// before this passes.
    pub fn check_downloadable(&self) -> Result<(), NotDownloadable> {
        if self.license.trim().is_empty() || self.license_url.trim().is_empty() {
            return Err(NotDownloadable::NoLicence {
                id: self.id.clone(),
            });
        }
        if self.files.is_empty() {
            return Err(NotDownloadable::NoFiles {
                id: self.id.clone(),
            });
        }
        for file in &self.files {
            if file.sha256.trim().is_empty() {
                return Err(NotDownloadable::EmptyChecksum {
                    id: self.id.clone(),
                    path: file.path.clone(),
                });
            }
            if normalise_sha256(&file.sha256).is_none() {
                return Err(NotDownloadable::BadChecksum {
                    id: self.id.clone(),
                    path: file.path.clone(),
                });
            }
            self.file_url(file)?;
        }
        Ok(())
    }

    fn bad_url(&self, url: &str, reason: &'static str) -> NotDownloadable {
        NotDownloadable::BadUrl {
            id: self.id.clone(),
            url: url.to_owned(),
            reason,
        }
    }
}

fn parse_url(id: &str, raw: &str) -> Result<Url, NotDownloadable> {
    Url::parse(raw).map_err(|_| NotDownloadable::BadUrl {
        id: id.to_owned(),
        url: raw.to_owned(),
        reason: "not a valid URL",
    })
}

fn check_url(id: &str, url: Url) -> Result<Url, NotDownloadable> {
    match url_problem(&url) {
        None => Ok(url),
        Some(reason) => Err(NotDownloadable::BadUrl {
            id: id.to_owned(),
            url: url.to_string(),
            reason,
        }),
    }
}

/// The transport rule for every request and every redirect: HTTPS, except that
/// the loopback address may use plain HTTP (local servers in tests), and no
/// credentials inside the URL.
pub(crate) fn url_problem(url: &Url) -> Option<&'static str> {
    if !url.username().is_empty() || url.password().is_some() {
        return Some("a URL with credentials is not allowed");
    }
    match url.scheme() {
        "https" => None,
        "http" if is_loopback(url) => None,
        "http" => Some("plain HTTP is only allowed for the loopback address"),
        _ => Some("only https URLs are allowed"),
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

/// Lowercase hexadecimal of a 64-character SHA-256, or `None`.
pub(crate) fn normalise_sha256(text: &str) -> Option<String> {
    let text = text.trim();
    (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn entry_toml(files: &str) -> String {
        format!(
            r#"
[[model]]
id = "stt-test"
role = "stt"
engine = "sherpa-onnx"
version = "1"
license = "MIT"
license_url = "https://example.org/licence"
source = "https://example.org/models/stt-test"
size_bytes = 0
files = [{files}]
"#
        )
    }

    fn parse_entry(files: &str) -> ModelEntry {
        Manifest::parse(&entry_toml(files))
            .expect("parses")
            .models
            .remove(0)
    }

    #[test]
    fn the_documented_example_parses_with_an_empty_checksum() {
        let entry = parse_entry(r#"{ path = "encoder.onnx", sha256 = "" }"#);
        assert_eq!(entry.role, ModelRole::Stt);
        assert_eq!(entry.files[0].path, "encoder.onnx");
    }

    #[test]
    fn an_empty_checksum_is_refused_for_download() {
        let entry = parse_entry(r#"{ path = "encoder.onnx", sha256 = "" }"#);
        assert_eq!(
            entry.check_downloadable(),
            Err(NotDownloadable::EmptyChecksum {
                id: "stt-test".into(),
                path: "encoder.onnx".into()
            })
        );
        // A checksum that is only whitespace counts as empty too.
        let entry = parse_entry(r#"{ path = "a", sha256 = "   " }"#);
        assert!(matches!(
            entry.check_downloadable(),
            Err(NotDownloadable::EmptyChecksum { .. })
        ));
        // A missing checksum is the same as an empty one.
        let entry = parse_entry(r#"{ path = "a" }"#);
        assert!(matches!(
            entry.check_downloadable(),
            Err(NotDownloadable::EmptyChecksum { .. })
        ));
    }

    #[test]
    fn one_empty_checksum_among_filled_ones_refuses_the_whole_entry() {
        let files =
            format!(r#"{{ path = "a", sha256 = "{HASH_A}" }}, {{ path = "b", sha256 = "" }}"#);
        let entry = parse_entry(&files);
        assert!(matches!(
            entry.check_downloadable(),
            Err(NotDownloadable::EmptyChecksum { path, .. }) if path == "b"
        ));
    }

    #[test]
    fn a_malformed_checksum_is_refused() {
        for bad in ["abc", &"g".repeat(64), &"a".repeat(65)] {
            let entry = parse_entry(&format!(r#"{{ path = "a", sha256 = "{bad}" }}"#));
            assert!(
                matches!(
                    entry.check_downloadable(),
                    Err(NotDownloadable::BadChecksum { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_filled_entry_is_downloadable_and_urls_join_source_and_path() {
        let files = format!(r#"{{ path = "sub/encoder.onnx", sha256 = "{HASH_A}" }}"#);
        let entry = parse_entry(&files);
        assert_eq!(entry.check_downloadable(), Ok(()));
        let url = entry.file_url(&entry.files[0]).expect("url");
        assert_eq!(
            url.as_str(),
            "https://example.org/models/stt-test/sub/encoder.onnx"
        );
    }

    #[test]
    fn a_file_url_overrides_the_source() {
        let files = format!(
            r#"{{ path = "m.onnx", sha256 = "{HASH_A}", url = "https://cdn.example.org/m.onnx" }}"#
        );
        let entry = parse_entry(&files);
        assert_eq!(
            entry.file_url(&entry.files[0]).expect("url").as_str(),
            "https://cdn.example.org/m.onnx"
        );
    }

    #[test]
    fn an_entry_without_files_is_refused() {
        let entry = parse_entry("");
        assert_eq!(
            entry.check_downloadable(),
            Err(NotDownloadable::NoFiles {
                id: "stt-test".into()
            })
        );
    }

    #[test]
    fn plain_http_credentials_and_odd_schemes_are_refused() {
        let cases = [
            ("http://example.org/m.onnx", true),
            ("ftp://example.org/m.onnx", true),
            ("https://user:pw@example.org/m.onnx", true),
            ("file:///etc/passwd", true),
            ("http://127.0.0.1:8080/m.onnx", false),
            ("http://localhost/m.onnx", false),
            ("http://[::1]:9/m.onnx", false),
            ("https://example.org/m.onnx", false),
            ("http://127.0.0.1.example.org/m.onnx", true),
        ];
        for (url, refused) in cases {
            let files = format!(r#"{{ path = "m", sha256 = "{HASH_A}", url = "{url}" }}"#);
            let entry = parse_entry(&files);
            assert_eq!(entry.check_downloadable().is_err(), refused, "{url}");
        }
    }

    #[test]
    fn a_missing_licence_is_refused() {
        let mut entry = parse_entry(&format!(r#"{{ path = "a", sha256 = "{HASH_A}" }}"#));
        entry.license_url.clear();
        assert!(matches!(
            entry.check_downloadable(),
            Err(NotDownloadable::NoLicence { .. })
        ));
    }

    #[test]
    fn structure_errors_are_caught_at_parse_time() {
        let dup = format!("{}{}", entry_toml(""), entry_toml(""));
        assert!(matches!(
            Manifest::parse(&dup),
            Err(ManifestError::DuplicateId(_))
        ));
        for bad_path in [
            "../x", "/abs", "a\\\\b", "C:/x", "a//b", "x.part", "", "./a",
        ] {
            let text = entry_toml(&format!(r#"{{ path = "{bad_path}", sha256 = "" }}"#));
            assert!(
                matches!(Manifest::parse(&text), Err(ManifestError::BadPath { .. })),
                "{bad_path:?}"
            );
        }
        let dup_path = entry_toml(r#"{ path = "a" }, { path = "a" }"#);
        assert!(matches!(
            Manifest::parse(&dup_path),
            Err(ManifestError::DuplicatePath { .. })
        ));
        let bad_id = entry_toml("").replace("stt-test", "Bad Id");
        assert!(matches!(
            Manifest::parse(&bad_id),
            Err(ManifestError::BadId(_))
        ));
        let future = format!("schema_version = 2\n{}", entry_toml(""));
        assert!(matches!(
            Manifest::parse(&future),
            Err(ManifestError::SchemaVersion { found: 2 })
        ));
        let unknown = entry_toml("").replace("size_bytes = 0", "size_bytes = 0\nmirror = \"x\"");
        assert!(matches!(
            Manifest::parse(&unknown),
            Err(ManifestError::Parse(_))
        ));
        assert!(matches!(
            Manifest::parse("this is not toml ["),
            Err(ManifestError::Parse(_))
        ));
        let bad_role = entry_toml("").replace(r#"role = "stt""#, r#"role = "llm""#);
        assert!(matches!(
            Manifest::parse(&bad_role),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn the_notice_carries_licence_text_and_the_review_flag() {
        let mut entry = parse_entry(&format!(r#"{{ path = "a", sha256 = "{HASH_A}" }}"#));
        entry.license_text = Some("Use restrictions: ...".into());
        entry.license_review = true;
        let notice = entry.licence_notice();
        assert_eq!(notice.license, "MIT");
        assert_eq!(notice.text.as_deref(), Some("Use restrictions: ..."));
        assert!(notice.needs_review);
        assert!(notice.accept().matches(&entry));
        // The same acceptance does not cover a changed licence.
        entry.license = "Other".into();
        assert!(!notice.accept().matches(&entry));
    }
}
