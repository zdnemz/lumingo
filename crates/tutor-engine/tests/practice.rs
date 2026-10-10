#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::{FakeLlm, example_unit, make_profile, make_session, temp_db, test_clock};
use curriculum::Activity;
use llm_client::{InvalidOutput, InvalidReason, LadderLevel, LlmError, TimeoutKind};
use serde_json::{Value, json};
use storage::{AttemptOrigin, Database, GeneratedKind, LlmOutcome, SessionKind};
use tokio_util::sync::CancellationToken;
use tutor_engine::{
    FallbackReason, PracticeAnswer, PracticeConfig, PracticeGenerator, PracticeSource,
    record_practice_attempt,
};

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    llm: Arc<FakeLlm>,
    generator: PracticeGenerator,
    profile_id: i64,
    session_id: i64,
}

fn recorder(f: &Fixture) -> tutor_engine::EvidenceRecorder {
    tutor_engine::EvidenceRecorder::new(f.db.clone(), test_clock())
}

async fn fixture() -> Fixture {
    let (dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let session = make_session(&db, profile.id, SessionKind::Drill).await;
    let llm = FakeLlm::new();
    let generator = PracticeGenerator::new(
        PracticeConfig {
            profile_id: profile.id,
            session_id: session.id,
            provider_profile_id: None,
            model: "test-model".into(),
            first_language: "Indonesian".into(),
            word_levels: None,
        },
        llm.clone(),
        db.clone(),
        test_clock(),
    );
    Fixture {
        _dir: dir,
        db,
        llm,
        generator,
        profile_id: profile.id,
        session_id: session.id,
    }
}

fn mcq(stem: &str, answer_index: i64) -> Value {
    json!({
        "type": "mcq", "objective_id": "o2-introduce", "grammar_id": "g-be-i-am",
        "stem": stem, "options": ["am", "is", "are"], "answer_index": answer_index,
        "text": "", "answers": [], "tokens": [], "answer": "",
        "explanation_en": "Use am with I.", "explanation_l1": "Pakai am dengan I."
    })
}

fn gap(text: &str) -> Value {
    json!({
        "type": "gap_fill", "objective_id": "o2-introduce", "grammar_id": "g-be-from",
        "stem": "", "options": [], "answer_index": -1,
        "text": text, "answers": [["is"]], "tokens": [], "answer": "",
        "explanation_en": "Use is with he.", "explanation_l1": "Pakai is dengan he."
    })
}

fn reorder() -> Value {
    json!({
        "type": "reorder", "objective_id": "o2-introduce", "grammar_id": "g-be-from",
        "stem": "", "options": [], "answer_index": -1, "text": "", "answers": [],
        "tokens": ["from", "I", "am", "Bali"], "answer": "I am from Bali",
        "explanation_en": "Say who you are.", "explanation_l1": "Katakan siapa kamu."
    })
}

fn items(list: Vec<Value>) -> Value {
    json!({ "items": list })
}

fn never() -> CancellationToken {
    CancellationToken::new()
}

#[tokio::test]
async fn valid_items_are_converted_marked_generated_and_stored_with_the_session() {
    let f = fixture().await;
    let unit = example_unit();
    f.llm.queue_structured(Ok(items(vec![
        mcq("Dewi says: I ___ a student.", 0),
        gap("Budi ___ from Bali."),
        reorder(),
    ])));
    let set = f.generator.generate(&unit, 3, &[], &never()).await.unwrap();

    assert_eq!(set.source, PracticeSource::Generated);
    assert_eq!(set.fallback, None);
    assert_eq!(set.items.len(), 3);
    assert!(
        set.items
            .iter()
            .all(|i| i.origin == AttemptOrigin::Generated)
    );
    assert!(!set.vocabulary_checked);
    let ids: Vec<&str> = set.items.iter().map(|i| i.activity.id()).collect();
    assert_eq!(
        ids,
        [
            format!("gen-s{}-1", f.session_id),
            format!("gen-s{}-2", f.session_id),
            format!("gen-s{}-3", f.session_id)
        ]
    );

    let stored =
        f.db.generated_content()
            .for_session(f.session_id)
            .await
            .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, GeneratedKind::PracticeItems);
    assert_eq!(stored[0].contract_version, "practice_items/1");
    assert_eq!(stored[0].model, "test-model");
    assert_eq!(stored[0].content["items"].as_array().unwrap().len(), 3);

    let seen = f.llm.structured_requests();
    assert!(seen[0].system.contains("Write exactly 3 items"));
    let body: Value = serde_json::from_str(&seen[0].messages[0].content).unwrap();
    assert_eq!(body["max_level"], "A1");
    assert!(!body["existing_items"].as_array().unwrap().is_empty());
    let grammar: Vec<&str> = body["grammar"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["id"].as_str().unwrap())
        .collect();
    assert_eq!(grammar, ["g-be-i-am", "g-be-from", "g-question-chunks"]);
}

