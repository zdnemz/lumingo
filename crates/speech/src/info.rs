use std::fmt;

use serde::{Deserialize, Serialize};

/// Which engine produced a result. It is written into benchmark results and
/// into every assessment evidence row, so any score can be traced to the exact
/// engine and model that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub id: String,
    pub version: String,
    /// SHA-256 of the model files, when the engine loads a model.
    pub model_checksum: Option<String>,
}

impl EngineInfo {
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            model_checksum: None,
        }
    }

    #[must_use]
    pub fn with_model_checksum(mut self, checksum: impl Into<String>) -> Self {
        self.model_checksum = Some(checksum.into());
        self
    }
}

impl fmt::Display for EngineInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.id, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::EngineInfo;

    #[test]
    fn displays_id_and_version() {
        assert_eq!(
            EngineInfo::new("whisper", "base.en").to_string(),
            "whisper base.en"
        );
    }

    #[test]
    fn carries_a_model_checksum_when_given() {
        let info = EngineInfo::new("x", "1").with_model_checksum("abc");
        assert_eq!(info.model_checksum.as_deref(), Some("abc"));
        assert_eq!(EngineInfo::new("x", "1").model_checksum, None);
    }
}
