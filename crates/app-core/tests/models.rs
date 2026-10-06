#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The model routes of the core: the manifest, the licence rule, one download at a
//! time with progress, cancel and the end states. The model host is a server on the
//! loopback address inside the test.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use app_core::api::{
    DownloadRequest, DownloadState, Downloadable, ErrorCode, Feature, ModelList, ServerEvent,
};
use app_core::error::CoreError;
use app_core::{AppCore, SessionManager};
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use common::sessions::Events;
use common::{ManualClock, TestCore, config};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

const LICENCE: &str = "MIT";
const LICENCE_URL: &str = "https://example.org/licence";

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A body of `len` bytes that is not all the same.
fn body(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i * 7 % 251).unwrap())
        .collect()
}

/// The model host: serves `/models/{id}/{file}` and counts the requests.
struct Host {
    base: String,
    requests: Arc<AtomicUsize>,
    /// Sends the body slowly, so a download is still running when the test looks.
    slow: Arc<AtomicBool>,
}

#[derive(Clone)]
struct HostState {
    requests: Arc<AtomicUsize>,
    slow: Arc<AtomicBool>,
    files: Arc<std::collections::HashMap<String, Vec<u8>>>,
}

async fn serve_file(
    State(state): State<HostState>,
    Path((_model, file)): Path<(String, String)>,
) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let Some(bytes) = state.files.get(&file).cloned() else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    };
    let slow = state.slow.load(Ordering::SeqCst);
    let total = bytes.len();
    let chunks: Vec<Vec<u8>> = bytes.chunks(4_096).map(<[u8]>::to_vec).collect();
    let stream = futures_util::stream::unfold(chunks.into_iter(), move |mut rest| async move {
        let next = rest.next()?;
        if slow {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(next)), rest))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, total)
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn host(files: Vec<(&'static str, Vec<u8>)>) -> Host {
    let state = HostState {
        requests: Arc::new(AtomicUsize::new(0)),
        slow: Arc::new(AtomicBool::new(false)),
        files: Arc::new(files.into_iter().map(|(n, b)| (n.to_owned(), b)).collect()),
    };
    let requests = Arc::clone(&state.requests);
    let slow = Arc::clone(&state.slow);
    let app = Router::new()
        .route("/models/{id}/{file}", get(serve_file))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Host {
        base,
        requests,
        slow,
    }
}

fn entry(id: &str, base: &str, files: &[(&str, &str)]) -> String {
    let list = files
        .iter()
        .map(|(path, sum)| format!(r#"{{ path = "{path}", sha256 = "{sum}" }}"#))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
[[model]]
id = "{id}"
role = "stt"
engine = "test-engine"
version = "1"
license = "{LICENCE}"
license_url = "{LICENCE_URL}"
license_text = "Permission is granted, free of charge, to use this test model."
source = "{base}/models/{id}"
files = [{list}]
"#
    )
}

/// A core whose manifest is `manifest`. The session manager is attached too, with
/// no audio and no speech, so the snapshot says what a real build with every
/// feature off says.
async fn core_with(manifest: &str) -> (TestCore, Events) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.toml");
    std::fs::write(&path, format!("schema_version = 1\n{manifest}")).unwrap();
    let clock = ManualClock::new("2026-10-05T08:00:00.000Z", "2026-10-05");
    let mut config = config(&dir, &clock, &[]);
    config.models_manifest = path;
    let core = AppCore::open(config).await.unwrap();
    SessionManager::attach(&core, app_core::engines::Engines::without("no audio"))
        .await
        .unwrap();
    let events = Events::collect(&core);
    (TestCore { core, dir, clock }, events)
}

fn accept(license: &str, url: &str) -> DownloadRequest {
    DownloadRequest {
        accept_licence: true,
        license: license.to_owned(),
        license_url: url.to_owned(),
    }
}

fn progress_of(events: &[ServerEvent], model: &str) -> Vec<(DownloadState, u64, Option<u64>)> {
    events
        .iter()
        .filter_map(|e| match e {
            ServerEvent::DownloadProgress {
                model_id,
                state,
                bytes_done,
                bytes_total,
                ..
            } if model_id == model => Some((*state, *bytes_done, *bytes_total)),
            _ => None,
        })
        .collect()
}

async fn finished(events: &Events, model: &str) -> Vec<(DownloadState, u64, Option<u64>)> {
    let all = events
        .wait("the download to end", |events| {
            progress_of(events, model)
                .iter()
                .any(|(state, ..)| *state != DownloadState::Running)
        })
        .await;
    progress_of(&all, model)
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_manifest_models_are_not_available_and_the_snapshot_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new("2026-10-05T08:00:00.000Z", "2026-10-05");
    let mut config = config(&dir, &clock, &[]);
    config.models_manifest = dir.path().join("missing.toml");
    let core = AppCore::open(config).await.unwrap();
    SessionManager::attach(&core, app_core::engines::Engines::without("no audio"))
        .await
        .unwrap();

    let refused = core.models().await.unwrap_err();
    let body = refused.body();
    assert_eq!(body.error, ErrorCode::NotAvailable);
    assert_eq!(body.feature, Some(Feature::Models));
    assert!(body.message.contains("manifest"), "{}", body.message);
    assert!(
        !body.message.contains(dir.path().to_str().unwrap()),
        "no path"
    );
    assert!(core.snapshot().unavailable.contains(&Feature::Models));
    let download = core
        .download_model("any", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap_err();
    assert_eq!(download.body().error, ErrorCode::NotAvailable);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shipped_manifest_lists_every_model_as_not_downloadable_with_the_reason() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new("2026-10-05T08:00:00.000Z", "2026-10-05");
    let mut config = config(&dir, &clock, &[]);
    config.models_manifest = path;
    let core = AppCore::open(config).await.unwrap();
    SessionManager::attach(&core, app_core::engines::Engines::without("no audio"))
        .await
        .unwrap();

    let ModelList { models } = core.models().await.unwrap();
    assert!(!models.is_empty());
    for model in &models {
        assert!(!model.licence.license.is_empty(), "{}", model.id);
        assert!(model.installed.is_none());
        assert!(!model.downloading);
        let Downloadable::No { reason } = &model.downloadable else {
            panic!(
                "{} is downloadable although its checksums are empty",
                model.id
            );
        };
        assert!(!reason.is_empty());
    }
    assert!(
        !core.snapshot().unavailable.contains(&Feature::Models),
        "the manifest loaded, so the list is available"
    );

    // Nothing is fetched for a candidate: the refusal comes before any request.
    let first = &models[0];
    let refused = core
        .download_model(
            &first.id,
            accept(&first.licence.license, &first.licence.license_url),
        )
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::Conflict(_)), "{refused:?}");
    let unknown = core
        .download_model("no-such-model", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap_err();
    assert!(matches!(unknown, CoreError::NotFound { .. }), "{unknown:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_needs_the_licence_that_was_shown_and_nothing_is_fetched_without_it() {
    let bytes = body(10_000);
    let host = host(vec![("m.onnx", bytes.clone())]).await;
    let (t, _events) = core_with(&entry(
        "stt-test",
        &host.base,
        &[("m.onnx", &sha256(&bytes))],
    ))
    .await;

    let not_accepted = DownloadRequest {
        accept_licence: false,
        ..accept(LICENCE, LICENCE_URL)
    };
    for request in [
        not_accepted,
        accept("Apache-2.0", LICENCE_URL),
        accept(LICENCE, "https://example.org/another-licence"),
        accept("", ""),
    ] {
        let refused = t
            .core
            .download_model("stt-test", request)
            .await
            .unwrap_err();
        assert!(
            matches!(refused, CoreError::LicenceNotAccepted(_)),
            "{refused:?}"
        );
        let body = refused.body();
        assert_eq!(body.error, ErrorCode::LicenceNotAccepted);
        assert!(!body.message.is_empty());
    }
    assert_eq!(
        host.requests.load(Ordering::SeqCst),
        0,
        "no request was made"
    );
    let list = t.core.models().await.unwrap();
    assert_eq!(list.models[0].downloadable, Downloadable::Yes);
    assert!(list.models[0].installed.is_none());
    assert_eq!(list.models[0].licence.license, LICENCE);
    assert!(list.models[0].licence.text.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_entry_with_an_empty_checksum_is_refused_before_any_request() {
    let host = host(vec![("m.onnx", body(1_000))]).await;
    let (t, _events) = core_with(&entry("stt-test", &host.base, &[("m.onnx", "")])).await;
    let list = t.core.models().await.unwrap();
    assert!(matches!(
        list.models[0].downloadable,
        Downloadable::No { .. }
    ));
    let refused = t
        .core
        .download_model("stt-test", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::Conflict(_)), "{refused:?}");
    assert!(
        refused.body().message.contains("checksum"),
        "{}",
        refused.body().message
    );
    assert_eq!(host.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_reports_progress_ends_done_and_the_model_shows_as_installed() {
    let bytes = body(400_000);
    let host = host(vec![("m.onnx", bytes.clone())]).await;
    host.slow.store(true, Ordering::SeqCst);
    let (t, events) = core_with(&entry(
        "stt-test",
        &host.base,
        &[("m.onnx", &sha256(&bytes))],
    ))
    .await;
    let started = std::time::Instant::now();
    let accepted = t
        .core
        .download_model("stt-test", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap();
    assert_eq!(accepted.model_id, "stt-test");
    assert_eq!(accepted.files, 1);
    assert!(t.core.models().await.unwrap().models[0].downloading);

    let progress = finished(&events, "stt-test").await;
    let took = started.elapsed();
    let (last_state, last_done, last_total) = *progress.last().unwrap();
    assert_eq!(last_state, DownloadState::Done);
    assert_eq!(last_done, 400_000);
    assert_eq!(last_total, Some(400_000));
    let running: Vec<u64> = progress
        .iter()
        .filter(|(s, ..)| *s == DownloadState::Running)
        .map(|(_, done, _)| *done)
        .collect();
    assert!(
        running.len() >= 2,
        "progress was reported while it ran: {progress:?}"
    );
    assert!(running.windows(2).all(|w| w[0] <= w[1]), "{running:?}");
    // At most ten progress events a second, with one more for the end of a file.
    assert!(
        running.len() as f64 <= took.as_secs_f64() * 10.0 + 3.0,
        "{} events in {took:?}",
        running.len()
    );

    let list = t.core.models().await.unwrap();
    let model = &list.models[0];
    assert!(!model.downloading);
    let installed = model.installed.as_ref().expect("installed");
    assert_eq!(installed.files, 1);
    assert_eq!(installed.size_bytes, 400_000);
    assert_eq!(installed.combined_sha256.len(), 64);
    let file = t.core.config().models_dir().join("stt-test").join("m.onnx");
    assert_eq!(std::fs::read(file).unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_does_not_match_its_checksum_fails_the_download_and_leaves_nothing() {
    let host = host(vec![("m.onnx", body(20_000))]).await;
    let wrong = sha256(b"something else");
    let (t, events) = core_with(&entry("stt-test", &host.base, &[("m.onnx", &wrong)])).await;
    t.core
        .download_model("stt-test", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap();
    let all = events
        .wait("the failure", |events| {
            events.iter().any(|e| {
                matches!(
                    e,
                    ServerEvent::DownloadProgress {
                        state: DownloadState::Failed,
                        ..
                    }
                )
            })
        })
        .await;
    let message = all
        .iter()
        .find_map(|e| match e {
            ServerEvent::DownloadProgress {
                state: DownloadState::Failed,
                message,
                ..
            } => message.clone(),
            _ => None,
        })
        .expect("a sentence for the learner");
    assert!(message.contains("checksum"), "{message}");
    assert!(
        !message.contains(t.dir.path().to_str().unwrap()) && !message.contains("127.0.0.1"),
        "no path and no address: {message}"
    );
    let list = t.core.models().await.unwrap();
    assert!(list.models[0].installed.is_none());
    assert!(!list.models[0].downloading, "the slot is free again");
    assert!(
        !t.core
            .config()
            .models_dir()
            .join("stt-test")
            .join("m.onnx")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_download_runs_at_a_time_and_a_cancel_reaches_the_transfer() {
    let bytes = body(2_000_000);
    let host = host(vec![("a.onnx", bytes.clone()), ("b.onnx", bytes.clone())]).await;
    host.slow.store(true, Ordering::SeqCst);
    let sum = sha256(&bytes);
    let manifest = format!(
        "{}{}",
        entry("stt-a", &host.base, &[("a.onnx", &sum)]),
        entry("stt-b", &host.base, &[("b.onnx", &sum)])
    );
    let (t, events) = core_with(&manifest).await;

    t.core
        .download_model("stt-a", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap();
    let second = t
        .core
        .download_model("stt-b", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap_err();
    assert!(matches!(second, CoreError::Busy), "{second:?}");
    // The other model is not the one that runs.
    assert!(matches!(
        t.core.cancel_download("stt-b"),
        Err(CoreError::NotFound { .. })
    ));
    events
        .wait("progress", |events| {
            !progress_of(events, "stt-a").is_empty()
        })
        .await;

    assert!(t.core.cancel_download("stt-a").unwrap().ok);
    let progress = finished(&events, "stt-a").await;
    assert_eq!(progress.last().unwrap().0, DownloadState::Cancelled);
    let list = t.core.models().await.unwrap();
    assert!(
        list.models
            .iter()
            .all(|m| m.installed.is_none() && !m.downloading)
    );

    // Nothing runs now: a cancel finds no download, and the next one may start.
    assert!(matches!(
        t.core.cancel_download("stt-a"),
        Err(CoreError::NotFound { .. })
    ));
    host.slow.store(false, Ordering::SeqCst);
    t.core
        .download_model("stt-b", accept(LICENCE, LICENCE_URL))
        .await
        .expect("the slot is free");
    let done = finished(&events, "stt-b").await;
    assert_eq!(done.last().unwrap().0, DownloadState::Done);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_is_not_started_once_the_program_is_stopping() {
    let bytes = body(1_000);
    let host = host(vec![("m.onnx", bytes.clone())]).await;
    let (t, _events) = core_with(&entry(
        "stt-test",
        &host.base,
        &[("m.onnx", &sha256(&bytes))],
    ))
    .await;
    t.core.request_shutdown();
    let refused = t
        .core
        .download_model("stt-test", accept(LICENCE, LICENCE_URL))
        .await
        .unwrap_err();
    assert!(matches!(refused, CoreError::ShuttingDown), "{refused:?}");
    assert_eq!(host.requests.load(Ordering::SeqCst), 0);
}
