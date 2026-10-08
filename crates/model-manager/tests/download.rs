#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
//! Downloads against a local HTTP server: resume, checksums, refusals.

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use common::{Route, TestServer, pseudo_random, sha256_hex};
use model_manager::{
    DownloadProgress, InstalledRecord, Manifest, ModelError, ModelManager, NotDownloadable,
};
use speech::CancelFlag;
use tempfile::TempDir;

struct FileSpec {
    path: &'static str,
    sha256: String,
}

fn manifest_toml(base: &str, files: &[FileSpec]) -> String {
    let list = files
        .iter()
        .map(|f| format!(r#"{{ path = "{}", sha256 = "{}" }}"#, f.path, f.sha256))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
[[model]]
id = "stt-test"
role = "stt"
engine = "test-engine"
version = "1"
license = "MIT"
license_url = "https://example.org/licence"
license_text = "Permission is granted, free of charge, ..."
source = "{base}/models/stt-test"
files = [{list}]
"#
    )
}

struct Setup {
    server: TestServer,
    dir: TempDir,
}

impl Setup {
    fn new() -> Self {
        Self {
            server: TestServer::start(),
            dir: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn manager(&self, files: &[FileSpec]) -> ModelManager {
        let manifest =
            Manifest::parse(&manifest_toml(&self.server.base_url(), files)).expect("manifest");
        ModelManager::open(manifest, self.dir.path())
            .expect("manager")
            .with_stall_limit(Duration::from_millis(400))
    }

    fn model_file(&self, name: &str) -> PathBuf {
        self.dir.path().join("stt-test").join(name)
    }
}

fn install(
    manager: &mut ModelManager,
    cancel: &CancelFlag,
) -> (Result<InstalledRecord, ModelError>, Vec<DownloadProgress>) {
    let acceptance = manager.licence_notice("stt-test").expect("notice").accept();
    let mut events = Vec::new();
    let result = manager
        .install("stt-test", &acceptance, cancel, &mut |p| {
            events.push(p.clone());
        })
        .cloned();
    (result, events)
}

fn spec(path: &'static str, body: &[u8]) -> FileSpec {
    FileSpec {
        path,
        sha256: sha256_hex(body),
    }
}

#[test]
fn a_clean_download_is_verified_installed_and_recorded() {
    let setup = Setup::new();
    let body = pseudo_random(300_000, 1);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);

    let (result, progress) = install(&mut manager, &CancelFlag::new());
    let record = result.expect("installs");

    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
    assert!(!setup.model_file("model.onnx.part").exists());
    assert_eq!(record.files[0].sha256, sha256_hex(&body));
    assert_eq!(record.files[0].size_bytes, 300_000);
    assert_eq!(record.license, "MIT");
    assert!(record.installed_at_unix > 1_700_000_000);
    assert_eq!(record.combined_sha256().len(), 64);
    let last = progress.last().expect("progress events");
    assert_eq!(
        (last.bytes_done, last.file_index, last.file_count),
        (300_000, 0, 1)
    );

    // The record survives a restart.
    let reopened = ModelManager::open(manager.manifest().clone(), setup.dir.path()).expect("open");
    assert_eq!(reopened.installed_record("stt-test"), Some(&record));
    manager
        .verify_installed("stt-test", &CancelFlag::new())
        .expect("files still match");
}

#[test]
fn an_interrupted_download_resumes_with_a_range_request() {
    let setup = Setup::new();
    let body = pseudo_random(300_000, 2);
    let path = "/models/stt-test/model.onnx";
    let mut route = Route::new(body.clone());
    route.cut_after = Some(120_000);
    route.cuts_remaining = 1;
    setup.server.set(path, route);
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);

    let (first, _) = install(&mut manager, &CancelFlag::new());
    let Err(ModelError::Interrupted { received, expected }) = first else {
        panic!("expected Interrupted, got {first:?}");
    };
    assert_eq!(expected, Some(300_000));
    assert!(received > 0 && received < 300_000);
    let part = setup.model_file("model.onnx.part");
    assert_eq!(fs::metadata(&part).expect("part kept").len(), received);
    assert!(!setup.model_file("model.onnx").exists());
    assert!(manager.installed_record("stt-test").is_none());

    let (second, _) = install(&mut manager, &CancelFlag::new());
    second.expect("the second attempt completes");

    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
    assert!(!part.exists());
    let requests = setup.server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].range, None);
    assert_eq!(requests[1].range, Some(format!("bytes={received}-")));
    assert_eq!(requests[1].status, 206);
    // The second response carried only what was missing.
    assert_eq!(requests[1].body_bytes_sent, 300_000 - received as usize);
}

