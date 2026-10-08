//! Speech models: what the manifest offers, what is installed, and downloads.
//!
//! The work is `model-manager`'s: it reads `models/manifest.toml`, refuses an
//! entry without a checksum before any request, downloads with resume, verifies
//! SHA-256 and records the install. This module adds the three things the server
//! needs around it:
//!
//! * the licence rule: a download starts only when the request names the licence
//!   that was shown, so a learner never accepts one text and gets another;
//! * one download at a time. A second request is refused with `Busy`;
//! * progress as `DownloadProgress` events, at most ten per second per download,
//!   and a cancel that reaches the transfer within one read.
//!
//! A download is blocking work. It runs on a blocking thread, never on a Tokio
//! worker, and holds no lock on anything the rest of the program reads.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use model_manager::{
    DownloadProgress as Progress, Manifest, ModelEntry, ModelError, ModelManager, ModelRole,
};
use speech::CancelFlag;

use crate::api::{
    Ack, DownloadRequest, DownloadStarted, DownloadState, Downloadable, Feature,
    InstalledModelView, LicenceView, ModelList, ModelRoleView, ModelView, ServerEvent,
};
use crate::core::AppCore;
use crate::error::{CoreError, CoreResult};
use crate::events::EventBus;

/// Progress events of one download are at least this far apart. A file that ends
/// and a download that ends are always reported.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

struct ActiveDownload {
    model_id: String,
    cancel: CancelFlag,
}

/// The manifest and the folder models are installed under.
pub(crate) struct ModelHub {
    manifest: Manifest,
    root: PathBuf,
    /// The one download that is running, if any.
    active: Arc<Mutex<Option<ActiveDownload>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ModelHub {
    /// Reads the manifest. `Err` carries a sentence for the log and for the
    /// snapshot; models are then not available. Blocking: call it on a blocking
    /// thread.
    pub(crate) fn load(manifest_path: &Path, root: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(manifest_path)
            .map_err(|error| format!("the model manifest cannot be read ({:?})", error.kind()))?;
        let manifest = Manifest::parse(&text)
            .map_err(|error| format!("the model manifest is not valid: {error}"))?;
        Ok(Self {
            manifest,
            root: root.to_owned(),
            active: Arc::new(Mutex::new(None)),
        })
    }

    fn downloading(&self) -> Option<String> {
        lock(&self.active).as_ref().map(|a| a.model_id.clone())
    }
}

fn role_view(role: ModelRole) -> ModelRoleView {
    match role {
        ModelRole::Vad => ModelRoleView::Vad,
        ModelRole::Stt => ModelRoleView::Stt,
        ModelRole::Tts => ModelRoleView::Tts,
        ModelRole::Pron => ModelRoleView::Pron,
    }
}

fn view_of(entry: &ModelEntry, manager: &ModelManager, downloading: Option<&str>) -> ModelView {
    let notice = entry.licence_notice();
    let installed = manager
        .installed_record(&entry.id)
        .map(|record| InstalledModelView {
            version: record.version.clone(),
            files: u32::try_from(record.files.len()).unwrap_or(u32::MAX),
            size_bytes: record.files.iter().map(|f| f.size_bytes).sum(),
            combined_sha256: record.combined_sha256(),
        });
    ModelView {
        id: entry.id.clone(),
        role: role_view(entry.role),
        engine: entry.engine.clone(),
        version: entry.version.clone(),
        size_bytes: entry.size_bytes,
        licence: LicenceView {
            license: notice.license,
            license_url: notice.license_url,
            text: notice.text,
            needs_review: notice.needs_review,
        },
        downloadable: match entry.check_downloadable() {
            Ok(()) => Downloadable::Yes,
            Err(reason) => Downloadable::No {
                reason: reason.to_string(),
            },
        },
        installed,
        downloading: downloading == Some(entry.id.as_str()),
        notes: entry.notes.clone(),
    }
}

/// What a failed download says to the learner. It names the kind of failure and
/// never a path or an address.
fn failure_sentence(error: &ModelError) -> String {
    match error {
        ModelError::Cancelled => "the download was cancelled".to_owned(),
        ModelError::ChecksumMismatch { .. } => {
            "a downloaded file did not match its checksum and was discarded".to_owned()
        }
        ModelError::Interrupted { .. } | ModelError::Stalled { .. } => {
            "the transfer was interrupted; start the download again to resume".to_owned()
        }
        ModelError::Status { status, .. } => {
            format!("the model host answered with HTTP status {status}")
        }
        ModelError::TooLarge { .. } => {
            "a file was larger than the manifest says and was discarded".to_owned()
        }
        ModelError::Io { .. } => "a model file could not be read or written".to_owned(),
        ModelError::Client(_) | ModelError::BadResponse(_) => {
            "the download could not be carried out".to_owned()
        }
        other => other.to_string(),
    }
}

struct Reporter {
    bus: Arc<EventBus>,
    model_id: String,
    last: Option<(Instant, usize)>,
    latest: Option<Progress>,
}

impl Reporter {
    fn publish(&self, progress: &Progress, state: DownloadState, message: Option<String>) {
        self.bus.publish(|seq| ServerEvent::DownloadProgress {
            seq,
            model_id: progress.model_id.clone(),
            file: progress.file.clone(),
            file_index: u32::try_from(progress.file_index).unwrap_or(u32::MAX),
            file_count: u32::try_from(progress.file_count).unwrap_or(u32::MAX),
            bytes_done: progress.bytes_done,
            bytes_total: progress.bytes_total,
            state,
            message,
        });
    }

