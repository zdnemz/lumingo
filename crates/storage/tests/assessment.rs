#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{make_profile, new_attempt, temp_db, ts};
use serde_json::json;
use storage::{
    AttemptOrigin, AttemptStatus, EstimateLevel, EstimateStatus, EvidenceKind, NewEvidence,
    NewSkillEstimate, ScoreUpdate, StorageError, group_by_response,
};

const DIMENSIONS: [&str; 4] = ["task", "organisation", "vocabulary", "grammar"];

async fn insert_essay(db: &storage::Database, profile_id: i64, response_id: &str, at: i64) {
    let rows: Vec<_> = DIMENSIONS
        .iter()
        .map(|dimension| storage::NewAttempt {
            created_at: ts(at),
            ..new_attempt(profile_id, response_id, dimension)
        })
        .collect();
    db.attempts()
        .insert_response(&rows)
        .await
        .expect("insert essay");
}

#[tokio::test]
async fn dimension_rows_of_one_response_group_into_one_observation() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    insert_essay(&db, profile.id, "resp-essay-1", 100).await;
    insert_essay(&db, profile.id, "resp-essay-2", 200).await;

    let rows = db
        .attempts()
        .for_skill_since(profile.id, "writing", &ts(0))
        .await
        .expect("rows");
    // Without grouping, two essays on four dimensions look like eight pieces of evidence.
    assert_eq!(rows.len(), 8);
    let groups = group_by_response(rows);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].response_id, "resp-essay-1");
    assert_eq!(groups[1].response_id, "resp-essay-2");
    for group in &groups {
        let dimensions: Vec<&str> = group
            .attempts
            .iter()
            .map(|a| a.dimension.as_str())
            .collect();
        assert_eq!(dimensions, DIMENSIONS);
    }

    let one = db
        .attempts()
        .by_response("resp-essay-2")
        .await
        .expect("by response");
    assert_eq!(one.len(), 4);
    assert!(one.iter().all(|a| a.response_id == "resp-essay-2"));
}

#[tokio::test]
async fn a_response_is_inserted_whole_or_not_at_all() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let mut rows: Vec<_> = DIMENSIONS
        .iter()
        .map(|d| new_attempt(profile.id, "resp-bad", d))
        .collect();
    // normalized above 1 is refused by the schema, after three rows were written.
    rows[3].normalized = Some(1.5);

    let result = db.attempts().insert_response(&rows).await;
    assert!(matches!(result, Err(StorageError::Constraint(_))));
    assert!(
        db.attempts()
            .by_response("resp-bad")
            .await
            .expect("rows")
            .is_empty()
    );
}

#[tokio::test]
async fn rows_that_do_not_belong_to_one_response_are_refused() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let mixed = [
        new_attempt(profile.id, "resp-a", "task"),
        new_attempt(profile.id, "resp-b", "grammar"),
    ];
    assert!(matches!(
        db.attempts().insert_response(&mixed).await,
        Err(StorageError::Rule(_))
    ));
    assert!(matches!(
        db.attempts().insert_response(&[]).await,
        Err(StorageError::Rule(_))
    ));
}

#[tokio::test]
async fn free_mode_and_generated_work_cannot_be_stored_as_counting_toward_an_estimate() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    for origin in [AttemptOrigin::FreeMode, AttemptOrigin::Generated] {
        let counting = storage::NewAttempt {
            origin,
            counts_toward_estimate: true,
            ..new_attempt(profile.id, "resp-free", "task")
        };
        assert!(matches!(
            db.attempts().insert(&counting).await,
            Err(StorageError::Rule(_))
        ));
        let not_counting = storage::NewAttempt {
            counts_toward_estimate: false,
            ..counting
        };
        db.attempts()
            .insert(&not_counting)
            .await
            .expect("stored without counting");
    }
    let authored = db
        .attempts()
        .insert(&new_attempt(profile.id, "resp-auth", "task"))
        .await
        .expect("authored");
    assert!(authored.counts_toward_estimate);
}

#[tokio::test]
async fn skill_reads_are_windowed_and_ordered_by_time() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    for (response, at) in [("late", 300), ("early", 100), ("middle", 200)] {
        let attempt = storage::NewAttempt {
            created_at: ts(at),
            ..new_attempt(profile.id, response, "task")
        };
        db.attempts().insert(&attempt).await.expect("insert");
    }
    let listening = storage::NewAttempt {
        skill: "listening".to_owned(),
        ..new_attempt(profile.id, "other-skill", "task")
    };
    db.attempts().insert(&listening).await.expect("insert");

    let rows = db
        .attempts()
        .for_skill_since(profile.id, "writing", &ts(150))
        .await
        .expect("rows");
    let responses: Vec<&str> = rows.iter().map(|a| a.response_id.as_str()).collect();
    assert_eq!(responses, ["middle", "late"]);
}

