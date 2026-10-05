use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use speech::CancelFlag;

use crate::download::{DownloadJob, DownloadOptions, build_client, download_verified};
use crate::error::ModelError;
use crate::hash::sha256_file;
use crate::manifest::{
    LicenceAcceptance, LicenceNotice, Manifest, ModelEntry, NotDownloadable, normalise_sha256,
};
use crate::store::{InstalledFile, InstalledRecord, InstalledStore};

/// Where an install stands, for the `DownloadProgress` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    pub model_id: String,
    pub file: String,
    /// Position of this file among the model's files, from 0.
    pub file_index: usize,
    pub file_count: usize,
    /// Bytes of this file on disk so far, resumed bytes included.
    pub bytes_done: u64,
    /// Size of this file when the server or the manifest declares it.
    pub bytes_total: Option<u64>,
}

/// Reads the manifest, downloads models and keeps the installed-model record.
///
/// Every method that can take time blocks and takes a [`CancelFlag`]. Call
/// them from a dedicated thread, never from a Tokio worker thread.
#[derive(Debug)]
pub struct ModelManager {
    root: PathBuf,
    manifest: Manifest,
    store: InstalledStore,
    options: DownloadOptions,
}

impl ModelManager {
    /// `root` is the folder models are installed under, one subfolder per id,
    /// with `installed.json` beside them. It is created on the first install.
    pub fn open(manifest: Manifest, root: impl Into<PathBuf>) -> Result<Self, ModelError> {
        let root = root.into();
        let store = InstalledStore::load(&root)?;
        Ok(Self {
            root,
            manifest,
            store,
            options: DownloadOptions::default(),
        })
    }

    /// Reads `models/manifest.toml` (or any manifest path) and opens it.
    pub fn open_manifest_file(
        manifest_path: &Path,
        root: impl Into<PathBuf>,
    ) -> Result<Self, ModelError> {
        let text = fs::read_to_string(manifest_path)
            .map_err(|e| ModelError::io(format!("reading {}", manifest_path.display()), e))?;
        Self::open(Manifest::parse(&text)?, root)
    }

