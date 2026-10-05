#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{make_profile, make_session, make_turn, new_turn, temp_db, ts};
use serde_json::json;
use storage::{
    GeneratedKind, NewErrorEvent, NewGeneratedContent, SessionKind, SessionStatus, Severity,
    StorageError, TurnAnalysis, TurnRole,
};

#[tokio::test]
async fn session_round_trips_and_lists_newest_first() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let first = make_session(&db, profile.id, SessionKind::Lesson).await;
    let second = make_session(&db, profile.id, SessionKind::TextChat).await;

    assert_eq!(first.status, SessionStatus::Active);
    assert_eq!(
        db.sessions().get(first.id).await.expect("get"),
        Some(first.clone())
    );
    let listed = db
        .sessions()
        .list_for_profile(profile.id, 10)
        .await
        .expect("list");
    // Same started_at, so the newer id comes first.
    let ids: Vec<i64> = listed.iter().map(|s| s.id).collect();
    assert_eq!(ids, [second.id, first.id]);
}

#[tokio::test]
async fn finishing_a_session_records_the_outcome_once() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::Reading).await;
    let summary = json!({ "turns": 4, "completed_activities": 2 });

    db.sessions()
        .finish(
            session.id,
            SessionStatus::Completed,
            &ts(500),
            Some(&summary),
        )
        .await
        .expect("finish");
    let stored = db
        .sessions()
        .get(session.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(stored.status, SessionStatus::Completed);
    assert_eq!(stored.ended_at, Some(ts(500)));
    assert_eq!(stored.summary, Some(summary));

    let again = db
        .sessions()
        .finish(session.id, SessionStatus::Aborted, &ts(600), None)
        .await;
    assert!(matches!(again, Err(StorageError::Rule(_))));
    let missing = db
        .sessions()
        .finish(9_999, SessionStatus::Aborted, &ts(600), None)
        .await;
    assert!(matches!(missing, Err(StorageError::NotFound { .. })));
    let active = db
        .sessions()
        .finish(session.id, SessionStatus::Active, &ts(600), None)
        .await;
    assert!(matches!(active, Err(StorageError::Rule(_))));
}

#[tokio::test]
async fn a_session_for_a_missing_profile_is_refused_by_the_foreign_key() {
    let (_dir, db) = temp_db().await;
    let result = db
        .sessions()
        .create(&storage::NewSession {
            profile_id: 42,
            kind: SessionKind::Lesson,
            unit_id: None,
            activity_id: None,
            mode: None,
            provider_profile_id: None,
            app_version: "0.0.0-test".to_owned(),
            started_at: ts(1),
        })
        .await;
    assert!(matches!(result, Err(StorageError::Constraint(_))));
}

#[tokio::test]
async fn turn_numbers_count_from_one_inside_each_session() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let a = make_session(&db, profile.id, SessionKind::TextChat).await;
    let b = make_session(&db, profile.id, SessionKind::TextChat).await;

    let a1 = make_turn(&db, a.id, TurnRole::Learner, "Hello").await;
    let a2 = make_turn(&db, a.id, TurnRole::Tutor, "Hi!").await;
    let b1 = make_turn(&db, b.id, TurnRole::Learner, "Good morning").await;

    assert_eq!((a1.seq, a2.seq, b1.seq), (1, 2, 1));
    let listed = db.turns().list(a.id).await.expect("list");
    assert_eq!(listed, vec![a1, a2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_appends_never_share_a_sequence_number() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::TextChat).await;

    let mut tasks = Vec::new();
    for n in 0..16 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db.turns()
                .append(&new_turn(
                    session.id,
                    TurnRole::Learner,
                    &format!("line {n}"),
                    n,
                ))
                .await
                .expect("append")
                .seq
        }));
    }
    let mut seqs = Vec::new();
    for task in tasks {
        seqs.push(task.await.expect("join"));
    }
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=16).collect::<Vec<i64>>());
}

#[tokio::test]
async fn recent_returns_the_last_turns_oldest_first() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::TextChat).await;
    for n in 1..=5 {
        make_turn(&db, session.id, TurnRole::Learner, &format!("line {n}")).await;
    }
    let recent = db.turns().recent(session.id, 3).await.expect("recent");
    let texts: Vec<&str> = recent.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, ["line 3", "line 4", "line 5"]);
}

