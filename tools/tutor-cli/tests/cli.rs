#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The program as a user runs it: exit codes, result files and what it prints.
//! No hardware and no network: the provider is a local server, a closed port or
//! an address that routes nowhere, and the speech side is the fakes of the
//! `test-support` feature.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEY: &str = "sk-test-0123456789abcdef-KEYMATERIAL";

fn scenario() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../curriculum/examples/a1-u01.example.json")
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    took: Duration,
}

fn chunk(delta: &str, finish: Option<&str>) -> String {
    let finish = finish.map_or("null".to_owned(), |f| format!("\"{f}\""));
    format!(
        "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{{delta}}},\"finish_reason\":{finish}}}]}}\n\n"
    )
}

/// A provider on 127.0.0.1 that answers every request with one streamed reply.
struct Provider {
    base_url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Provider {
    fn start(reply: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let log = Arc::clone(&log);
                std::thread::spawn(move || serve(stream, reply, &log));
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            seen,
        }
    }

    fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(stream: TcpStream, reply: &str, log: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut length = 0_usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0_u8; length];
    let _ = reader.read_exact(&mut body);
    log.lock().unwrap().push(request_line.trim().to_owned());
    let payload = format!(
        "{}{}{}data: [DONE]\n\n",
        chunk("\"role\":\"assistant\",\"content\":\"\"", None),
        chunk(&format!("\"content\":{}", serde_json::json!(reply)), None),
        chunk("", Some("stop")),
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = reader.into_inner();
    let _ = stream.write_all(response.as_bytes());
}

fn tutor_cli(dir: &Path, base_url: Option<&str>, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tutor-cli"));
    command
        .current_dir(dir)
        .env_remove("RUST_LOG")
        .args(["chat", "--data-dir"])
        .arg(dir.join("data"))
        .args(["--scenario"])
        .arg(scenario())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "TUTOR_LLM_PROTOCOL",
        "TUTOR_LLM_BASE_URL",
        "TUTOR_LLM_MODEL",
        "TUTOR_LLM_API_KEY",
    ] {
        command.env_remove(name);
    }
    if let Some(url) = base_url {
        command
            .env("TUTOR_LLM_PROTOCOL", "openai_chat")
            .env("TUTOR_LLM_BASE_URL", url)
            .env("TUTOR_LLM_MODEL", "test-model")
            .env("TUTOR_LLM_API_KEY", KEY);
    }
    command
}

fn run(mut command: Command, limit: Duration) -> Run {
    let started = Instant::now();
    let mut child = command.spawn().expect("the program starts");
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let code = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status.code();
        }
        if started.elapsed() > limit {
            child.kill().ok();
            child.wait().ok();
            panic!("the program did not end within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Run {
        code,
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
        took: started.elapsed(),
    }
}

fn no_secret(run: &Run) {
    for text in [&run.stdout, &run.stderr] {
        assert!(!text.contains(KEY), "the key was printed:\n{text}");
        assert!(
            !text.contains("KEYMATERIAL"),
            "the key was printed:\n{text}"
        );
    }
}

// ---- offline (S3-12) ---------------------------------------------------------------

