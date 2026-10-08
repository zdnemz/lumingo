#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{temp_db, ts};
use serde_json::json;
use storage::{
    IndexStatus, InstalledModel, KeySource, Level, LlmCallType, LlmOutcome, ModelRole,
    NewCurriculumVersion, NewLlmCall, NewObjective, NewPerfSample, NewProviderProfile, NewUnit,
    ProviderProtocol, StorageError,
};

fn unit(id: &str, level: Level, sequence: i64, sha: &str) -> NewUnit {
    NewUnit {
        id: id.to_owned(),
        level,
        sequence,
        title_en: format!("Title of {id}"),
        file_sha256: sha.to_owned(),
        objectives: vec![
            NewObjective {
                id: format!("{id}/o1"),
                skill: "listening".to_owned(),
                can_do_en: "I can follow a short greeting.".to_owned(),
            },
            NewObjective {
                id: format!("{id}/o2"),
                skill: "speaking".to_owned(),
                can_do_en: "I can introduce myself.".to_owned(),
            },
        ],
    }
}

fn version(name: &str, manifest: &str, units: Vec<NewUnit>, at: i64) -> NewCurriculumVersion {
    NewCurriculumVersion {
        content_version: name.to_owned(),
        schema_version: "1".to_owned(),
        manifest_sha256: manifest.to_owned(),
        installed_at: ts(at),
        units,
    }
}

#[tokio::test]
async fn a_curriculum_version_is_indexed_with_its_checksums() {
    let (_dir, db) = temp_db().await;
    let curriculum = db.curriculum();
    assert_eq!(
        curriculum
            .index_status("0.1.0", "m1")
            .await
            .expect("status"),
        IndexStatus::Missing
    );

    let units = vec![
        unit("a1-u01", Level::A1, 1, "sha-u01"),
        unit("a1-u02", Level::A1, 2, "sha-u02"),
    ];
    let installed = curriculum
        .install(&version("0.1.0", "m1", units, 1))
        .await
        .expect("install");
    assert_eq!(installed.unit_count, 2);

    assert_eq!(
        curriculum
            .index_status("0.1.0", "m1")
            .await
            .expect("status"),
        IndexStatus::Current
    );
    assert_eq!(
        curriculum
            .index_status("0.1.0", "other")
            .await
            .expect("status"),
        IndexStatus::Changed
    );
    let checksums = curriculum.unit_checksums().await.expect("checksums");
    assert_eq!(
        checksums,
        [
            ("a1-u01".to_owned(), "sha-u01".to_owned()),
            ("a1-u02".to_owned(), "sha-u02".to_owned())
        ]
    );
    let objectives = curriculum.objectives("a1-u01").await.expect("objectives");
    assert_eq!(objectives.len(), 2);
    assert_eq!(
        curriculum.latest_version().await.expect("latest"),
        Some(installed)
    );
}

#[tokio::test]
async fn reinstalling_a_unit_in_a_newer_version_updates_it_in_place() {
    let (_dir, db) = temp_db().await;
    let curriculum = db.curriculum();
    curriculum
        .install(&version(
            "0.1.0",
            "m1",
            vec![unit("a1-u01", Level::A1, 1, "old")],
            1,
        ))
        .await
        .expect("first");
    let mut changed = unit("a1-u01", Level::A1, 1, "new");
    changed.objectives.truncate(1);
    let second = curriculum
        .install(&version("0.2.0", "m2", vec![changed], 2))
        .await
        .expect("second");

    let stored = curriculum
        .unit("a1-u01")
        .await
        .expect("unit")
        .expect("exists");
    assert_eq!(stored.file_sha256, "new");
    assert_eq!(stored.curriculum_version_id, second.id);
    assert_eq!(
        curriculum
            .objectives("a1-u01")
            .await
            .expect("objectives")
            .len(),
        1
    );
}

#[tokio::test]
async fn a_failed_install_leaves_the_previous_index_untouched() {
    let (_dir, db) = temp_db().await;
    let curriculum = db.curriculum();
    curriculum
        .install(&version(
            "0.1.0",
            "m1",
            vec![unit("a1-u01", Level::A1, 1, "sha")],
            1,
        ))
        .await
        .expect("first");

    // Sequence 31 breaks the CHECK on the second unit, after the first was written.
    let broken = version(
        "0.2.0",
        "m2",
        vec![
            unit("a1-u02", Level::A1, 2, "sha2"),
            unit("a1-u03", Level::A1, 31, "sha3"),
        ],
        2,
    );
    assert!(matches!(
        curriculum.install(&broken).await,
        Err(StorageError::Constraint(_))
    ));
    assert_eq!(
        curriculum
            .index_status("0.2.0", "m2")
            .await
            .expect("status"),
        IndexStatus::Missing
    );
    assert!(curriculum.unit("a1-u02").await.expect("unit").is_none());
}

#[tokio::test]
async fn an_objective_must_belong_to_its_unit_by_id() {
    let (_dir, db) = temp_db().await;
    let mut bad = unit("a1-u01", Level::A1, 1, "sha");
    bad.objectives[0].id = "a1-u09/o1".to_owned();
    let result = db
        .curriculum()
        .install(&version("0.1.0", "m1", vec![bad], 1))
        .await;
    assert!(matches!(result, Err(StorageError::Rule(_))));
}