#[test]
fn a_server_that_ignores_range_replaces_the_partial_file() {
    let setup = Setup::new();
    let body = pseudo_random(100_000, 3);
    let path = "/models/stt-test/model.onnx";
    let mut route = Route::new(body.clone());
    route.honor_range = false;
    setup.server.set(path, route);
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);
    fs::create_dir_all(setup.dir.path().join("stt-test")).expect("dir");
    fs::write(setup.model_file("model.onnx.part"), &body[..40_000]).expect("part");

    let (result, _) = install(&mut manager, &CancelFlag::new());
    result.expect("installs");

    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
    let requests = setup.server.requests();
    assert_eq!(requests[0].range.as_deref(), Some("bytes=40000-"));
    assert_eq!(requests[0].status, 200);
}

#[test]
fn a_partial_file_that_is_already_complete_is_accepted_after_a_416() {
    let setup = Setup::new();
    let body = pseudo_random(50_000, 4);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);
    fs::create_dir_all(setup.dir.path().join("stt-test")).expect("dir");
    fs::write(setup.model_file("model.onnx.part"), &body).expect("part");

    let (result, _) = install(&mut manager, &CancelFlag::new());
    result.expect("installs");

    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
    let requests = setup.server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].status, 416);
}

#[test]
fn a_stale_partial_file_fails_the_check_and_is_fetched_again_from_the_start() {
    let setup = Setup::new();
    let body = pseudo_random(100_000, 5);
    let stale = pseudo_random(100_000, 99);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);
    fs::create_dir_all(setup.dir.path().join("stt-test")).expect("dir");
    // The first 30 000 bytes belong to some other version of the file.
    fs::write(setup.model_file("model.onnx.part"), &stale[..30_000]).expect("part");

    let (result, _) = install(&mut manager, &CancelFlag::new());
    result.expect("installs after one restart");

    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
    let requests = setup.server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].range.as_deref(), Some("bytes=30000-"));
    assert_eq!(requests[1].range, None, "the retry starts from byte 0");
}

#[test]
fn a_wrong_checksum_is_an_error_and_nothing_is_kept() {
    let setup = Setup::new();
    let served = pseudo_random(80_000, 6);
    let expected = pseudo_random(80_000, 7);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(served.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &expected)]);

    let (result, _) = install(&mut manager, &CancelFlag::new());
    let Err(ModelError::ChecksumMismatch {
        expected: want,
        actual,
        ..
    }) = result
    else {
        panic!("expected ChecksumMismatch, got {result:?}");
    };
    assert_eq!(want, sha256_hex(&expected));
    assert_eq!(actual, sha256_hex(&served));
    assert!(!setup.model_file("model.onnx").exists());
    assert!(!setup.model_file("model.onnx.part").exists());
    assert!(manager.installed_record("stt-test").is_none());
    assert_eq!(manager.installed().count(), 0);
    // A fresh download is tried once, not twice.
    assert_eq!(setup.server.request_count(), 1);
}

#[test]
fn an_entry_with_an_empty_checksum_is_refused_before_any_request() {
    let setup = Setup::new();
    let body = pseudo_random(1000, 8);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let empty = FileSpec {
        path: "model.onnx",
        sha256: String::new(),
    };
    let mut manager = setup.manager(&[empty]);

    let (result, _) = install(&mut manager, &CancelFlag::new());
    assert!(matches!(
        result,
        Err(ModelError::NotDownloadable(
            NotDownloadable::EmptyChecksum { .. }
        ))
    ));
    assert_eq!(setup.server.request_count(), 0);
    assert!(!setup.dir.path().join("stt-test").exists());
    assert_eq!(manager.undownloadable().len(), 1);
}

