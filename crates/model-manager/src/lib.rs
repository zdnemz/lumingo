//! Model manager: reads `models/manifest.toml`, shows the licence, downloads
//! with resume, verifies SHA-256 and records what is installed.
//!
//! Rules this crate keeps:
//!
//! * `models/manifest.toml` is the only source of download locations.
//! * An entry with an empty checksum is refused before any network request, so
//!   nothing is ever fetched that cannot be verified.
//! * A download starts only with a [`LicenceAcceptance`], which comes from a
//!   [`LicenceNotice`], so the licence is shown first.
//! * Every URL, and every redirect, must be HTTPS. Plain HTTP is accepted for the
//!   loopback address only, which is what the tests use. URLs with credentials
//!   are refused.
//! * Only a file whose SHA-256 matches is moved to its final name. A wrong
//!   checksum leaves nothing behind.
//! * Downloads block and honour a [`speech::CancelFlag`]. Run them on a
//!   dedicated thread. Model files are never committed or bundled: they are
//!   fetched on the learner's machine from the upstream host.
#![forbid(unsafe_code)]

mod download;
mod error;
mod hash;
mod manager;
mod manifest;
mod store;

pub use error::ModelError;
pub use manager::{DownloadProgress, ModelManager};
pub use manifest::{
    LicenceAcceptance, LicenceNotice, Manifest, ManifestError, ManifestFile, ModelEntry, ModelRole,
    NotDownloadable, SCHEMA_VERSION,
};
pub use store::{InstalledFile, InstalledRecord, RECORD_FILE};
