#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![cfg(feature = "cli")]
//! Drives the `pron` program end to end on a synthetic posterior file, so the
//! command line is exercised without a model. Run with `--features cli`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pron_engine::{Arpabet, LogPosteriors, Measurement, PhoneMap, SpeedProfile};

const TEST_LEXICON: &str = include_str!("data/test_lexicon.dict");

struct Workdir(PathBuf);

impl Workdir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pron-cli-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Workdir(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `<pad>` plus the first label of every ARPAbet symbol, as a vocab.json.
fn vocab_labels() -> Vec<String> {
    let map = PhoneMap::bundled_candidate().expect("bundled map");
    let mut labels = vec!["<pad>".to_owned()];
    for symbol in Arpabet::ALL {
        let first = map.labels_of(symbol)[0].clone();
        if !labels.contains(&first) {
            labels.push(first);
        }
    }
    labels
}

fn write_vocab(dir: &Workdir) -> PathBuf {
    let object: serde_json::Map<String, serde_json::Value> = vocab_labels()
        .into_iter()
        .enumerate()
        .map(|(i, l)| (l, serde_json::Value::from(i)))
        .collect();
    let path = dir.path("vocab.json");
    std::fs::write(&path, serde_json::Value::Object(object).to_string()).expect("vocab");
    path
}

/// A matrix saying `<blank> K K AE AE T T <blank>` with 0.9 on the intended label.
fn write_cat_posteriors(dir: &Workdir) -> PathBuf {
    let labels = vocab_labels();
    let map = PhoneMap::bundled_candidate().expect("bundled map");
    let column = |symbol: Arpabet| {
        labels
            .iter()
            .position(|l| *l == map.labels_of(symbol)[0])
            .expect("column")
    };
    let sequence = [
        0,
        column(Arpabet::K),
        column(Arpabet::K),
        column(Arpabet::AE),
        column(Arpabet::AE),
        column(Arpabet::T),
        column(Arpabet::T),
        0,
    ];
    let rest = 0.1 / (labels.len() - 1) as f32;
    let mut data = Vec::new();
    for dominant in sequence {
        for c in 0..labels.len() {
            data.push(if c == dominant {
                0.9f32.ln()
            } else {
                rest.ln()
            });
        }
    }
    let m = LogPosteriors::new(data, sequence.len(), labels.len()).expect("matrix");
    let path = dir.path("posteriors.json");
    std::fs::write(&path, m.to_json().expect("json")).expect("posteriors");
    path
}

fn write_lexicon(dir: &Workdir) -> PathBuf {
    let path = dir.path("test_lexicon.dict");
    std::fs::write(&path, TEST_LEXICON).expect("lexicon");
    path
}

fn pron(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pron"))
        .args(args)
        .output()
        .expect("runs pron")
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

#[test]
fn scores_a_posterior_file_and_labels_it_experimental_and_uncalibrated() {
    let dir = Workdir::new("score");
    let lex = write_lexicon(&dir);
    let vocab = write_vocab(&dir);
    let post = write_cat_posteriors(&dir);
    let out = pron(&[
        "score",
        "--text",
        "cat",
        "--lexicon",
        s(&lex),
        "--vocab",
        s(&vocab),
        "--posteriors",
        s(&post),
        "--focus",
        "K,TH",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(text.contains("experimental"));
    assert!(text.contains("Calibration: none"));
    assert!(text.contains("candidate-unverified-against-vocab"));
    assert!(text.contains("cat  [K AE T]"));
    assert!(text.contains("focus K: 1 occurrence(s)"));
    assert!(text.contains("focus TH: 0 occurrence(s)"));
    assert!(text.contains("utterance score -"));
}

#[test]
fn json_output_has_measured_gop_and_null_scores() {
    let dir = Workdir::new("json");
    let lex = write_lexicon(&dir);
    let vocab = write_vocab(&dir);
    let post = write_cat_posteriors(&dir);
    let out = pron(&[
        "score",
        "--text",
        "cat Jakarta",
        "--lexicon",
        s(&lex),
        "--vocab",
        s(&vocab),
        "--posteriors",
        s(&post),
        "--json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(json["experimental"], true);
    assert_eq!(json["outcome"]["status"], "aligned");
    assert_eq!(json["utterance_score"], serde_json::Value::Null);
    assert_eq!(json["words"][0]["kind"], "scored");
    assert_eq!(json["words"][1]["kind"], "not_checked");
    assert_eq!(json["words"][1]["reason"], "not_in_lexicon");
    let phones = json["words"][0]["phonemes"].as_array().expect("phonemes");
    assert_eq!(phones.len(), 3);
    for p in phones {
        assert!(p["gop"].as_f64().expect("gop") <= 0.0);
        assert_eq!(p["score"], serde_json::Value::Null);
        assert_eq!(p["flagged"], serde_json::Value::Null);
    }
}

#[test]
fn a_calibration_file_turns_scores_on_and_says_it_is_not_validated() {
    let dir = Workdir::new("calibration");
    let lex = write_lexicon(&dir);
    let vocab = write_vocab(&dir);
    let post = write_cat_posteriors(&dir);
    let cal = dir.path("calibration.toml");
    // Test-only numbers: the file exists to exercise the loader.
    std::fs::write(
        &cal,
        "[curve]\nmidpoint = -3.0\nslope = 1.0\n[thresholds]\nvowel = -3.0\nstop = -3.0\n",
    )
    .expect("calibration");
    let out = pron(&[
        "score",
        "--text",
        "cat",
        "--lexicon",
        s(&lex),
        "--vocab",
        s(&vocab),
        "--posteriors",
        s(&post),
        "--calibration",
        s(&cal),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(text.contains("Calibration: loaded, NOT validated."));
    assert!(text.contains("ok"));
}

#[test]
fn a_rejected_free_speech_transcript_prints_not_scored() {
    let dir = Workdir::new("rejected");
    let lex = write_lexicon(&dir);
    let vocab = write_vocab(&dir);
    let post = write_cat_posteriors(&dir);
    let out = pron(&[
        "score",
        "--text",
        "cat",
        "--lexicon",
        s(&lex),
        "--vocab",
        s(&vocab),
        "--posteriors",
        s(&post),
        "--free-speech",
        "--transcript-rejected",
    ]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(text.contains("NOT SCORED: TranscriptRejected"));
    assert!(text.contains("cat: not scored"));
}

#[test]
fn bad_inputs_fail_with_a_message_not_a_panic() {
    let dir = Workdir::new("errors");
    let lex = write_lexicon(&dir);
    let vocab = write_vocab(&dir);
    let post = write_cat_posteriors(&dir);
    let missing = dir.path("nope.dict");
    let out = pron(&[
        "score",
        "--text",
        "cat",
        "--lexicon",
        s(&missing),
        "--vocab",
        s(&vocab),
        "--posteriors",
        s(&post),
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read the lexicon file"));

    let out = pron(&[
        "score",
        "--text",
        "cat",
        "--lexicon",
        s(&lex),
        "--vocab",
        s(&vocab),
        "--posteriors",
        s(&post),
        "--focus",
        "XX",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not an ARPAbet symbol"));
}

#[cfg(not(feature = "ort-backend"))]
#[test]
fn a_wav_file_without_the_ort_backend_is_refused_honestly() {
    let dir = Workdir::new("nowav");
    let lex = write_lexicon(&dir);
    let vocab = write_vocab(&dir);
    let out = pron(&[
        "score",
        "--text",
        "cat",
        "--lexicon",
        s(&lex),
        "--vocab",
        s(&vocab),
        "speech.wav",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ort-backend"));
}

fn write_profile(dir: &Workdir, measurement: Measurement) -> PathBuf {
    let path = dir.path("speed.json");
    SpeedProfile {
        measurement,
        fixture_audio_seconds: 4.0,
        warmup_runs: 1,
        timed_runs: 5,
        label: "TEST".to_owned(),
        measured_at_unix: None,
    }
    .save(&path)
    .expect("profile");
    path
}

#[test]
fn mode_shows_the_decision_for_a_stored_profile_and_for_no_profile() {
    let dir = Workdir::new("mode");
    let fast = write_profile(
        &dir,
        Measurement {
            stt_alone: 0.0625,
            stt_together: 0.25,
            model_alone: 0.03125,
            model_together: 0.125,
            align: 0.0078125,
        },
    );
    let out = pron(&[
        "mode",
        "--profile",
        s(&fast),
        "--max-added-wait-ms",
        "500",
        "--utterance-seconds",
        "8",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        text.contains("pronunciation timing: blocking (automatic;"),
        "{text}"
    );

    let slow = write_profile(
        &dir,
        Measurement {
            stt_alone: 0.0625,
            stt_together: 0.125,
            model_alone: 0.5,
            model_together: 1.0,
            align: 0.0625,
        },
    );
    let out = pron(&[
        "mode",
        "--profile",
        s(&slow),
        "--max-added-wait-ms",
        "500",
        "--utterance-seconds",
        "8",
    ]);
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        text.contains("pronunciation timing: deferred (automatic;"),
        "{text}"
    );

    let out = pron(&[
        "mode",
        "--profile",
        s(&slow),
        "--max-added-wait-ms",
        "500",
        "--setting",
        "blocking",
    ]);
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(text.contains("blocking (set by the setting;"), "{text}");

    let out = pron(&["mode", "--max-added-wait-ms", "500"]);
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        text.contains("deferred (automatic, not measured yet)"),
        "{text}"
    );
}
