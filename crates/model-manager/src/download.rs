//! Resumable download of one file with SHA-256 verification.
//!
//! The file is written to `<name>.part`. A later attempt finds the partial file
//! and asks the server for the rest with `Range: bytes=<len>-`. The answer
//! decides what happens next:
//!
//! * `206 Partial Content` starting at `<len>`: the bytes are appended.
//! * `200 OK`: the server ignored the range. The partial file is replaced.
//! * `416 Range Not Satisfiable`: the partial file is already as long as the
//!   file, or longer. The checksum decides.
//!
//! Only a file whose SHA-256 matches the manifest is moved to its final name.
//! A resumed file that fails the check is thrown away and fetched once more from
//! the start, because a stale partial file from an older version is the usual
//! cause. A fresh file that fails the check is an error and nothing is kept.
//!
//! This code blocks. It runs its own single-thread runtime for the HTTP
//! client, so call it from a dedicated thread, never from a Tokio worker.

use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use speech::CancelFlag;

use crate::error::ModelError;
use crate::hash::sha256_file;
use crate::manifest::url_problem;

/// How often a waiting transfer looks at the cancel flag.
const POLL: Duration = Duration::from_millis(100);
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Clone, Copy)]
pub(crate) struct DownloadOptions {
    /// A step that yields no data for this long fails with `Stalled`. The
    /// partial file is kept for the next attempt.
    pub stall_limit: Duration,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            stall_limit: Duration::from_secs(30),
        }
    }
}

/// What one call did, for tests and logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DownloadReport {
    /// Bytes that were already on disk when the call began.
    pub resumed_from: u64,
    /// Whether the first attempt failed the checksum and a second one ran.
    pub restarted: bool,
}

pub(crate) fn build_client() -> Result<Client, ModelError> {
    let policy = Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if let Some(reason) = url_problem(attempt.url()) {
            attempt.error(format!("a redirect was refused: {reason}"))
        } else {
            attempt.follow()
        }
    });
    Client::builder()
        .user_agent(concat!("lumingo-model-manager/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .redirect(policy)
        .build()
        .map_err(|e| ModelError::Client(e.without_url().to_string()))
}

pub(crate) fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// Everything one file download needs.
#[derive(Clone, Copy)]
pub(crate) struct DownloadJob<'a> {
    pub client: &'a Client,
    pub url: &'a Url,
    pub dest: &'a Path,
    /// 64 lowercase hexadecimal characters.
    pub expected_sha256: &'a str,
    pub size_limit: Option<u64>,
    pub options: DownloadOptions,
    pub cancel: &'a CancelFlag,
}

/// Downloads `url` to `dest` and verifies it against `expected_sha256`, which
/// must be 64 lowercase hexadecimal characters.
///
/// `progress` receives the bytes on disk so far (resumed bytes included) and
/// the total when the server declared one.
pub(crate) fn download_verified(
    job: &DownloadJob<'_>,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<DownloadReport, ModelError> {
    let DownloadJob {
        dest,
        expected_sha256,
        size_limit,
        cancel,
        ..
    } = *job;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| ModelError::io("starting the download runtime", e))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| ModelError::io(format!("creating {}", parent.display()), e))?;
    }
    let part = part_path(dest);
    let mut restarted = false;
    let mut first_resume = None;
    loop {
        let mut resume_from = if restarted { 0 } else { part_len(&part)? };
        // A partial file longer than the declared size can never become right.
        if size_limit.is_some_and(|limit| resume_from > limit) {
            remove_part(&part)?;
            resume_from = 0;
        }
        if restarted {
            remove_part(&part)?;
        }
        first_resume.get_or_insert(resume_from);
        runtime.block_on(fetch(job, &part, resume_from, progress))?;
        let actual = sha256_file(&part, cancel)?;
        if actual == expected_sha256 {
            fs::rename(&part, dest)
                .map_err(|e| ModelError::io(format!("moving {} into place", part.display()), e))?;
            return Ok(DownloadReport {
                resumed_from: first_resume.unwrap_or(0),
                restarted,
            });
        }
        remove_part(&part)?;
        if resume_from > 0 && !restarted {
            tracing::warn!(
                "a resumed download failed its checksum; fetching it again from the start"
            );
            restarted = true;
            continue;
        }
        return Err(ModelError::ChecksumMismatch {
            path: dest.display().to_string(),
            expected: expected_sha256.to_owned(),
            actual,
        });
    }
}

fn part_len(part: &Path) -> Result<u64, ModelError> {
    match fs::metadata(part) {
        Ok(meta) => Ok(meta.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(ModelError::io(format!("reading {}", part.display()), e)),
    }
}

fn remove_part(part: &Path) -> Result<(), ModelError> {
    match fs::remove_file(part) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ModelError::io(format!("removing {}", part.display()), e)),
    }
}