#[tokio::test]
async fn invalid_items_are_dropped_and_enough_valid_ones_make_the_set() {
    let f = fixture().await;
    let unit = example_unit();
    f.llm.queue_structured(Ok(items(vec![
        mcq("Dewi says: I ___ a student.", 0),
        mcq("Broken: I ___ a teacher.", 7),
        gap("Budi ___ from Bali."),
        gap("Budi ___ from Bali and ___ happy."),
    ])));
    let set = f.generator.generate(&unit, 4, &[], &never()).await.unwrap();
    assert_eq!(set.source, PracticeSource::Generated);
    assert_eq!(set.items.len(), 2);
    assert_eq!(set.rejected.len(), 2);
    assert!(!set.regenerated);
    assert_eq!(f.llm.structured_calls(), 1);
}

#[tokio::test]
async fn fewer_than_half_valid_regenerates_once_and_the_second_try_can_pass() {
    let f = fixture().await;
    let unit = example_unit();
    f.llm.queue_structured(Ok(items(vec![
        mcq("Bad one: I ___ a cook.", 9),
        mcq("Bad two: I ___ a cook.", 9),
    ])));
    f.llm.queue_structured(Ok(items(vec![
        mcq("Dewi says: I ___ a student.", 0),
        mcq("Putu says: I ___ a teacher.", 0),
    ])));
    let set = f.generator.generate(&unit, 2, &[], &never()).await.unwrap();
    assert_eq!(set.source, PracticeSource::Generated);
    assert!(set.regenerated);
    assert_eq!(set.items.len(), 2);
    assert_eq!(f.llm.structured_calls(), 2);
}

