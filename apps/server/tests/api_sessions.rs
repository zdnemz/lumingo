#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! One happy path for every new route group, over fakes, and the honest answers
//! of a server that has no audio, no speech and no models.

mod common;

use std::time::Duration;

use app_core::api::ServerEvent;
use app_core::voice::testing::{ReplyScript, Step};
use axum::http::Method;
use common::{Harness, MANIFEST, Options, Req, Sessions, harness_with};
use serde_json::{Value, json};

fn reply(text: &str) -> Step {
    Step::Reply(ReplyScript::new(&[text]))
}

fn chat() -> Value {
    json!({"kind": "text_chat", "topic": {"kind": "typed", "text": "food"}})
}

/// A server with the example unit, the fake session manager, and fake audio and
/// speech when `audio` is set.
async fn server(steps: Vec<Step>, audio: bool) -> Harness {
    harness_with(Options {
        with_unit: true,
        sessions: Some(Sessions {
            steps,
            transcripts: vec!["my name is Dewi"],
            audio,
        }),
        manifest: Some(MANIFEST.to_owned()),
        ..Options::default()
    })
    .await
}

async fn wait_for(_h: &Harness, what: &str, check: impl Fn() -> bool) {
    for _ in 0..1_000 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_routes_start_pause_resume_and_stop_a_session() {
    let h = server(vec![reply("Hello.")], false).await;
    let (status, view) = h.call(Req::post("/api/sessions", chat())).await;
    assert_eq!(status, 200, "{view}");
    let id = view["id"].as_i64().unwrap();
    wait_for(&h, "the opening", || {
        h.core
            .snapshot()
            .active_session
            .is_some_and(|s| s.turns_completed == 1)
    })
    .await;
    let (_, state) = h.get_json("/api/state").await;
    assert_eq!(state["active_session"]["id"], id);
    assert_eq!(state["active_session"]["recent"][0]["text"], "Hello.");

    let (status, paused) = h
        .call(Req::post(&format!("/api/sessions/{id}/pause"), json!({})))
        .await;
    assert_eq!((status, paused["life"].as_str()), (200, Some("paused")));
    let (status, resumed) = h
        .call(Req::post(&format!("/api/sessions/{id}/resume"), json!({})))
        .await;
    assert_eq!((status, resumed["life"].as_str()), (200, Some("active")));

    // The body of a stop may be left out; with one, it can cancel.
    let (status, ended) = h
        .call(Req::post(
            &format!("/api/sessions/{id}/stop"),
            json!({"cancel": true}),
        ))
        .await;
    assert_eq!(status, 200, "{ended}");
    assert_eq!(ended["status"], "aborted");
    assert!(ended["feedback"].is_null());
    let (_, state) = h.get_json("/api/state").await;
    assert!(state["active_session"].is_null());

    // Unknown and malformed.
    let (status, body) = h
        .call(Req::post("/api/sessions/4242/stop", json!({})))
        .await;
    assert_eq!((status, body["error"].as_str()), (404, Some("not_found")));
    let (status, body) = h
        .call(Req::post("/api/sessions/abc/pause", json!({})))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_input"))
    );
    let (status, body) = h
        .call(Req::post("/api/sessions", json!({"kind": "no_such_kind"})))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_input"))
    );
    let (status, body) = h
        .call(Req::post("/api/sessions", json!({"kind": "text_chat"})))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid_input"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_turn_routes_take_text_and_say_what_a_text_chat_cannot_do() {
    let h = server(vec![reply("Hello."), reply("Nice.")], false).await;
    let (_, view) = h.call(Req::post("/api/sessions", chat())).await;
    let id = view["id"].as_i64().unwrap();
    wait_for(&h, "the opening", || {
        h.core
            .snapshot()
            .active_session
            .is_some_and(|s| s.turns_completed == 1)
    })
    .await;
    let (status, accepted) = h
        .call(Req::post(
            &format!("/api/sessions/{id}/text"),
            json!({"text": "I am Dewi."}),
        ))
        .await;
    assert_eq!((status, accepted["turn"].as_u64()), (200, Some(2)));
    for bad in [
        json!({"text": ""}),
        json!({"text": "x".repeat(3000)}),
        json!({}),
    ] {
        let (status, body) = h
            .call(Req::post(&format!("/api/sessions/{id}/text"), bad))
            .await;
        assert!(status == 400 || status == 409, "{status} {body}");
    }
    // Editing and push-to-talk belong to a voice conversation.
    let (status, body) = h
        .call(Req::post(
            &format!("/api/sessions/{id}/turns/2/edit"),
            json!({"text": "I am Dewi Lestari."}),
        ))
        .await;
    assert_eq!((status, body["error"].as_str()), (409, Some("conflict")));
    let (status, _) = h
        .call(Req::post(
            &format!("/api/sessions/{id}/turns/not-a-number/edit"),
            json!({"text": "x"}),
        ))
        .await;
    assert_eq!(status, 400);
    let (status, _) = h
        .call(Req::post(
            &format!("/api/sessions/{id}/push-to-talk"),
            json!({"pressed": true}),
        ))
        .await;
    assert_eq!(status, 409);
    // With nothing speaking, stopping the speech is an answer, not an error.
    let (status, ack) = h
        .call(Req::post("/api/tutor/stop-speaking", json!({})))
        .await;
    assert_eq!((status, ack["ok"].as_bool()), (200, Some(true)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_voice_conversation_takes_a_corrected_transcript_and_push_to_talk_over_http() {
    let h = server(vec![reply("Hello."), reply("Nice to meet you.")], true).await;
    let (status, view) = h
        .call(Req::post(
            "/api/sessions",
            json!({"kind": "conversation", "topic": {"kind": "typed", "text": "food"}}),
        ))
        .await;
    assert_eq!(status, 200, "{view}");
    let id = view["id"].as_i64().unwrap();
    assert_eq!(view["channel"], "voice");
    wait_for(&h, "the opening", || {
        h.core
            .snapshot()
            .active_session
            .is_some_and(|s| s.turns_completed == 1)
    })
    .await;
    let (status, ack) = h
        .call(Req::post(
            &format!("/api/sessions/{id}/push-to-talk"),
            json!({"pressed": false}),
        ))
        .await;
    assert_eq!((status, ack["ok"].as_bool()), (200, Some(true)));

    // A spoken turn: a tone on the fake microphone, then silence.
    let backend = h
        .attached
        .as_ref()
        .and_then(|a| a.fake.as_ref())
        .map(|f| f.audio.backend.inner.clone())
        .unwrap();
    backend.set_input_tone(Some(440.0));
    tokio::time::sleep(Duration::from_millis(800)).await;
    backend.set_input_tone(None);
    wait_for(&h, "the spoken turn to be stored", || {
        h.core.snapshot().active_session.is_some_and(|s| {
            s.recent
                .iter()
                .any(|l| l.role == app_core::api::LineRole::Learner && l.seq.is_some())
        })
    })
    .await;
    let snapshot = h.core.snapshot().active_session.unwrap();
    let line = snapshot
        .recent
        .iter()
        .find(|l| l.role == app_core::api::LineRole::Learner)
        .unwrap();
    let (status, edited) = h
        .call(Req::post(
            &format!("/api/sessions/{id}/turns/{}/edit", line.turn),
            json!({"text": "My name is Dewi."}),
        ))
        .await;
    assert_eq!(status, 200, "{edited}");
    assert_eq!(edited["turn"], line.turn);
    assert!(
        ["stored_only", "analysed_from_edit"].contains(&edited["effect"].as_str().unwrap()),
        "{edited}"
    );
    let (status, ended) = h
        .call(Req::post(&format!("/api/sessions/{id}/stop"), json!({})))
        .await;
    assert_eq!((status, ended["status"].as_str()), (200, Some("completed")));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_activity_routes_offer_an_activity_and_score_the_answer() {
    let h = server(vec![], false).await;
    let (status, view) = h
        .call(Req::post(
            "/api/sessions",
            json!({"kind": "lesson", "unit_id": "a1-u01"}),
        ))
        .await;
    assert_eq!(status, 200, "{view}");
    let id = view["id"].as_i64().unwrap();
    let (status, next) = h
        .get_json(&format!("/api/sessions/{id}/next-activity"))
        .await;
    assert_eq!(status, 200, "{next}");
    assert_eq!(next["activity"]["id"], "a02-greeting-by-time");
    assert!(
        !next.to_string().contains("answer_index"),
        "no answer is sent"
    );
    assert!(next["unavailable"].as_array().unwrap().len() >= 5);

    let (status, done) = h
        .call(Req::post(
            "/api/activities/submit",
            json!({"session_id": id, "activity_id": "a02-greeting-by-time",
                   "answer": {"kind": "choice", "index": 1}}),
        ))
        .await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["result"]["score"], 1.0);

    // An activity that needs speech output says so with a typed 501.
    let (status, body) = h
        .call(Req::post(
            "/api/activities/submit",
            json!({"session_id": id, "activity_id": "a01-listen-question",
                   "answer": {"kind": "choice", "index": 0}}),
        ))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (501, Some("not_available"))
    );
    assert_eq!(body["feature"], "speech");
    let (status, _) = h.get_json("/api/sessions/4242/next-activity").await;
    assert_eq!(status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_free_mode_routes_take_a_draft_and_a_reading_text() {
    let h = server(vec![], false).await;
    let (status, view) = h
        .call(Req::post(
            "/api/sessions",
            json!({"kind": "writing", "topic": {"kind": "typed", "text": "your morning"}}),
        ))
        .await;
    assert_eq!(status, 200, "{view}");
    let id = view["id"].as_i64().unwrap();
    let (status, draft) = h
        .call(Req::post(
            &format!("/api/writing/{id}/drafts"),
            json!({"text": "I am Dewi. I wake up at six."}),
        ))
        .await;
    assert_eq!(status, 200, "{draft}");
    assert_eq!(draft["words"], 8);
    let (status, _) = h
        .call(Req::post(
            &format!("/api/writing/{id}/drafts"),
            json!({"text": " "}),
        ))
        .await;
    assert_eq!(status, 400);
    h.call(Req::post(
        &format!("/api/sessions/{id}/stop"),
        json!({"cancel": true}),
    ))
    .await;

    let (_, view) = h
        .call(Req::post("/api/sessions", json!({"kind": "reading"})))
        .await;
    let id = view["id"].as_i64().unwrap();
    // The scripted model has no text for this, so the authored sets come back.
    let (status, outcome) = h
        .call(Req::post(
            "/api/reading/generate",
            json!({"session_id": id, "topic": {"kind": "typed", "text": "the market"}}),
        ))
        .await;
    assert_eq!(status, 200, "{outcome}");
    assert_eq!(outcome["kind"], "fallback");
    let set = &outcome["sets"][0];
    let questions = set["text"]["questions"].as_array().unwrap().len();
    let (status, answered) = h
        .call(Req::post(
            &format!("/api/reading/{id}/answers"),
            json!({"target": {"kind": "authored", "unit_id": set["unit_id"],
                              "activity_id": set["activity_id"]},
                   "answers": vec![Value::Null; questions]}),
        ))
        .await;
    assert_eq!(status, 200, "{answered}");
    assert_eq!(answered["score"]["correct"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_speech_routes_list_devices_speak_a_stored_text_and_run_the_microphone_test() {
    let h = server(vec![reply("Good day. What is your name?")], true).await;
    let (status, devices) = h.get_json("/api/audio/devices").await;
    assert_eq!(status, 200, "{devices}");
    assert_eq!(devices["inputs"][0]["name"], "Fake microphone");
    assert_eq!(devices["outputs"][0]["name"], "Fake speaker");

    let (_, view) = h.call(Req::post("/api/sessions", chat())).await;
    let id = view["id"].as_i64().unwrap();
    wait_for(&h, "the opening", || {
        h.core
            .snapshot()
            .active_session
            .is_some_and(|s| s.turns_completed == 1)
    })
    .await;
    let seq = h
        .core
        .snapshot()
        .active_session
        .unwrap()
        .recent
        .iter()
        .find(|l| l.role == app_core::api::LineRole::Tutor)
        .and_then(|l| l.seq)
        .unwrap();

    // By reference to the stored turn. Text in the request is not read.
    let (status, spoken) = h
        .call(Req::post(
            "/api/tts/speak",
            json!({"source": "turn", "session_id": id, "turn_seq": seq,
                   "text": "Something the server never stored."}),
        ))
        .await;
    assert_eq!(status, 200, "{spoken}");
    assert_eq!(spoken["sentences"], 2);
    let fake = h.attached.as_ref().unwrap().fake.as_ref().unwrap();
    wait_for(&h, "the speech", || {
        fake.speech.tts_log.spoken.lock().unwrap().len() == 2
    })
    .await;
    let said = fake.speech.tts_log.spoken.lock().unwrap().clone();
    assert_eq!(said, ["Good day.", "What is your name?"]);
    assert!(
        !said.iter().any(|s| s.contains("never stored")),
        "request text was spoken"
    );

    // Text alone is not a request, and a reference to nothing stored is a 404.
    for body in [
        json!({"text": "Say this."}),
        json!({"source": "text", "text": "Say this."}),
    ] {
        let (status, body) = h.call(Req::post("/api/tts/speak", body)).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (400, Some("invalid_input"))
        );
    }
    let (status, body) = h
        .call(Req::post(
            "/api/tts/speak",
            json!({"source": "turn", "session_id": id, "turn_seq": 4242}),
        ))
        .await;
    assert_eq!((status, body["error"].as_str()), (404, Some("not_found")));
    assert_eq!(
        fake.speech.tts_log.spoken.lock().unwrap().len(),
        2,
        "nothing more was spoken"
    );

    let (status, ack) = h
        .call(Req::post("/api/tutor/stop-speaking", json!({})))
        .await;
    assert_eq!((status, ack["ok"].as_bool()), (200, Some(true)));

    h.call(Req::post(
        &format!("/api/sessions/{id}/stop"),
        json!({"cancel": true}),
    ))
    .await;
    let (status, report) = h
        .call(Req::post("/api/audio/test", json!({"duration_ms": 500})))
        .await;
    assert_eq!(status, 200, "{report}");
    assert_eq!(report["input_device"], "Fake microphone");
    assert_eq!(report["silent"], true);
    let (status, _) = h
        .call(Req::post("/api/audio/test", json!({"duration_ms": 100})))
        .await;
    assert_eq!(status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_model_routes_list_the_manifest_and_refuse_a_download_without_the_licence() {
    let h = server(vec![], false).await;
    let (status, list) = h.get_json("/api/models").await;
    assert_eq!(status, 200, "{list}");
    let models = list["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["downloadable"]["state"], "yes");
    assert_eq!(models[1]["downloadable"]["state"], "no");
    assert_eq!(models[0]["licence"]["license"], "MIT");

    let licence = json!({"accept_licence": true, "license": "MIT",
                         "license_url": "https://example.org/licence"});
    // Not accepted, or not the licence that was shown.
    for body in [
        json!({"accept_licence": false, "license": "MIT",
               "license_url": "https://example.org/licence"}),
        json!({"accept_licence": true, "license": "GPL-3.0",
               "license_url": "https://example.org/licence"}),
    ] {
        let (status, body) = h
            .call(Req::post("/api/models/stt-test/download", body))
            .await;
        assert_eq!(
            (status, body["error"].as_str()),
            (409, Some("licence_not_accepted"))
        );
    }
    // An empty checksum is refused whatever the licence says.
    let (status, body) = h
        .call(Req::post(
            "/api/models/stt-candidate/download",
            licence.clone(),
        ))
        .await;
    assert_eq!((status, body["error"].as_str()), (409, Some("conflict")));
    assert!(
        body["message"].as_str().unwrap().contains("checksum"),
        "{body}"
    );
    let (status, body) = h
        .call(Req::post(
            "/api/models/no-such-model/download",
            licence.clone(),
        ))
        .await;
    assert_eq!((status, body["error"].as_str()), (404, Some("not_found")));
    let (status, _) = h
        .call(Req::post("/api/models/stt-test/cancel", json!({})))
        .await;
    assert_eq!(status, 404, "nothing is downloading");

    // With the licence, the download starts and its end arrives as an event. The
    // host of this manifest does not answer, so it ends failed, with a sentence
    // that holds no address.
    let mut rx = h.core.events().subscribe();
    let (status, started) = h
        .call(Req::post("/api/models/stt-test/download", licence))
        .await;
    assert_eq!(status, 200, "{started}");
    assert_eq!(started["model_id"], "stt-test");
    let ended = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(ServerEvent::DownloadProgress { state, message, .. }) = rx.recv().await
                && state != app_core::api::DownloadState::Running
            {
                return (state, message);
            }
        }
    })
    .await
    .expect("the download ends");
    assert_eq!(ended.0, app_core::api::DownloadState::Failed);
    assert!(!ended.1.unwrap().contains("127.0.0.1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_audio_speech_or_a_manifest_the_routes_answer_a_typed_not_available() {
    let h = harness_with(Options {
        with_unit: true,
        sessions: Some(Sessions::text_only(vec![])),
        ..Options::default()
    })
    .await;
    let (_, state) = h.get_json("/api/state").await;
    assert_eq!(state["unavailable"], json!(["speech", "models"]));
    assert!(
        state["engines"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["state"] == "unavailable" && !e["detail"].as_str().unwrap().is_empty())
    );

    for (method, uri, body, feature) in [
        (Method::GET, "/api/audio/devices", None, Some("speech")),
        (
            Method::POST,
            "/api/audio/test",
            Some(json!({"duration_ms": 500})),
            Some("speech"),
        ),
        (Method::GET, "/api/models", None, Some("models")),
        (
            Method::POST,
            "/api/sessions",
            Some(json!({"kind": "conversation", "topic": {"kind": "typed", "text": "food"}})),
            None,
        ),
        (
            Method::POST,
            "/api/sessions",
            Some(json!({"kind": "review"})),
            None,
        ),
        (
            Method::POST,
            "/api/sessions",
            Some(json!({"kind": "placement"})),
            None,
        ),
    ] {
        let (status, response) = h.call(Req::json(method, uri, body)).await;
        assert_eq!(status, 501, "{uri}: {response}");
        assert_eq!(response["error"], "not_available", "{uri}");
        assert!(!response["message"].as_str().unwrap().is_empty());
        if let Some(feature) = feature {
            assert_eq!(response["feature"], feature, "{uri}");
        }
    }
    // Text chat, lessons and the inspector work all the same.
    let (status, _) = h.call(Req::post("/api/sessions", chat())).await;
    assert_eq!(status, 200);
    let (status, inspector) = h.get_json("/api/inspector").await;
    assert_eq!(status, 200);
    assert_eq!(inspector["capacity"], 50);
    assert_eq!(inspector["entries"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_session_manager_every_session_route_says_sessions_are_not_available() {
    let h = harness_with(Options::default()).await;
    let (_, state) = h.get_json("/api/state").await;
    assert!(
        state["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "sessions")
    );
    let (status, body) = h.call(Req::post("/api/sessions", chat())).await;
    assert_eq!((status, body["feature"].as_str()), (501, Some("sessions")));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_key_is_in_no_answer_no_event_and_no_inspector_entry() {
    const KEY: &str = "sk-server-SECRETMATERIAL-4d3c2b1a";
    let h = server(vec![reply("Hello."), reply("Good.")], true).await;
    let mut rx = h.core.events().subscribe();
    let (status, saved) = h
        .call(Req::post(
            "/api/providers",
            json!({"name": "mine", "protocol": "openai_chat",
                   "base_url": "https://api.example.test/v1", "model": "m", "api_key": KEY}),
        ))
        .await;
    assert_eq!(status, 200, "{saved}");
    let (_, view) = h.call(Req::post("/api/sessions", chat())).await;
    let id = view["id"].as_i64().unwrap();
    wait_for(&h, "the opening", || {
        h.core
            .snapshot()
            .active_session
            .is_some_and(|s| s.turns_completed == 1)
    })
    .await;
    h.call(Req::post(
        &format!("/api/sessions/{id}/text"),
        json!({"text": "I am Dewi."}),
    ))
    .await;
    wait_for(&h, "the second turn", || {
        h.core
            .snapshot()
            .active_session
            .is_some_and(|s| s.turns_completed == 2)
    })
    .await;
    h.call(Req::post(&format!("/api/sessions/{id}/stop"), json!({})))
        .await;

    let mut said = String::new();
    for uri in [
        "/api/state",
        "/api/providers",
        "/api/inspector",
        "/api/diagnostics",
        "/api/export",
        "/api/models",
        "/api/audio/devices",
        "/api/progress",
        "/api/settings",
        "/api/units",
    ] {
        let (_, body) = h.get_json(uri).await;
        said.push_str(&body.to_string());
    }
    while let Ok(event) = rx.try_recv() {
        said.push_str(&serde_json::to_string(&event).unwrap());
    }
    assert!(
        !said.contains("SECRETMATERIAL"),
        "the key is in an answer or an event"
    );
    assert!(said.contains("1a"), "the last four characters are shown");
}