#[tokio::test]
async fn correcting_a_transcript_keeps_the_first_recogniser_output() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::Conversation).await;
    let turn = make_turn(&db, session.id, TurnRole::Learner, "I goed to market").await;

    db.turns()
        .correct_text(turn.id, "I went to market", Some(4))
        .await
        .expect("correct");
    db.turns()
        .correct_text(turn.id, "I went to the market", Some(5))
        .await
        .expect("correct again");

    let stored = db.turns().get(turn.id).await.expect("get").expect("exists");
    assert_eq!(stored.text, "I went to the market");
    assert_eq!(stored.stt_text.as_deref(), Some("I goed to market"));
    assert!(stored.edited_by_learner);
    assert_eq!(stored.word_count, Some(5));
}

#[tokio::test]
async fn analysis_and_error_events_are_stored_together_and_replaced_by_turn() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::TextChat).await;
    let turn = make_turn(&db, session.id, TurnRole::Learner, "She go home").await;

    let analysis = TurnAnalysis {
        turn_id: turn.id,
        analysis: json!({ "errors": [{ "category": "agreement" }] }),
        contract_version: "t2/1".to_owned(),
        ladder_level: 2,
        model: "test-model".to_owned(),
        created_at: ts(30),
    };
    let event = NewErrorEvent {
        turn_id: turn.id,
        profile_id: profile.id,
        category: "agreement".to_owned(),
        quote: "She go home".to_owned(),
        correction: "She goes home".to_owned(),
        severity: Severity::Minor,
        created_at: ts(30),
    };
    db.analysis()
        .store(&analysis, std::slice::from_ref(&event))
        .await
        .expect("store");

    assert_eq!(
        db.analysis().get(turn.id).await.expect("get"),
        Some(analysis.clone())
    );
    let events = db.analysis().error_events(turn.id).await.expect("events");
    assert_eq!(events.len(), 1);
    assert!(!events[0].addressed);

    db.analysis()
        .mark_addressed(events[0].id)
        .await
        .expect("addressed");
    let events = db.analysis().error_events(turn.id).await.expect("events");
    assert!(events[0].addressed);

    let revised = TurnAnalysis {
        ladder_level: 3,
        ..analysis
    };
    db.analysis()
        .store(&revised, &[])
        .await
        .expect("store again");
    assert_eq!(
        db.analysis()
            .get(turn.id)
            .await
            .expect("get")
            .map(|a| a.ladder_level),
        Some(3)
    );
}

#[tokio::test]
async fn analysis_with_a_ladder_level_outside_one_to_four_is_refused() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::TextChat).await;
    let turn = make_turn(&db, session.id, TurnRole::Learner, "Hello").await;
    let analysis = TurnAnalysis {
        turn_id: turn.id,
        analysis: json!({}),
        contract_version: "t2/1".to_owned(),
        ladder_level: 9,
        model: "test-model".to_owned(),
        created_at: ts(30),
    };
    let result = db.analysis().store(&analysis, &[]).await;
    assert!(matches!(result, Err(StorageError::Constraint(_))));
}

#[tokio::test]
async fn generated_content_and_recordings_belong_to_their_session() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::Reading).await;
    let turn = make_turn(&db, session.id, TurnRole::Learner, "Read aloud").await;

    let passage = db
        .generated_content()
        .add(&NewGeneratedContent {
            session_id: session.id,
            kind: GeneratedKind::ReadingPassage,
            content: json!({ "title": "A walk", "paragraphs": ["It was sunny."] }),
            contract_version: "r1/1".to_owned(),
            model: "test-model".to_owned(),
            created_at: ts(40),
        })
        .await
        .expect("add content");
    assert_eq!(
        db.generated_content()
            .for_session(session.id)
            .await
            .expect("list"),
        vec![passage]
    );

    db.audio_clips()
        .add(turn.id, "audio/one.wav", 1_500, &ts(41))
        .await
        .expect("clip");
    assert_eq!(
        db.sessions().audio_paths(session.id).await.expect("paths"),
        ["audio/one.wav"]
    );
    assert_eq!(
        db.audio_clips()
            .paths_for_profile(profile.id)
            .await
            .expect("paths"),
        ["audio/one.wav"]
    );
}