#[test]
fn the_licence_is_available_without_the_network_and_a_stale_acceptance_is_refused() {
    let setup = Setup::new();
    let body = pseudo_random(1000, 9);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);

    let notice = manager.licence_notice("stt-test").expect("notice");
    assert_eq!(notice.license, "MIT");
    assert_eq!(notice.license_url, "https://example.org/licence");
    assert!(
        notice
            .text
            .as_deref()
            .is_some_and(|t| t.starts_with("Permission"))
    );
    assert_eq!(setup.server.request_count(), 0);

    // An acceptance made for another licence text does not cover this entry.
    let other = Manifest::parse(
        &manifest_toml(&setup.server.base_url(), &[spec("model.onnx", &body)])
            .replace("license = \"MIT\"", "license = \"OpenRAIL-M\""),
    )
    .expect("manifest");
    let other_manager = ModelManager::open(other, setup.dir.path()).expect("manager");
    let stale = other_manager
        .licence_notice("stt-test")
        .expect("notice")
        .accept();
    let result = manager.install("stt-test", &stale, &CancelFlag::new(), &mut |_| {});
    assert!(matches!(result, Err(ModelError::LicenceNotAccepted { .. })));
    assert_eq!(setup.server.request_count(), 0);
}

#[test]
fn an_unknown_model_id_is_reported() {
    let setup = Setup::new();
    let manager = setup.manager(&[]);
    assert!(matches!(
        manager.licence_notice("nope"),
        Err(ModelError::UnknownModel(_))
    ));
}

#[test]
fn files_that_are_already_installed_are_not_fetched_again() {
    let setup = Setup::new();
    let a = pseudo_random(60_000, 10);
    let b = pseudo_random(70_000, 11);
    setup
        .server
        .set("/models/stt-test/a.onnx", Route::new(a.clone()));
    setup
        .server
        .set("/models/stt-test/b.bin", Route::new(b.clone()));
    let mut manager = setup.manager(&[spec("a.onnx", &a), spec("b.bin", &b)]);

    let (result, events) = install(&mut manager, &CancelFlag::new());
    result.expect("installs");
    assert_eq!(setup.server.request_count(), 2);
    assert_eq!(events.last().expect("events").file_index, 1);

    let (again, events) = install(&mut manager, &CancelFlag::new());
    again.expect("installs again");
    assert_eq!(setup.server.request_count(), 2, "no new requests");
    assert_eq!(events.len(), 2, "one progress event per skipped file");
}

#[test]
fn a_file_changed_after_install_fails_verification_and_is_repaired_by_install() {
    let setup = Setup::new();
    let body = pseudo_random(60_000, 12);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);
    install(&mut manager, &CancelFlag::new())
        .0
        .expect("installs");

    let mut damaged = body.clone();
    damaged[10] ^= 0xff;
    fs::write(setup.model_file("model.onnx"), &damaged).expect("damage");
    assert!(matches!(
        manager.verify_installed("stt-test", &CancelFlag::new()),
        Err(ModelError::ChecksumMismatch { .. })
    ));

    install(&mut manager, &CancelFlag::new())
        .0
        .expect("repairs");
    manager
        .verify_installed("stt-test", &CancelFlag::new())
        .expect("matches again");
    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
}

#[test]
fn cancelling_stops_the_download_and_keeps_the_partial_file() {
    let setup = Setup::new();
    let body = pseudo_random(400_000, 13);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);
    let cancel = CancelFlag::new();
    let acceptance = manager.licence_notice("stt-test").expect("notice").accept();

    let result = manager.install("stt-test", &acceptance, &cancel, &mut |p| {
        if p.bytes_done > 0 {
            cancel.cancel();
        }
    });
    assert!(matches!(result, Err(ModelError::Cancelled)), "{result:?}");
    assert!(!setup.model_file("model.onnx").exists());
    assert!(manager.installed_record("stt-test").is_none());

    // The same call with a fresh flag finishes the job.
    let (result, _) = install(&mut manager, &CancelFlag::new());
    result.expect("installs after the cancelled attempt");
}

#[test]
fn a_transfer_that_goes_quiet_fails_as_stalled_and_can_be_resumed() {
    let setup = Setup::new();
    let body = pseudo_random(200_000, 14);
    let path = "/models/stt-test/model.onnx";
    let mut route = Route::new(body.clone());
    route.stall_after = Some(50_000);
    setup.server.set(path, route);
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);

    let (result, _) = install(&mut manager, &CancelFlag::new());
    assert!(
        matches!(result, Err(ModelError::Stalled { .. })),
        "{result:?}"
    );
    let part_len = fs::metadata(setup.model_file("model.onnx.part"))
        .expect("part kept")
        .len();
    assert!(part_len > 0 && part_len <= 50_000);

    setup.server.update(path, |r| r.stall_after = None);
    let (result, _) = install(&mut manager, &CancelFlag::new());
    result.expect("resumes");
    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
}