    fn on_progress(&mut self, progress: &Progress) {
        let file_done = progress.bytes_total == Some(progress.bytes_done);
        let new_file = self
            .last
            .is_none_or(|(_, index)| index != progress.file_index);
        let due = self
            .last
            .is_none_or(|(at, _)| at.elapsed() >= PROGRESS_EVERY);
        if due || file_done || new_file {
            self.last = Some((Instant::now(), progress.file_index));
            self.publish(progress, DownloadState::Running, None);
        }
        self.latest = Some(progress.clone());
    }

    /// The last event of the download: always sent.
    fn finish(&self, state: DownloadState, message: Option<String>) {
        let progress = self.latest.clone().unwrap_or_else(|| Progress {
            model_id: self.model_id.clone(),
            file: String::new(),
            file_index: 0,
            file_count: 0,
            bytes_done: 0,
            bytes_total: None,
        });
        self.publish(&progress, state, message);
    }
}

impl AppCore {
    fn model_hub(&self) -> CoreResult<&ModelHub> {
        self.models.as_ref().ok_or_else(|| {
            CoreError::unavailable(
                Some(Feature::Models),
                self.models_problem
                    .clone()
                    .unwrap_or_else(|| "model downloads are not available".to_owned()),
            )
        })
    }

    /// The models of the manifest with their licences, whether each can be
    /// downloaded, and what is installed.
    pub async fn models(&self) -> CoreResult<ModelList> {
        let hub = self.model_hub()?;
        let manifest = hub.manifest.clone();
        let root = hub.root.clone();
        let downloading = hub.downloading();
        tokio::task::spawn_blocking(move || {
            let manager = ModelManager::open(manifest, &root)
                .map_err(|error| CoreError::Internal(error.to_string()))?;
            let models = manager
                .manifest()
                .models
                .iter()
                .map(|entry| view_of(entry, &manager, downloading.as_deref()))
                .collect();
            Ok(ModelList { models })
        })
        .await
        .map_err(|_| CoreError::Internal("reading the model record did not finish".to_owned()))?
    }

    /// Starts downloading a model. The request must name the licence that
    /// `models` showed; the manifest entry must have files with checksums. Both
    /// are checked before anything is fetched. Returns when the download has
    /// started: progress and the end arrive as `DownloadProgress` events.
    pub async fn download_model(
        &self,
        id: &str,
        request: DownloadRequest,
    ) -> CoreResult<DownloadStarted> {
        self.ensure_running()?;
        let hub = self.model_hub()?;
        let entry = hub
            .manifest
            .entry(id)
            .cloned()
            .ok_or(CoreError::NotFound { what: "model" })?;
        entry
            .check_downloadable()
            .map_err(|reason| CoreError::Conflict(reason.to_string()))?;
        if !request.accept_licence {
            return Err(CoreError::LicenceNotAccepted(
                "the licence of this model must be accepted before it is downloaded".to_owned(),
            ));
        }
        if request.license != entry.license || request.license_url != entry.license_url {
            return Err(CoreError::LicenceNotAccepted(
                "the licence in the request is not the current licence of this model; read it again"
                    .to_owned(),
            ));
        }
        let acceptance = entry.licence_notice().accept();

        let cancel = CancelFlag::new();
        {
            let mut active = lock(&hub.active);
            if active.is_some() {
                return Err(CoreError::Busy);
            }
            *active = Some(ActiveDownload {
                model_id: entry.id.clone(),
                cancel: cancel.clone(),
            });
        }

        let files = u32::try_from(entry.files.len()).unwrap_or(u32::MAX);
        let manifest = hub.manifest.clone();
        let root = hub.root.clone();
        let slot = Arc::clone(&hub.active);
        let bus = Arc::clone(&self.bus);
        let model_id = entry.id.clone();
        let shutdown = self.shutdown.clone();
        // The program is stopping: end the transfer, which keeps its partial file.
        let watcher_cancel = cancel.clone();
        let done = tokio_util::sync::CancellationToken::new();
        let watcher_done = done.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = shutdown.cancelled() => watcher_cancel.cancel(),
                () = watcher_done.cancelled() => {}
            }
        });
        let handle = tokio::runtime::Handle::current();
        let _detached = handle.spawn_blocking(move || {
            let mut reporter = Reporter {
                bus: Arc::clone(&bus),
                model_id: model_id.clone(),
                last: None,
                latest: None,
            };
            let outcome = ModelManager::open(manifest, &root).and_then(|mut manager| {
                manager
                    .install(&model_id, &acceptance, &cancel, &mut |progress| {
                        reporter.on_progress(progress);
                    })
                    .map(|_| ())
            });
            // The slot opens before the last event, so a client that sees the end
            // can start the next download at once.
            *lock(&slot) = None;
            match outcome {
                Ok(()) => reporter.finish(DownloadState::Done, None),
                Err(ModelError::Cancelled) => reporter.finish(DownloadState::Cancelled, None),
                Err(error) => {
                    tracing::warn!(%error, "a model download failed");
                    reporter.finish(DownloadState::Failed, Some(failure_sentence(&error)));
                }
            }
            done.cancel();
        });
        Ok(DownloadStarted {
            model_id: entry.id,
            files,
        })
    }

    /// Cancels the download of `id`. The partial file is kept, so starting the
    /// download again resumes it.
    pub fn cancel_download(&self, id: &str) -> CoreResult<Ack> {
        let hub = self.model_hub()?;
        match lock(&hub.active).as_ref() {
            Some(active) if active.model_id == id => {
                active.cancel.cancel();
                Ok(Ack { ok: true })
            }
            Some(_) | None => Err(CoreError::NotFound {
                what: "running download",
            }),
        }
    }
}
