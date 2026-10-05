#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The Progress, Game, Data and Diagnostics route groups.

mod common;

use app_core::api::XpSourceKind;
use axum::http::header;
use common::{Harness, Options, Req, body_json, harness, harness_with, send};
use serde_json::{Value, json};
use storage::{
    AttemptOrigin, AttemptStatus, EvidenceKind, InputMode, Level, NewAttempt, NewEvidence,
    NewSession, NewTurn, Scorer, SessionKind, TurnRole, UnitProgress, UnitStatus,
};

const LEARNER_TEXT: &str = "ZEBRAQUILL writes a sentence";

async fn harness_with_unit() -> Harness {
    harness_with(Options {
        with_unit: true,
        ..Options::default()
    })
    .await
}

/// A session with a recorded turn and one scored attempt that has evidence.
/// Returns (session id, attempt id, recording path).
async fn seed(h: &Harness) -> (i64, i64, std::path::PathBuf) {
    let db = h.core.database();
    let at = h.core.clock().now();
    let profile = h.core.profile_id();
    let session = db
        .sessions()
        .create(&NewSession {
            profile_id: profile,
            kind: SessionKind::Writing,
            unit_id: Some("a1-u01".to_owned()),
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0".to_owned(),
            started_at: at,
        })
        .await
        .unwrap();
    let turn = db
        .turns()
        .append(&NewTurn {
            session_id: session.id,
            role: TurnRole::Learner,
            input_mode: InputMode::Voice,
            text: LEARNER_TEXT.to_owned(),
            stt_text: None,
            edited_by_learner: false,
            speech_ms: None,
            pause_ms: None,
            word_count: Some(4),
            created_at: at,
        })
        .await
        .unwrap();
    let relative = format!("audio/turn-{}.wav", turn.id);
    let file = h.core.config().data_dir.join(&relative);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"RIFF....WAVE").unwrap();
    db.audio_clips()
        .add(turn.id, &relative, 1000, &at)
        .await
        .unwrap();
    let attempt = db
        .attempts()
        .insert(&NewAttempt {
            profile_id: profile,
            session_id: Some(session.id),
            unit_id: Some("a1-u01".to_owned()),
            activity_id: "act-1".to_owned(),
            activity_type: "mcq".to_owned(),
            response_id: "resp-1".to_owned(),
            origin: AttemptOrigin::Authored,
            level: Level::A1,
            skill: "listening".to_owned(),
            dimension: "accuracy".to_owned(),
            scorer: Scorer::Deterministic,
            scorer_version: "det/1".to_owned(),
            raw_score: Some(1.0),
            max_score: Some(1.0),
            normalized: Some(1.0),
            confidence: Some(1.0),
            status: AttemptStatus::Scored,
            counts_toward_estimate: true,
            created_at: at,
        })
        .await
        .unwrap();
    db.evidence()
        .add(&NewEvidence {
            attempt_id: attempt.id,
            kind: EvidenceKind::ResponseText,
            content: Some(LEARNER_TEXT.to_owned()),
            data: None,
            created_at: at,
        })
        .await
        .unwrap();
    db.unit_progress()
        .set(&UnitProgress {
            profile_id: profile,
            unit_id: "a1-u01".to_owned(),
            status: UnitStatus::InProgress,
            best_checkpoint: None,
            updated_at: at,
        })
        .await
        .unwrap();
    (session.id, attempt.id, file)
}