#[test]
fn a_closed_port_ends_with_exit_code_3_within_the_timeout_and_a_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/v1");
    let result = run(
        tutor_cli(
            dir.path(),
            Some(&url),
            &["--text", "--provider-timeout-ms", "1500"],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(result.code, Some(3), "{}\n{}", result.stdout, result.stderr);
    assert!(
        result.took < Duration::from_millis(1_500),
        "{:?}",
        result.took
    );
    assert!(
        result.stderr.contains("provider unavailable"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("stopped cleanly"),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains("panicked"), "{}", result.stderr);
    no_secret(&result);
}

#[test]
fn an_address_that_routes_nowhere_also_ends_with_exit_code_3_within_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(
        tutor_cli(
            dir.path(),
            Some("https://10.255.255.1:443/v1"),
            &["--text", "--provider-timeout-ms", "1500"],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(result.code, Some(3), "{}\n{}", result.stdout, result.stderr);
    // Whether the machine refuses at once or the connection hangs, the budget holds.
    assert!(
        result.took < Duration::from_millis(2_500),
        "{:?}",
        result.took
    );
    no_secret(&result);
}

#[test]
fn a_scripted_run_that_loses_the_provider_still_writes_its_result_file() {
    let dir = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let script = dir.path().join("script.txt");
    std::fs::write(&script, "Hello\nMy name is Dewi\n").unwrap();
    let out = dir.path().join("result.jsonl");
    let result = run(
        tutor_cli(
            dir.path(),
            Some(&format!("http://127.0.0.1:{port}/v1")),
            &[
                "--script",
                script.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
                "--provider-timeout-ms",
                "1500",
            ],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(result.code, Some(3), "{}", result.stderr);
    let text = std::fs::read_to_string(&out).expect("the result file exists");
    let last: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "summary");
    assert_eq!(last["ended"], "provider_unavailable");
    assert!(!text.contains(KEY));
}

// ---- configuration and the default build ---------------------------------------------

#[test]
fn without_a_provider_the_program_says_what_to_set_and_exits_with_1() {
    let dir = tempfile::tempdir().unwrap();
    let result = run(
        tutor_cli(dir.path(), None, &["--text"]),
        Duration::from_secs(20),
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("TUTOR_LLM_BASE_URL"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("providers.toml"),
        "{}",
        result.stderr
    );
}

#[test]
fn voice_without_the_real_backend_features_exits_with_4_and_names_the_feature() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Provider::start("Hello.");
    let result = run(
        tutor_cli(dir.path(), Some(&provider.base_url), &["--backend", "cpal"]),
        Duration::from_secs(20),
    );
    // The tests build this crate without `cpal-backend` and `sherpa`.
    assert_eq!(result.code, Some(4), "{}", result.stderr);
    assert!(result.stderr.contains("--features"), "{}", result.stderr);
    assert!(
        provider.requests().is_empty(),
        "no request before the speech side exists"
    );
}

#[test]
fn a_bad_end_silence_is_refused_before_anything_starts() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Provider::start("Hello.");
    let result = run(
        tutor_cli(
            dir.path(),
            Some(&provider.base_url),
            &["--text", "--end-silence-ms", "100"],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(provider.requests().is_empty());
}

// ---- scripted runs --------------------------------------------------------------------

fn read_lines(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .expect("the result file exists")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_typed_script_writes_one_line_per_turn_with_the_latency_parts_and_a_summary() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Provider::start("Hello there. How are you?");
    let script = dir.path().join("script.txt");
    std::fs::write(&script, "# two lines, repeated\nHello\nMy name is Dewi\n").unwrap();
    let out = dir.path().join("result.jsonl");
    let result = run(
        tutor_cli(
            dir.path(),
            Some(&provider.base_url),
            &[
                "--script",
                script.to_str().unwrap(),
                "--turns",
                "5",
                "--out",
                out.to_str().unwrap(),
                "--mode",
                "accuracy",
            ],
        ),
        Duration::from_secs(60),
    );
    assert_eq!(result.code, Some(0), "{}\n{}", result.stdout, result.stderr);
    no_secret(&result);

    // Each turn prints its latency parts.
    assert_eq!(result.stdout.matches("latency  endpointing").count(), 6);
    assert!(result.stdout.contains("tutor>  Hello there."));
    assert!(result.stdout.contains("you>    My name is Dewi"));

    let lines = read_lines(&out);
    // The opening turn, then five scripted turns.
    assert_eq!(lines.len(), 1 + 6 + 1);
    assert_eq!(lines[0]["kind"], "run");
    assert_eq!(lines[0]["input"], "text");
    assert_eq!(lines[0]["feedback_mode"], "accuracy");
    assert_eq!(lines[0]["scenario_unit"], "a1-u01");
    assert_eq!(lines[0]["provider_host"], "127.0.0.1");
    assert_eq!(lines[0]["turns_planned"], 5);
    let inputs: Vec<&str> = lines[1..7]
        .iter()
        .map(|l| l["input"].as_str().unwrap())
        .collect();
    assert_eq!(inputs, ["opening", "text", "text", "text", "text", "text"]);
    for turn in &lines[1..7] {
        assert_eq!(turn["kind"], "turn");
        assert_eq!(turn["outcome"], "replied");
        assert!(turn["llm_first_sentence_ms"].as_f64().unwrap() >= 0.0);
        // Typed text has no endpointing and no recogniser, and no speech output here.
        assert!(turn["endpointing_wait_ms"].is_null());
        assert!(turn["stt_finalise_ms"].is_null());
        assert_eq!(turn["complete"], false);
    }
    let summary = &lines[7];
    assert_eq!(summary["kind"], "summary");
    assert_eq!(summary["ended"], "completed");
    assert_eq!(summary["turns_ended"], 6);
    assert!(summary["latency"]["llm_first_sentence"]["p50_ms"].is_number());
    assert!(summary["latency"]["llm_first_sentence"]["p95_ms"].is_number());
    // Six requests reached the provider and nothing else was asked of it.
    assert_eq!(provider.requests().len(), 6);
    assert!(
        provider
            .requests()
            .iter()
            .all(|r| r.starts_with("POST /v1/chat/completions"))
    );
}

#[test]
fn recorded_utterances_run_through_the_vad_the_recogniser_and_speech_with_fake_engines() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Provider::start("Hello there.");
    // A recording: 700 ms of a loud tone, which the fake VAD takes for speech.
    let tone: Vec<f32> = (0..11_200)
        .map(|i| 0.4 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin())
        .collect();
    std::fs::write(
        dir.path().join("hello.wav"),
        tutor_cli::wav::encode_pcm16(&tone, 16_000),
    )
    .unwrap();
    let script = dir.path().join("script.txt");
    std::fs::write(&script, "wav: hello.wav\n").unwrap();
    let out = dir.path().join("result.jsonl");
    let result = run(
        tutor_cli(
            dir.path(),
            Some(&provider.base_url),
            &[
                "--backend",
                "fake",
                "--script",
                script.to_str().unwrap(),
                "--turns",
                "2",
                "--out",
                out.to_str().unwrap(),
                "--learner-first",
            ],
        ),
        Duration::from_secs(90),
    );
    assert_eq!(result.code, Some(0), "{}\n{}", result.stdout, result.stderr);
    assert!(
        result.stderr.contains("warning: fake devices"),
        "{}",
        result.stderr
    );

    let lines = read_lines(&out);
    assert_eq!(lines[0]["backend"], "fake");
    assert_eq!(lines[0]["input"], "audio");
    assert!(lines[0]["warning"].as_str().unwrap().contains("fake"));
    assert_eq!(lines[0]["stt"]["id"], "fake-stt");
    assert_eq!(lines[0]["tts"]["id"], "fake-tts");
    let turns: Vec<&serde_json::Value> = lines.iter().filter(|l| l["kind"] == "turn").collect();
    assert_eq!(turns.len(), 2);
    for turn in turns {
        assert_eq!(turn["input"], "audio");
        assert_eq!(turn["complete"], true, "{turn}");
        let parts = [
            "endpointing_wait_ms",
            "stt_finalise_ms",
            "llm_first_sentence_ms",
            "tts_first_sentence_ms",
            "output_start_ms",
        ]
        .map(|key| {
            turn[key]
                .as_f64()
                .unwrap_or_else(|| panic!("{key} missing in {turn}"))
        });
        let sum: f64 = parts.iter().sum();
        assert!((sum - turn["sum_ms"].as_f64().unwrap()).abs() < 0.01);
        // The silence was fed in real time, so the wait is the real 600 ms or a bit more.
        assert!(parts[0] >= 500.0, "{turn}");
    }
    let summary = lines.last().unwrap();
    assert_eq!(summary["latency"]["complete_turns"], 2);
    assert!(summary["latency"]["sum"]["p95_ms"].is_number());
    assert!(result.stdout.contains("you>    fake transcript 1"));
}

#[test]
fn a_script_that_needs_the_recogniser_cannot_be_combined_with_text() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Provider::start("Hello.");
    let script = dir.path().join("script.txt");
    std::fs::write(&script, "wav: nothing.wav\n").unwrap();
    let result = run(
        tutor_cli(
            dir.path(),
            Some(&provider.base_url),
            &["--text", "--script", script.to_str().unwrap()],
        ),
        Duration::from_secs(20),
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("drop --text"), "{}", result.stderr);
}