/// `counted_for_skill`: the list behind one skill's estimate.
#[tokio::test]
async fn counted_attempts_of_a_skill_are_newest_first_and_only_mine() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let other = make_profile(&db).await;
    for (response, at) in [("late", 300), ("early", 100), ("middle", 200)] {
        let row = storage::NewAttempt {
            created_at: ts(at),
            ..new_attempt(profile.id, response, "task")
        };
        db.attempts().insert(&row).await.expect("insert");
    }
    // Same instant: the newest row wins the tie.
    for response in ["tie-first", "tie-second"] {
        let row = storage::NewAttempt {
            created_at: ts(300),
            ..new_attempt(profile.id, response, "task")
        };
        db.attempts().insert(&row).await.expect("insert");
    }
    let other_skill = storage::NewAttempt {
        skill: "listening".to_owned(),
        ..new_attempt(profile.id, "other-skill", "task")
    };
    db.attempts().insert(&other_skill).await.expect("insert");
    db.attempts()
        .insert(&new_attempt(other.id, "theirs", "task"))
        .await
        .expect("insert");

    let rows = db
        .attempts()
        .counted_for_skill(profile.id, "writing", 10)
        .await
        .expect("rows");
    let responses: Vec<&str> = rows.iter().map(|a| a.response_id.as_str()).collect();
    assert_eq!(
        responses,
        ["tie-second", "tie-first", "late", "middle", "early"]
    );
}

#[tokio::test]
async fn counted_attempts_leave_out_rows_that_do_not_count() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    db.attempts()
        .insert(&new_attempt(profile.id, "counts", "task"))
        .await
        .expect("insert");
    let off = storage::NewAttempt {
        counts_toward_estimate: false,
        ..new_attempt(profile.id, "off", "task")
    };
    db.attempts().insert(&off).await.expect("insert");
    let free = storage::NewAttempt {
        origin: storage::AttemptOrigin::FreeMode,
        counts_toward_estimate: false,
        ..new_attempt(profile.id, "free", "task")
    };
    db.attempts().insert(&free).await.expect("insert");

    let rows = db
        .attempts()
        .counted_for_skill(profile.id, "writing", 10)
        .await
        .expect("rows");
    let responses: Vec<&str> = rows.iter().map(|a| a.response_id.as_str()).collect();
    assert_eq!(responses, ["counts"]);
}

#[tokio::test]
async fn counted_attempts_are_cut_at_the_limit_and_an_unknown_skill_is_empty() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    for n in 0..3 {
        let row = storage::NewAttempt {
            created_at: ts(100 + n),
            ..new_attempt(profile.id, &format!("resp-{n}"), "task")
        };
        db.attempts().insert(&row).await.expect("insert");
    }
    let two = db
        .attempts()
        .counted_for_skill(profile.id, "writing", 2)
        .await
        .expect("rows");
    assert_eq!(two.len(), 2);
    assert_eq!(two[0].response_id, "resp-2");
    // The caller asking for one more than it shows sees the cut.
    let three = db
        .attempts()
        .counted_for_skill(profile.id, "writing", 3)
        .await
        .expect("rows");
    assert_eq!(three.len(), 3);

    let none = db
        .attempts()
        .counted_for_skill(profile.id, "reading", 10)
        .await
        .expect("an unknown skill is not an error");
    assert!(none.is_empty());
}