#[tokio::test]
async fn when_the_second_try_also_fails_the_learner_gets_authored_items() {
    let f = fixture().await;
    let unit = example_unit();
    for _ in 0..2 {
        f.llm.queue_structured(Ok(items(vec![
            mcq("Bad one: I ___ a cook.", 9),
            mcq("Bad two: I ___ a cook.", 9),
        ])));
    }
    let set = f
        .generator
        .generate(&unit, 2, &["a03-gap-am".to_owned()], &never())
        .await
        .unwrap();
    assert_eq!(set.source, PracticeSource::Authored);
    assert_eq!(set.fallback, Some(FallbackReason::TooFewValid));
    assert_eq!(set.items.len(), 2);
    assert_eq!(
        set.items[0].activity.id(),
        "a03-gap-am",
        "wrong answers come first"
    );
    assert!(
        set.items
            .iter()
            .all(|i| i.origin == AttemptOrigin::Authored)
    );
    assert_eq!(f.llm.structured_calls(), 2, "one regeneration, not more");
    assert!(
        f.db.generated_content()
            .for_session(f.session_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn provider_trouble_and_invalid_output_fall_back_to_authored_items() {
    let unit = example_unit();
    for (error, reason) in [
        (
            LlmError::Timeout(TimeoutKind::Total),
            FallbackReason::ProviderUnavailable,
        ),
        (
            LlmError::RateLimited { retry_after: None },
            FallbackReason::ProviderUnavailable,
        ),
        (
            LlmError::Auth { status: 401 },
            FallbackReason::ProviderUnavailable,
        ),
        // What the `NoProvider` client answers: not the model's fault.
        (
            LlmError::InvalidRequest("no provider is configured".to_owned()),
            FallbackReason::ProviderUnavailable,
        ),
        (
            LlmError::InvalidOutput(InvalidOutput {
                reason: InvalidReason::SchemaMismatch,
                paths: vec![],
                ladder_level: LadderLevel::NativeSchema,
                repaired: true,
            }),
            FallbackReason::InvalidOutput,
        ),
    ] {
        let f = fixture().await;
        f.llm.queue_structured(Err(error));
        let set = f.generator.generate(&unit, 3, &[], &never()).await.unwrap();
        assert_eq!(set.source, PracticeSource::Authored);
        assert_eq!(set.fallback, Some(reason));
        assert_eq!(set.items.len(), 3);
        assert_eq!(
            f.llm.structured_calls(),
            1,
            "no second call against a failing provider"
        );
    }
}

#[tokio::test]
async fn a_cancelled_call_is_an_error_not_a_fallback() {
    let f = fixture().await;
    f.llm.queue_structured(Err(LlmError::Cancelled));
    let result = f
        .generator
        .generate(&example_unit(), 3, &[], &never())
        .await;
    assert!(matches!(
        result,
        Err(tutor_engine::EngineError::Llm(LlmError::Cancelled))
    ));
}

#[tokio::test]
async fn the_policy_limit_per_session_is_enforced_across_calls() {
    let f = fixture().await;
    let mut unit = example_unit();
    unit.generation_policy.max_items_per_session = 3;
    f.llm.queue_structured(Ok(items(vec![
        mcq("Dewi says: I ___ a student.", 0),
        gap("Budi ___ from Bali."),
    ])));
    let first = f.generator.generate(&unit, 2, &[], &never()).await.unwrap();
    assert_eq!(first.items.len(), 2);

    // Only one more item is allowed, whatever is asked for.
    f.llm.queue_structured(Ok(items(vec![reorder()])));
    let second = f.generator.generate(&unit, 5, &[], &never()).await.unwrap();
    assert_eq!(second.items.len(), 1);
    let seen = f.llm.structured_requests();
    assert!(seen[1].system.contains("Write exactly 1 items"));
    let body: Value = serde_json::from_str(&seen[1].messages[0].content).unwrap();
    let existing = body["existing_items"].as_array().unwrap();
    assert!(
        existing.iter().any(|e| e == "Budi ___ from Bali."),
        "earlier items are not repeated"
    );
    drop(seen);

    let third = f.generator.generate(&unit, 2, &[], &never()).await.unwrap();
    assert_eq!(third.fallback, Some(FallbackReason::SessionLimit));
    assert_eq!(third.source, PracticeSource::Authored);
    assert_eq!(
        f.llm.structured_calls(),
        2,
        "no call once the limit is reached"
    );
}

#[tokio::test]
async fn a_policy_that_forbids_generation_makes_no_call() {
    let f = fixture().await;
    let mut unit = example_unit();
    unit.generation_policy.allowed_types.clear();
    let set = f.generator.generate(&unit, 3, &[], &never()).await.unwrap();
    assert_eq!(set.fallback, Some(FallbackReason::PolicyForbids));
    assert!(
        set.items.is_empty(),
        "no allowed type means no authored item of one either"
    );
    assert_eq!(f.llm.structured_calls(), 0);
}

#[tokio::test]
async fn the_call_is_logged_as_practice_generation() {
    let f = fixture().await;
    f.llm
        .queue_structured(Ok(items(vec![mcq("Dewi says: I ___ a student.", 0)])));
    f.generator
        .generate(&example_unit(), 1, &[], &never())
        .await
        .unwrap();
    let calls = f.db.diagnostics().recent_llm_calls(5).await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].call_type, storage::LlmCallType::PracticeGen);
    assert_eq!(calls[0].outcome, LlmOutcome::Ok);
}

#[tokio::test]
async fn answers_are_scored_and_never_count_toward_an_estimate() {
    let f = fixture().await;
    let unit = example_unit();
    f.llm.queue_structured(Ok(items(vec![
        mcq("Dewi says: I ___ a student.", 0),
        gap("Budi ___ from Bali."),
        reorder(),
    ])));
    let set = f.generator.generate(&unit, 3, &[], &never()).await.unwrap();
    let answers = [
        PracticeAnswer::Choice(0),
        PracticeAnswer::Gaps(vec!["are".into()]),
        PracticeAnswer::Order(["I", "am", "from", "Bali"].map(String::from).to_vec()),
    ];
    let mut scores = Vec::new();
    for (item, answer) in set.items.iter().zip(&answers) {
        let (score, attempt) = record_practice_attempt(
            &recorder(&f),
            f.profile_id,
            f.session_id,
            &unit,
            item,
            answer,
        )
        .await
        .unwrap();
        scores.push(score);
        assert_eq!(attempt.origin, AttemptOrigin::Generated);
        assert!(!attempt.counts_toward_estimate);
    }
    assert_eq!(scores, [1.0, 0.0, 1.0]);
    let stored = f.db.attempts().for_session(f.session_id).await.unwrap();
    assert_eq!(stored.len(), 3);
    assert!(stored.iter().all(|a| !a.counts_toward_estimate));
}

#[tokio::test]
async fn a_replayed_authored_item_is_practice_and_does_not_count_either() {
    let f = fixture().await;
    let unit = example_unit();
    f.llm
        .queue_structured(Err(LlmError::Timeout(TimeoutKind::Total)));
    let set = f
        .generator
        .generate(&unit, 1, &["a03-gap-am".to_owned()], &never())
        .await
        .unwrap();
    let item = &set.items[0];
    let Activity::GapFill(gap) = &item.activity else {
        panic!("expected the gap fill");
    };
    let right: Vec<String> = gap.answers.iter().map(|a| a[0].clone()).collect();
    let (score, attempt) = record_practice_attempt(
        &recorder(&f),
        f.profile_id,
        f.session_id,
        &unit,
        item,
        &PracticeAnswer::Gaps(right),
    )
    .await
    .unwrap();
    assert_eq!(score, 1.0);
    assert_eq!(attempt.origin, AttemptOrigin::Authored);
    assert!(!attempt.counts_toward_estimate);
}

#[tokio::test]
async fn an_answer_of_the_wrong_kind_is_refused() {
    let f = fixture().await;
    let unit = example_unit();
    f.llm
        .queue_structured(Ok(items(vec![mcq("Dewi says: I ___ a student.", 0)])));
    let set = f.generator.generate(&unit, 1, &[], &never()).await.unwrap();
    let result = record_practice_attempt(
        &recorder(&f),
        f.profile_id,
        f.session_id,
        &unit,
        &set.items[0],
        &PracticeAnswer::Gaps(vec!["am".into()]),
    )
    .await;
    assert!(matches!(
        result,
        Err(tutor_engine::EngineError::Activity(
            tutor_engine::ActivityError::WrongKind { .. }
        ))
    ));
}