#[test]
fn an_error_status_is_reported_and_leaves_no_files() {
    let setup = Setup::new();
    let body = pseudo_random(1000, 15);
    let mut route = Route::new(body.clone());
    route.status = Some(503);
    setup.server.set("/models/stt-test/model.onnx", route);
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);

    let (result, _) = install(&mut manager, &CancelFlag::new());
    assert!(matches!(
        result,
        Err(ModelError::Status { status: 503, .. })
    ));
    assert!(!setup.model_file("model.onnx.part").exists());

    let (missing, _) = {
        let other = setup.manager(&[spec("other.bin", &body)]);
        let mut other = other;
        install(&mut other, &CancelFlag::new())
    };
    assert!(matches!(
        missing,
        Err(ModelError::Status { status: 404, .. })
    ));
}

#[test]
fn a_redirect_is_followed_on_the_loopback_address() {
    let setup = Setup::new();
    let body = pseudo_random(30_000, 16);
    let mut redirect = Route::new(Vec::new());
    redirect.redirect_to = Some("/cdn/real.onnx".into());
    setup.server.set("/models/stt-test/model.onnx", redirect);
    setup.server.set("/cdn/real.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);

    install(&mut manager, &CancelFlag::new())
        .0
        .expect("installs");
    assert_eq!(
        fs::read(setup.model_file("model.onnx")).expect("file"),
        body
    );
}

#[test]
fn a_redirect_to_plain_http_on_another_host_is_refused() {
    let setup = Setup::new();
    let body = pseudo_random(1000, 17);
    let mut redirect = Route::new(Vec::new());
    // Not loopback, so refused before any connection is made to it.
    redirect.redirect_to = Some("http://example.invalid/model.onnx".into());
    setup.server.set("/models/stt-test/model.onnx", redirect);
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);

    let (result, _) = install(&mut manager, &CancelFlag::new());
    assert!(matches!(result, Err(ModelError::Client(_))), "{result:?}");
    assert!(!setup.model_file("model.onnx").exists());
}

#[test]
fn a_new_version_drops_files_the_old_one_listed() {
    let setup = Setup::new();
    let a = pseudo_random(10_000, 18);
    let b = pseudo_random(10_000, 19);
    setup
        .server
        .set("/models/stt-test/a.onnx", Route::new(a.clone()));
    setup
        .server
        .set("/models/stt-test/b.bin", Route::new(b.clone()));
    let mut manager = setup.manager(&[spec("a.onnx", &a), spec("b.bin", &b)]);
    install(&mut manager, &CancelFlag::new()).0.expect("v1");
    assert!(setup.model_file("b.bin").exists());

    let mut next = setup.manager(&[spec("a.onnx", &a)]);
    install(&mut next, &CancelFlag::new()).0.expect("v2");
    assert!(setup.model_file("a.onnx").exists());
    assert!(!setup.model_file("b.bin").exists());
    assert_eq!(
        next.installed_record("stt-test")
            .expect("record")
            .files
            .len(),
        1
    );
}

#[test]
fn uninstall_removes_files_and_record() {
    let setup = Setup::new();
    let body = pseudo_random(10_000, 20);
    setup
        .server
        .set("/models/stt-test/model.onnx", Route::new(body.clone()));
    let mut manager = setup.manager(&[spec("model.onnx", &body)]);
    install(&mut manager, &CancelFlag::new())
        .0
        .expect("installs");
    assert!(
        manager
            .installed_file_path("stt-test", "model.onnx")
            .is_some()
    );

    assert!(manager.uninstall("stt-test").expect("uninstall"));
    assert!(!setup.dir.path().join("stt-test").exists());
    assert!(manager.installed_record("stt-test").is_none());
    assert!(
        !manager
            .uninstall("stt-test")
            .expect("second uninstall is a no-op")
    );
}

#[test]
fn a_damaged_record_is_reported_not_silently_replaced() {
    let setup = Setup::new();
    fs::write(setup.dir.path().join("installed.json"), "{ not json").expect("write");
    let manifest = Manifest::parse(&manifest_toml(&setup.server.base_url(), &[])).expect("m");
    assert!(matches!(
        ModelManager::open(manifest, setup.dir.path()),
        Err(ModelError::Record { .. })
    ));
}