#[tokio::test]
async fn a_pending_score_is_filled_in_later() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let waiting = storage::NewAttempt {
        status: AttemptStatus::PendingLlm,
        normalized: None,
        confidence: None,
        raw_score: None,
        max_score: None,
        ..new_attempt(profile.id, "resp-wait", "task")
    };
    let attempt = db.attempts().insert(&waiting).await.expect("insert");
    let entry = db
        .pending_scoring()
        .enqueue(attempt.id, &json!({ "rubric": "w1" }), &ts(101))
        .await
        .expect("enqueue");
    assert!(matches!(
        db.pending_scoring()
            .enqueue(attempt.id, &json!({}), &ts(102))
            .await,
        Err(StorageError::Constraint(_))
    ));

    db.pending_scoring()
        .record_try(entry.id)
        .await
        .expect("try");
    let queued = db.pending_scoring().oldest(10).await.expect("oldest");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].tries, 1);

    let update = ScoreUpdate {
        scorer_version: "rubric-w1/1".to_owned(),
        raw_score: Some(2.0),
        max_score: Some(4.0),
        normalized: Some(0.5),
        confidence: Some(0.7),
        status: AttemptStatus::Scored,
    };
    db.attempts()
        .update_score(attempt.id, &update)
        .await
        .expect("update");
    db.pending_scoring().remove(entry.id).await.expect("remove");

    let stored = db
        .attempts()
        .get(attempt.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(stored.status, AttemptStatus::Scored);
    assert_eq!(stored.normalized, Some(0.5));
    assert!(
        db.pending_scoring()
            .oldest(10)
            .await
            .expect("oldest")
            .is_empty()
    );
}

#[tokio::test]
async fn evidence_is_kept_per_attempt_and_readable_per_response() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let rows = [
        new_attempt(profile.id, "resp-e", "task"),
        new_attempt(profile.id, "resp-e", "grammar"),
    ];
    let attempts = db.attempts().insert_response(&rows).await.expect("insert");
    let quote = db
        .evidence()
        .add(&NewEvidence {
            attempt_id: attempts[0].id,
            kind: EvidenceKind::Quote,
            content: Some("I go to school".to_owned()),
            data: None,
            created_at: ts(110),
        })
        .await
        .expect("quote");
    db.evidence()
        .add(&NewEvidence {
            attempt_id: attempts[1].id,
            kind: EvidenceKind::Metric,
            content: None,
            data: Some(json!({ "words": 42 })),
            created_at: ts(111),
        })
        .await
        .expect("metric");

    assert_eq!(
        db.evidence()
            .for_attempt(attempts[0].id)
            .await
            .expect("list"),
        vec![quote]
    );
    let all = db
        .evidence()
        .for_response("resp-e")
        .await
        .expect("by response");
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].data, Some(json!({ "words": 42 })));
}

fn estimate(
    profile_id: i64,
    skill: &str,
    level: Option<EstimateLevel>,
    at: i64,
) -> NewSkillEstimate {
    NewSkillEstimate {
        profile_id,
        skill: skill.to_owned(),
        level,
        status: if level.is_some() {
            EstimateStatus::Estimated
        } else {
            EstimateStatus::InsufficientEvidence
        },
        confidence: level.map(|_| 0.6),
        evidence_count: 9,
        algorithm_version: "est/1".to_owned(),
        detail: None,
        computed_at: ts(at),
    }
}

#[tokio::test]
async fn estimates_keep_history_and_the_latest_row_is_current() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let estimates = db.estimates();
    estimates
        .insert(&estimate(profile.id, "reading", None, 10))
        .await
        .expect("insert");
    estimates
        .insert(&estimate(
            profile.id,
            "reading",
            Some(EstimateLevel::A1),
            20,
        ))
        .await
        .expect("insert");
    estimates
        .insert(&estimate(
            profile.id,
            "listening",
            Some(EstimateLevel::PreA1),
            15,
        ))
        .await
        .expect("insert");
    estimates
        .insert(&estimate(
            profile.id,
            "reading",
            Some(EstimateLevel::A2),
            30,
        ))
        .await
        .expect("insert");

    let current = estimates
        .latest(profile.id, "reading")
        .await
        .expect("latest")
        .expect("exists");
    assert_eq!(current.level, Some(EstimateLevel::A2));
    let history = estimates
        .history(profile.id, "reading")
        .await
        .expect("history");
    let levels: Vec<Option<EstimateLevel>> = history.iter().map(|e| e.level).collect();
    assert_eq!(
        levels,
        [None, Some(EstimateLevel::A1), Some(EstimateLevel::A2)]
    );

    let per_skill = estimates
        .latest_per_skill(profile.id)
        .await
        .expect("per skill");
    let summary: Vec<(&str, Option<EstimateLevel>)> = per_skill
        .iter()
        .map(|e| (e.skill.as_str(), e.level))
        .collect();
    assert_eq!(
        summary,
        [
            ("listening", Some(EstimateLevel::PreA1)),
            ("reading", Some(EstimateLevel::A2))
        ]
    );
    assert_eq!(
        estimates
            .latest(profile.id, "speaking")
            .await
            .expect("latest"),
        None
    );
}