fn model(id: &str, role: ModelRole, sha: &str) -> InstalledModel {
    InstalledModel {
        id: id.to_owned(),
        role,
        version: "1".to_owned(),
        path: format!("models/files/{id}.onnx"),
        sha256: sha.to_owned(),
        size_bytes: 1_000,
        license: "MIT".to_owned(),
        installed_at: ts(5),
    }
}

#[tokio::test]
async fn installed_models_are_upserted_listed_and_removed() {
    let (_dir, db) = temp_db().await;
    let models = db.models();
    models
        .upsert(&model("silero-vad", ModelRole::Vad, "aaa"))
        .await
        .expect("vad");
    models
        .upsert(&model("whisper-small", ModelRole::Stt, "bbb"))
        .await
        .expect("stt");
    models
        .upsert(&model("silero-vad", ModelRole::Vad, "ccc"))
        .await
        .expect("re-download");

    let all = models.list().await.expect("list");
    assert_eq!(all.len(), 2);
    assert_eq!(
        models
            .get("silero-vad")
            .await
            .expect("get")
            .map(|m| m.sha256),
        Some("ccc".to_owned())
    );
    assert_eq!(
        models.by_role(ModelRole::Stt).await.expect("by role").len(),
        1
    );
    assert!(models.remove("silero-vad").await.expect("remove"));
    assert!(!models.remove("silero-vad").await.expect("remove again"));
}

fn provider(name: &str) -> NewProviderProfile {
    NewProviderProfile {
        name: name.to_owned(),
        protocol: ProviderProtocol::OpenaiChat,
        base_url: "https://api.example.test/v1".to_owned(),
        model: "test-model".to_owned(),
        key_source: KeySource::File,
        created_at: ts(1),
    }
}

#[tokio::test]
async fn only_one_provider_is_active_and_a_bad_switch_changes_nothing() {
    let (_dir, db) = temp_db().await;
    let providers = db.providers();
    let a = providers.create(&provider("a")).await.expect("a");
    let b = providers.create(&provider("b")).await.expect("b");
    assert!(providers.active().await.expect("active").is_none());

    providers.set_active(a.id).await.expect("activate a");
    providers.set_active(b.id).await.expect("activate b");
    assert_eq!(
        providers.active().await.expect("active").map(|p| p.id),
        Some(b.id)
    );
    let active_count = providers
        .list()
        .await
        .expect("list")
        .iter()
        .filter(|p| p.is_active)
        .count();
    assert_eq!(active_count, 1);

    assert!(matches!(
        providers.set_active(9_999).await,
        Err(StorageError::NotFound { .. })
    ));
    assert_eq!(
        providers.active().await.expect("active").map(|p| p.id),
        Some(b.id)
    );
    assert!(matches!(
        providers.create(&provider("a")).await,
        Err(StorageError::Constraint(_))
    ));
}

#[tokio::test]
async fn a_probe_and_a_qualification_are_recorded_on_the_profile() {
    let (_dir, db) = temp_db().await;
    let providers = db.providers();
    let p = providers.create(&provider("a")).await.expect("create");
    let capabilities = json!({ "structured_output": "json_schema" });
    providers
        .record_probe(p.id, &capabilities, &ts(2))
        .await
        .expect("probe");
    providers
        .set_qualified(p.id, Some(&ts(3)))
        .await
        .expect("qualified");

    let stored = providers.get(p.id).await.expect("get").expect("exists");
    assert_eq!(stored.capabilities, Some(capabilities));
    assert_eq!(stored.probed_at, Some(ts(2)));
    assert_eq!(stored.qualified_at, Some(ts(3)));
}

#[tokio::test]
async fn deleting_a_provider_keeps_its_call_log_with_a_null_provider() {
    let (_dir, db) = temp_db().await;
    let p = db.providers().create(&provider("a")).await.expect("create");
    db.diagnostics()
        .record_llm_call(&NewLlmCall {
            provider_profile_id: Some(p.id),
            call_type: LlmCallType::TutorTurn,
            model: "test-model".to_owned(),
            ladder_level: Some(2),
            ttft_ms: Some(400),
            total_ms: Some(1_800),
            input_tokens: Some(300),
            output_tokens: Some(60),
            http_status: Some(200),
            outcome: LlmOutcome::Ok,
            started_at: ts(10),
        })
        .await
        .expect("record");
    db.providers().delete(p.id).await.expect("delete provider");

    let calls = db.diagnostics().recent_llm_calls(10).await.expect("calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].provider_profile_id, None);
    assert_eq!(calls[0].total_ms, Some(1_800));
}

#[tokio::test]
async fn log_fields_that_could_carry_prose_are_refused() {
    let (_dir, db) = temp_db().await;
    let sample = |metric: &str, tag: &str| NewPerfSample {
        session_id: None,
        turn_seq: None,
        metric: metric.to_owned(),
        value_ms: 12.5,
        profile_tag: tag.to_owned(),
        created_at: ts(1),
    };
    db.diagnostics()
        .record_perf_sample(&sample("llm_first_sentence_ms", "floor"))
        .await
        .expect("plain identifiers are accepted");
    for bad in [
        sample("I went to the market yesterday", "floor"),
        sample("e2e_ms", "my own hardware, a laptop"),
        sample("", "floor"),
    ] {
        assert!(matches!(
            db.diagnostics().record_perf_sample(&bad).await,
            Err(StorageError::Rule(_))
        ));
    }
    let samples = db
        .diagnostics()
        .perf_samples("llm_first_sentence_ms", &ts(0))
        .await
        .expect("samples");
    assert_eq!(samples.len(), 1);
}