    /// Sets how long a transfer may deliver nothing before it fails with
    /// `Stalled`. The default is 30 seconds.
    #[must_use]
    pub fn with_stall_limit(mut self, limit: Duration) -> Self {
        self.options.stall_limit = limit;
        self
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn entry(&self, id: &str) -> Result<&ModelEntry, ModelError> {
        self.manifest
            .entry(id)
            .ok_or_else(|| ModelError::UnknownModel(id.to_owned()))
    }

    /// The licence the learner reads before a download. It involves no network.
    pub fn licence_notice(&self, id: &str) -> Result<LicenceNotice, ModelError> {
        Ok(self.entry(id)?.licence_notice())
    }

    /// Entries that cannot be downloaded and why. An empty checksum is the
    /// expected reason for a candidate the owner has not verified yet.
    pub fn undownloadable(&self) -> Vec<(&ModelEntry, NotDownloadable)> {
        self.manifest
            .models
            .iter()
            .filter_map(|entry| entry.check_downloadable().err().map(|why| (entry, why)))
            .collect()
    }

    pub fn installed(&self) -> impl Iterator<Item = &InstalledRecord> {
        self.store.all()
    }

    pub fn installed_record(&self, id: &str) -> Option<&InstalledRecord> {
        self.store.get(id)
    }

    /// Folder holding the files of one model.
    pub fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// Absolute path of one installed file, or `None` when the model is not
    /// installed or does not list that file.
    pub fn installed_file_path(&self, id: &str, file: &str) -> Option<PathBuf> {
        let record = self.store.get(id)?;
        record
            .files
            .iter()
            .any(|f| f.path == file)
            .then(|| file_path(&self.model_dir(id), file))
    }

    /// Downloads every file of a model, verifies each against the manifest and
    /// records the model as installed.
    ///
    /// Nothing is fetched unless the entry passes
    /// [`ModelEntry::check_downloadable`] and `acceptance` covers this model and
    /// its current licence. Files that are already installed and match their
    /// checksum are not fetched again. Calling it again after an interruption
    /// resumes the file that was in flight.
    pub fn install(
        &mut self,
        id: &str,
        acceptance: &LicenceAcceptance,
        cancel: &CancelFlag,
        progress: &mut dyn FnMut(&DownloadProgress),
    ) -> Result<&InstalledRecord, ModelError> {
        let entry = self.entry(id)?.clone();
        entry.check_downloadable()?;
        if !acceptance.matches(&entry) {
            return Err(ModelError::LicenceNotAccepted {
                model_id: entry.id.clone(),
            });
        }
        let client = build_client()?;
        let dir = self.model_dir(id);
        let mut installed_files = Vec::with_capacity(entry.files.len());
        for (index, file) in entry.files.iter().enumerate() {
            let expected =
                normalise_sha256(&file.sha256).ok_or_else(|| NotDownloadable::BadChecksum {
                    id: entry.id.clone(),
                    path: file.path.clone(),
                })?;
            let dest = file_path(&dir, &file.path);
            let mut report = |bytes_done: u64, bytes_total: Option<u64>| {
                progress(&DownloadProgress {
                    model_id: entry.id.clone(),
                    file: file.path.clone(),
                    file_index: index,
                    file_count: entry.files.len(),
                    bytes_done,
                    bytes_total: bytes_total.or(file.size_bytes),
                });
            };
            if is_installed_and_matches(&dest, &expected, cancel)? {
                let size = file_len(&dest)?;
                report(size, Some(size));
            } else {
                let url = entry.file_url(file)?;
                download_verified(
                    &DownloadJob {
                        client: &client,
                        url: &url,
                        dest: &dest,
                        expected_sha256: &expected,
                        size_limit: file.size_bytes,
                        options: self.options,
                        cancel,
                    },
                    &mut report,
                )?;
            }
            installed_files.push(InstalledFile {
                path: file.path.clone(),
                sha256: expected,
                size_bytes: file_len(&dest)?,
            });
        }
        self.remove_files_no_longer_listed(&entry, &dir);
        let record = InstalledRecord {
            id: entry.id.clone(),
            version: entry.version.clone(),
            license: entry.license.clone(),
            license_url: entry.license_url.clone(),
            source: entry.source.clone(),
            installed_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            files: installed_files,
        };
        self.store.put(record)?;
        self.store
            .get(id)
            .ok_or_else(|| ModelError::UnknownModel(id.to_owned()))
    }

    /// Hashes every installed file again and compares it with the record, to
    /// catch a file changed or truncated after the install.
    pub fn verify_installed(&self, id: &str, cancel: &CancelFlag) -> Result<(), ModelError> {
        let record = self
            .store
            .get(id)
            .ok_or_else(|| ModelError::UnknownModel(id.to_owned()))?;
        let dir = self.model_dir(id);
        for file in &record.files {
            let path = file_path(&dir, &file.path);
            let actual = sha256_file(&path, cancel)?;
            if actual != file.sha256 {
                return Err(ModelError::ChecksumMismatch {
                    path: path.display().to_string(),
                    expected: file.sha256.clone(),
                    actual,
                });
            }
        }
        Ok(())
    }

    /// Deletes a model's files and its record. Returns whether it was installed.
    pub fn uninstall(&mut self, id: &str) -> Result<bool, ModelError> {
        let dir = self.model_dir(id);
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ModelError::io(format!("removing {}", dir.display()), e)),
        }
        self.store.remove(id)
    }

    /// An update may drop a file. The old one would only waste disk.
    fn remove_files_no_longer_listed(&self, entry: &ModelEntry, dir: &Path) {
        let Some(old) = self.store.get(&entry.id) else {
            return;
        };
        for file in &old.files {
            if !entry.files.iter().any(|f| f.path == file.path) {
                let path = file_path(dir, &file.path);
                if let Err(error) = fs::remove_file(&path) {
                    tracing::warn!(%error, "an old model file could not be removed");
                }
            }
        }
    }
}

fn file_path(dir: &Path, relative: &str) -> PathBuf {
    // Paths in the manifest use '/', and the parser has already refused any that
    // could leave the model folder.
    relative
        .split('/')
        .fold(dir.to_path_buf(), |path, part| path.join(part))
}

fn file_len(path: &Path) -> Result<u64, ModelError> {
    fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| ModelError::io(format!("reading {}", path.display()), e))
}

fn is_installed_and_matches(
    dest: &Path,
    expected: &str,
    cancel: &CancelFlag,
) -> Result<bool, ModelError> {
    if !dest.is_file() {
        return Ok(false);
    }
    Ok(sha256_file(dest, cancel)? == expected)
}