/// Waits for one network step, looking at the cancel flag every [`POLL`] and
/// failing when the step takes longer than the stall limit.
async fn step<T>(
    future: impl Future<Output = T>,
    options: DownloadOptions,
    cancel: &CancelFlag,
) -> Result<T, ModelError> {
    tokio::pin!(future);
    let started = Instant::now();
    loop {
        match tokio::time::timeout(POLL, &mut future).await {
            Ok(value) => return Ok(value),
            Err(_) => {
                if cancel.is_cancelled() {
                    return Err(ModelError::Cancelled);
                }
                if started.elapsed() >= options.stall_limit {
                    return Err(ModelError::Stalled {
                        seconds: options.stall_limit.as_secs().max(1),
                    });
                }
            }
        }
    }
}

async fn fetch(
    job: &DownloadJob<'_>,
    part: &Path,
    resume_from: u64,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), ModelError> {
    let DownloadJob {
        client,
        url,
        size_limit,
        options,
        cancel,
        ..
    } = *job;
    if cancel.is_cancelled() {
        return Err(ModelError::Cancelled);
    }
    let mut request = client.get(url.clone());
    if resume_from > 0 {
        request = request.header(RANGE, format!("bytes={resume_from}-"));
    }
    let mut response = step(request.send(), options, cancel)
        .await?
        .map_err(|e| ModelError::Client(e.without_url().to_string()))?;

    let status = response.status().as_u16();
    let (append, total) = match status {
        206 => {
            let header = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ModelError::BadResponse("206 without Content-Range".into()))?;
            let (start, total) = parse_content_range(header)
                .ok_or_else(|| ModelError::BadResponse(format!("Content-Range {header:?}")))?;
            if start != resume_from {
                return Err(ModelError::BadResponse(format!(
                    "asked for byte {resume_from}, the server started at {start}"
                )));
            }
            (
                true,
                total.or_else(|| response.content_length().map(|n| n + start)),
            )
        }
        200 => (false, response.content_length()),
        // The partial file is as long as the file or longer: the checksum
        // decides whether it is the whole file.
        416 if resume_from > 0 => return Ok(()),
        other => {
            return Err(ModelError::Status {
                url: url.to_string(),
                status: other,
            });
        }
    };

    let mut file = open_part(part, append)?;
    let mut written = if append { resume_from } else { 0 };
    progress(written, total);
    loop {
        // Data that arrives fast never makes `step` time out, so look here too.
        if cancel.is_cancelled() {
            flush(&mut file, part)?;
            return Err(ModelError::Cancelled);
        }
        let chunk = match step(response.chunk(), options, cancel).await? {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            // The connection broke mid-body. What is on disk stays for resuming.
            Err(_) => {
                flush(&mut file, part)?;
                return Err(ModelError::Interrupted {
                    received: written,
                    expected: total,
                });
            }
        };
        file.write_all(&chunk)
            .map_err(|e| ModelError::io(format!("writing {}", part.display()), e))?;
        written += chunk.len() as u64;
        if size_limit.is_some_and(|limit| written > limit) {
            drop(file);
            remove_part(part)?;
            return Err(ModelError::TooLarge {
                path: part.display().to_string(),
                limit: size_limit.unwrap_or(0),
            });
        }
        progress(written, total);
    }
    flush(&mut file, part)?;
    match total {
        Some(expected) if written < expected => Err(ModelError::Interrupted {
            received: written,
            expected: Some(expected),
        }),
        Some(expected) if written > expected => Err(ModelError::BadResponse(format!(
            "received {written} bytes, the server declared {expected}"
        ))),
        _ => Ok(()),
    }
}

fn open_part(part: &Path, append: bool) -> Result<File, ModelError> {
    let mut options = OpenOptions::new();
    options.create(true);
    if append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    options
        .open(part)
        .map_err(|e| ModelError::io(format!("opening {}", part.display()), e))
}

fn flush(file: &mut File, part: &Path) -> Result<(), ModelError> {
    file.flush()
        .map_err(|e| ModelError::io(format!("writing {}", part.display()), e))
}

/// `bytes 100-199/200` gives `(100, Some(200))`. `bytes 100-199/*` gives
/// `(100, None)`.
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, _end) = range.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let total = match total.trim() {
        "*" => None,
        number => Some(number.parse().ok()?),
    };
    Some((start, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_values_parse() {
        assert_eq!(
            parse_content_range("bytes 100-199/200"),
            Some((100, Some(200)))
        );
        assert_eq!(parse_content_range("bytes 0-0/1"), Some((0, Some(1))));
        assert_eq!(parse_content_range("bytes 100-199/*"), Some((100, None)));
        assert_eq!(parse_content_range("bytes */200"), None);
        assert_eq!(parse_content_range("items 1-2/3"), None);
        assert_eq!(parse_content_range("bytes a-b/c"), None);
    }

    #[test]
    fn the_part_file_sits_next_to_the_target() {
        assert_eq!(
            part_path(Path::new("/m/stt/encoder.onnx")),
            PathBuf::from("/m/stt/encoder.onnx.part")
        );
    }
}