#[tokio::test]
async fn progress_shows_stored_rows_and_the_evidence_behind_an_attempt() {
    let h = harness_with_unit().await;
    let (session, attempt, _) = seed(&h).await;

    let (status, progress) = h.get_json("/api/progress").await;
    assert_eq!(status, 200);
    assert_eq!(progress["units"][0]["unit_id"], "a1-u01");
    assert_eq!(progress["units"][0]["status"], "in_progress");
    assert_eq!(progress["recent_sessions"][0]["id"], session);
    assert_eq!(progress["recent_sessions"][0]["kind"], "writing");
    assert_eq!(progress["estimates"], json!([]));
    assert_eq!(progress["reviews_due_truncated"], false);

    let (status, evidence) = h
        .get_json(&format!("/api/attempts/{attempt}/evidence"))
        .await;
    assert_eq!(status, 200);
    assert_eq!(evidence["skill"], "listening");
    assert_eq!(evidence["evidence"][0]["kind"], "response_text");
    assert_eq!(evidence["evidence"][0]["content"], LEARNER_TEXT);

    let (status, error) = h.get_json("/api/attempts/99999/evidence").await;
    assert_eq!((status, error["error"].as_str()), (404, Some("not_found")));
    let (status, _) = h.get_json("/api/attempts/abc/evidence").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn game_state_is_read_and_cosmetics_are_equipped_but_xp_cannot_be_claimed() {
    let h = harness(false).await;
    let (status, game) = h.get_json("/api/game").await;
    assert_eq!(status, 200);
    assert_eq!(game["xp_total"], 0);
    assert_eq!(game["rank"]["rank"], 1);
    assert_eq!(game["equipped"]["accessory"], Value::Null);
    let cap = game["cosmetics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "accessory-cap")
        .unwrap();
    assert_eq!(cap["unlocked"], true);

    let (status, worn) = h
        .call(Req::post(
            "/api/game/equip",
            json!({"slot": "accessory", "id": "accessory-cap"}),
        ))
        .await;
    assert_eq!(status, 200);
    assert_eq!(worn["equipped"]["accessory"], "accessory-cap");

    let (status, error) = h
        .call(Req::post(
            "/api/game/equip",
            json!({"slot": "accessory", "id": "accessory-crown"}),
        ))
        .await;
    assert_eq!(
        (status, error["error"].as_str()),
        (400, Some("invalid_input"))
    );
    let (status, bare) = h
        .call(Req::post(
            "/api/game/equip",
            json!({"slot": "accessory", "id": null}),
        ))
        .await;
    assert_eq!(status, 200);
    assert_eq!(bare["equipped"]["accessory"], Value::Null);

    // XP comes from finished practice inside the core. No route awards it.
    for req in [
        Req::post("/api/game/xp", json!({"amount": 1000})),
        Req::post("/api/game/award", json!({"amount": 1000})),
        Req::put("/api/game", json!({"xp_total": 1000})),
    ] {
        let status = send(&h.router, req.authed(&h.cookie)).await.status();
        assert!(status == 404 || status == 405, "{status}");
    }
    h.core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    let (_, game) = h.get_json("/api/game").await;
    assert_eq!(game["xp_total"], 5);
    assert_eq!(game["streak"]["current"], 1);
}

#[tokio::test]
async fn a_session_is_deleted_with_its_text_and_recording_and_unknown_ones_are_404() {
    let h = harness_with_unit().await;
    let (session, attempt, recording) = seed(&h).await;
    assert!(recording.exists());

    let (status, result) = h
        .call(Req::delete(&format!("/api/sessions/{session}")))
        .await;
    assert_eq!(status, 200);
    assert_eq!(result["audio_files_removed"], 1);
    assert_eq!(result["compacted"], true);
    assert!(!recording.exists());
    let (_, progress) = h.get_json("/api/progress").await;
    assert_eq!(progress["recent_sessions"], json!([]));
    assert_eq!(
        progress["units"].as_array().unwrap().len(),
        1,
        "numbers stay"
    );
    let (_, evidence) = h
        .get_json(&format!("/api/attempts/{attempt}/evidence"))
        .await;
    assert_eq!(
        evidence["evidence"][0]["content"],
        Value::Null,
        "the text is gone"
    );

    let (status, error) = h
        .call(Req::delete(&format!("/api/sessions/{session}")))
        .await;
    assert_eq!((status, error["error"].as_str()), (404, Some("not_found")));
    let (status, _) = h.call(Req::delete("/api/sessions/abc")).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn delete_all_removes_the_learner_and_the_core_keeps_working() {
    let h = harness_with_unit().await;
    let (_, _, recording) = seed(&h).await;
    h.core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    let (status, result) = h.call(Req::delete("/api/data")).await;
    assert_eq!(status, 200);
    assert_eq!(result["audio_files_removed"], 1);
    assert!(!recording.exists());

    let (_, progress) = h.get_json("/api/progress").await;
    assert_eq!(progress["units"], json!([]));
    assert_eq!(progress["recent_sessions"], json!([]));
    let (_, game) = h.get_json("/api/game").await;
    assert_eq!(game["xp_total"], 0);
    let (_, export) = h.get_json("/api/export").await;
    assert_eq!(export["data"]["sessions"], json!([]));
    assert!(!export.to_string().contains(LEARNER_TEXT));
}

#[tokio::test]
async fn the_export_is_a_download_without_any_key() {
    let h = harness_with_unit().await;
    seed(&h).await;
    let request = Req::post(
        "/api/providers",
        json!({"name": "mine", "protocol": "openai_chat",
               "base_url": "https://api.example.test/v1", "model": "m",
               "api_key": "sk-export-SECRETKEYMATERIAL-Hq7Lp"}),
    );
    let (status, _) = h.call(request).await;
    assert_eq!(status, 200);

    let response = send(&h.router, Req::get("/api/export").authed(&h.cookie)).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"lumingo-export-2026-10-05.json\""
    );
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let bundle = body_json(response).await;
    let text = bundle.to_string();
    assert!(!text.contains("SECRETKEYMATERIAL") && !text.contains("q7Lp"));
    assert_eq!(bundle["format_version"], 1);
    assert_eq!(bundle["data"]["providers"][0]["name"], "mine");
    assert_eq!(
        bundle["data"]["sessions"][0]["turns"][0]["turn"]["text"],
        LEARNER_TEXT
    );
    assert_eq!(bundle["data"]["attempts"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn diagnostics_report_the_machine_the_files_and_the_server() {
    let h = harness_with_unit().await;
    let (status, report) = h.get_json("/api/diagnostics").await;
    assert_eq!(status, 200);
    assert_eq!(report["server_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["server_address"], common::OWN_ORIGIN);
    assert_eq!(report["schema_version"], storage::SCHEMA_VERSION);
    assert_eq!(report["hardware"]["ram_total_bytes"], 16_000_000_000_u64);
    assert_eq!(report["log_folder"], Value::Null);
    assert_eq!(report["curriculum"]["unit_count"], 1);
    assert_eq!(report["curriculum"]["issue_count"], 0);
    assert_eq!(report["latency"].as_array().unwrap().len(), 5);
    assert_eq!(report["latency"][0]["stats"]["count"], 0);
    assert_eq!(report["latency"][0]["stats"]["p50_ms"], Value::Null);
    assert_eq!(report["llm"]["calls"], 0);
    assert_eq!(report["provider"], Value::Null);
}
