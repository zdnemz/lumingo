#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The diagnostics report.

mod common;

use common::fake_provider::FakeProvider;
use common::seed::test_core_with_unit;
use common::{fixed_hardware, test_core};
use storage::{LlmCallType, LlmOutcome, NewLlmCall, NewPerfSample, Timestamp};

#[tokio::test]
async fn a_fresh_install_reports_facts_and_no_made_up_statistics() {
    let t = test_core().await;
    let report = t.core.diagnostics().await.unwrap();
    assert_eq!(report.server_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(report.hardware, fixed_hardware());
    assert_eq!(report.schema_version, storage::SCHEMA_VERSION);
    assert_eq!(
        report.data_dir,
        t.core.config().data_dir.display().to_string()
    );
    assert_eq!(report.log_folder, None);
    assert!(report.provider.is_none());
    assert_eq!(report.curriculum.unit_count, 0);
    assert_eq!(report.curriculum.issue_count, 1, "the missing unit folder");
    assert_eq!(report.latency.len(), 5);
    for stat in &report.latency {
        assert_eq!(
            (stat.stats.count, stat.stats.p50_ms, stat.stats.p95_ms),
            (0, None, None)
        );
    }
    assert_eq!(report.llm.calls, 0);
    assert_eq!(report.llm.ttft.p50_ms, None);
}

#[tokio::test]
async fn latency_and_provider_call_statistics_come_from_the_stored_rows() {
    let t = test_core_with_unit().await;
    let db = t.core.database();
    let at = t.clock_now();
    for value in [100.0, 200.0, 300.0, 400.0] {
        db.diagnostics()
            .record_perf_sample(&NewPerfSample {
                session_id: None,
                turn_seq: None,
                metric: "stt_ms".to_owned(),
                value_ms: value,
                profile_tag: "dev".to_owned(),
                created_at: at,
            })
            .await
            .unwrap();
    }
    // Older than the 30-day window.
    db.diagnostics()
        .record_perf_sample(&NewPerfSample {
            session_id: None,
            turn_seq: None,
            metric: "stt_ms".to_owned(),
            value_ms: 99_999.0,
            profile_tag: "dev".to_owned(),
            created_at: Timestamp::parse("2026-01-01T00:00:00.000Z").unwrap(),
        })
        .await
        .unwrap();
    for (outcome, ttft, total) in [
        (LlmOutcome::Ok, Some(500), Some(1_000)),
        (LlmOutcome::Repaired, Some(700), Some(2_000)),
        (LlmOutcome::Timeout, None, Some(30_000)),
    ] {
        db.diagnostics()
            .record_llm_call(&NewLlmCall {
                provider_profile_id: None,
                call_type: LlmCallType::TutorTurn,
                model: "gpt-test".to_owned(),
                ladder_level: None,
                ttft_ms: ttft,
                total_ms: total,
                input_tokens: None,
                output_tokens: None,
                http_status: None,
                outcome,
                started_at: at,
            })
            .await
            .unwrap();
    }

    let report = t.core.diagnostics().await.unwrap();
    let stt = report
        .latency
        .iter()
        .find(|s| s.metric == "stt_ms")
        .unwrap();
    assert_eq!(stt.stats.count, 4);
    assert_eq!(stt.stats.p50_ms, Some(200.0));
    assert_eq!(stt.stats.p95_ms, Some(400.0));
    assert_eq!(report.llm.calls, 3);
    assert_eq!(report.llm.failures, 1);
    assert_eq!(report.llm.ttft.count, 2);
    assert_eq!(report.llm.ttft.p50_ms, Some(500.0));
    assert_eq!(report.llm.total.count, 3);
    assert_eq!(report.llm.total.p95_ms, Some(30_000.0));
    assert_eq!(report.curriculum.unit_count, 1);
    assert_eq!(report.curriculum.issue_count, 0);
    assert!(report.curriculum.content_version.is_some());
}

#[tokio::test]
async fn the_structured_output_level_of_the_last_test_is_shown() {
    let key = "sk-test-SECRETMATERIAL-9f8e7d6c";
    let provider = FakeProvider::start(key).await;
    let t = test_core().await;
    let request = serde_json::from_value(serde_json::json!({
        "name": "local", "protocol": "openai_chat", "base_url": provider.base_url,
        "model": "gpt-test", "api_key": key
    }))
    .unwrap();
    let info = t.core.save_provider(request).await.unwrap();
    t.core.test_provider(info.id).await.unwrap();

    let report = t.core.diagnostics().await.unwrap();
    let active = report.provider.as_ref().expect("active provider");
    assert_eq!(
        active.capabilities.as_ref().unwrap().structured_level,
        Some(1)
    );
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("SECRETMATERIAL")
    );
}
