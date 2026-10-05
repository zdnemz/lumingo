//! The record of which models are installed: `installed.json` in the models
//! folder. It is written whole and renamed into place, so a crash leaves
//! either the old record or the new one.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::ModelError;
use crate::hash::to_hex;

pub const RECORD_FILE: &str = "installed.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFile {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub id: String,
    pub version: String,
    pub license: String,
    pub license_url: String,
    pub source: String,
    /// Seconds since the Unix epoch when the install finished.
    pub installed_at_unix: u64,
    pub files: Vec<InstalledFile>,
}

impl InstalledRecord {
    /// One SHA-256 over every file's path and checksum, in path order. It is
    /// what `EngineInfo::model_checksum` carries, so a score can be traced to
    /// the exact files that produced it.
    pub fn combined_sha256(&self) -> String {
        let mut files: Vec<&InstalledFile> = self.files.iter().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut hasher = Sha256::new();
        for file in files {
            hasher.update(file.path.as_bytes());
            hasher.update([0]);
            hasher.update(file.sha256.as_bytes());
            hasher.update(b"\n");
        }
        to_hex(&hasher.finalize())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Document {
    models: BTreeMap<String, InstalledRecord>,
}

#[derive(Debug)]
pub(crate) struct InstalledStore {
    path: PathBuf,
    doc: Document,
}

impl InstalledStore {
    pub(crate) fn load(root: &Path) -> Result<Self, ModelError> {
        let path = root.join(RECORD_FILE);
        let doc = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| ModelError::Record {
                path: path.clone(),
                reason: e.to_string(),
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Document::default(),
            Err(e) => return Err(ModelError::io(format!("reading {}", path.display()), e)),
        };
        Ok(Self { path, doc })
    }

    pub(crate) fn get(&self, id: &str) -> Option<&InstalledRecord> {
        self.doc.models.get(id)
    }

    pub(crate) fn all(&self) -> impl Iterator<Item = &InstalledRecord> {
        self.doc.models.values()
    }

    pub(crate) fn put(&mut self, record: InstalledRecord) -> Result<(), ModelError> {
        self.doc.models.insert(record.id.clone(), record);
        self.save()
    }

    pub(crate) fn remove(&mut self, id: &str) -> Result<bool, ModelError> {
        let existed = self.doc.models.remove(id).is_some();
        if existed {
            self.save()?;
        }
        Ok(existed)
    }

    fn save(&self) -> Result<(), ModelError> {
        let json = serde_json::to_string_pretty(&self.doc).map_err(|e| ModelError::Record {
            path: self.path.clone(),
            reason: e.to_string(),
        })?;
        let tmp = self.path.with_extension("json.tmp");
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| ModelError::io(format!("creating {}", parent.display()), e))?;
        }
        fs::write(&tmp, json)
            .map_err(|e| ModelError::io(format!("writing {}", tmp.display()), e))?;
        fs::rename(&tmp, &self.path)
            .map_err(|e| ModelError::io(format!("replacing {}", self.path.display()), e))
    }
}
