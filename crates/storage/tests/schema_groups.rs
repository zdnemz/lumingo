#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The table groups of `docs/DATA_MODEL.md` and the walls between them:
//! technical logs never hold learner text, and the game layer is separate from
//! assessment.

mod common;

use std::collections::BTreeSet;

use common::{count, make_profile, make_session, new_attempt, raw_conn, temp_db, ts};
use sqlx::SqliteConnection;
use storage::{
    AttemptOrigin, EstimateLevel, EstimateStatus, LocalDate, NewSkillEstimate, NewXp, SessionKind,
    XpSourceKind,
};

const SETTINGS: &[&str] = &["app_settings", "profiles"];
const CURRICULUM_INDEX: &[&str] = &["curriculum_versions", "units", "objectives"];
const PROVIDERS_AND_MODELS: &[&str] = &["provider_profiles", "models_installed"];
const CONVERSATION: &[&str] = &[
    "sessions",
    "turns",
    "turn_analysis",
    "error_events",
    "generated_content",
    "audio_clips",
];
const ASSESSMENT: &[&str] = &[
    "assessment_attempts",
    "assessment_evidence",
    "pron_results",
    "skill_estimates",
    "objective_mastery",
    "error_stats",
    "review_schedule",
    "unit_progress",
    "pending_scoring",
];
const TECHNICAL_LOGS: &[&str] = &["llm_calls", "perf_samples"];
const GAME: &[&str] = &[
    "xp_ledger",
    "rest_tokens",
    "streak_days",
    "unlockables",
    "equipped_cosmetics",
];

async fn all_tables(conn: &mut SqliteConnection) -> BTreeSet<String> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(conn)
    .await
    .expect("tables");
    names.into_iter().collect()
}

async fn columns(conn: &mut SqliteConnection, table: &str) -> Vec<(String, String)> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, type FROM pragma_table_info(?1) ORDER BY cid")
            .bind(table)
            .fetch_all(conn)
            .await
            .expect("columns");
    rows
}

async fn references(conn: &mut SqliteConnection, table: &str) -> BTreeSet<String> {
    let targets: Vec<String> =
        sqlx::query_scalar("SELECT \"table\" FROM pragma_foreign_key_list(?1)")
            .bind(table)
            .fetch_all(conn)
            .await
            .expect("foreign keys");
    targets.into_iter().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

#[tokio::test]
async fn every_table_belongs_to_exactly_one_known_group() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;
    let groups: [&[&str]; 7] = [
        SETTINGS,
        CURRICULUM_INDEX,
        PROVIDERS_AND_MODELS,
        CONVERSATION,
        ASSESSMENT,
        TECHNICAL_LOGS,
        GAME,
    ];
    let listed: Vec<&str> = groups.iter().flat_map(|g| g.iter().copied()).collect();
    let listed_set = set(&listed);
    assert_eq!(
        listed.len(),
        listed_set.len(),
        "a table is listed in two groups"
    );
    // A new table must be classified here before it can ship.
    assert_eq!(all_tables(&mut conn).await, listed_set);
    assert_eq!(
        listed.len(),
        24 + 5,
        "24 tables from migration 0001, 5 from 0002"
    );
}

#[tokio::test]
async fn technical_logs_have_no_columns_that_can_hold_learner_text() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;

    // The only text columns, by table: closed vocabularies (CHECK lists),
    // identifiers (model, metric and hardware names) and timestamps.
    let allowed_text: [(&str, &[&str]); 2] = [
        (
            "llm_calls",
            &["call_type", "model", "outcome", "started_at"],
        ),
        ("perf_samples", &["metric", "profile_tag", "created_at"]),
    ];
    let words_that_mean_text = [
        "text",
        "content",
        "prompt",
        "response",
        "message",
        "body",
        "transcript",
        "quote",
        "payload",
        "json",
        "reason",
        "correction",
        "title",
        "note",
    ];
    for table in TECHNICAL_LOGS {
        let allowed = allowed_text
            .iter()
            .find(|(name, _)| name == table)
            .map(|(_, cols)| *cols)
            .expect("table is listed");
        let cols = columns(&mut conn, table).await;
        assert!(!cols.is_empty(), "{table} has columns");
        for (name, declared) in &cols {
            for word in words_that_mean_text {
                assert!(
                    !name.contains(word),
                    "{table}.{name} looks like a text column"
                );
            }
            let numeric = declared == "INTEGER" || declared == "REAL";
            assert!(
                numeric || allowed.contains(&name.as_str()),
                "{table}.{name} ({declared}) is a text column that is not on the allowed list"
            );
        }
        // Every allowed text column really exists, so the list cannot go stale.
        for name in allowed {
            assert!(
                cols.iter().any(|(n, _)| n == name),
                "{table}.{name} is missing"
            );
        }
    }

    // The vocabularies are enforced by the database, not only by Rust types.
    let ddl: Vec<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'llm_calls'",
    )
    .fetch_all(&mut conn)
    .await
    .expect("ddl");
    assert!(ddl[0].contains("call_type            TEXT NOT NULL CHECK (call_type IN"));
    assert!(ddl[0].contains("outcome              TEXT NOT NULL CHECK (outcome IN"));
}

#[tokio::test]
async fn the_game_layer_shares_no_foreign_key_with_assessment_or_conversation() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;

    // Inside the layer a table may point at another game table; outside it only at
    // profiles, the root every learner table hangs from.
    let allowed_for_game = set(&["profiles", "rest_tokens", "unlockables"]);
    for table in GAME {
        let targets = references(&mut conn, table).await;
        assert!(
            targets.is_subset(&allowed_for_game),
            "{table} references {targets:?}"
        );
    }
    // And nothing outside the layer points into it.
    let game = set(GAME);
    for table in SETTINGS
        .iter()
        .chain(CURRICULUM_INDEX)
        .chain(PROVIDERS_AND_MODELS)
        .chain(CONVERSATION)
        .chain(ASSESSMENT)
        .chain(TECHNICAL_LOGS)
    {
        let targets = references(&mut conn, table).await;
        assert!(
            targets.is_disjoint(&game),
            "{table} references the game layer: {targets:?}"
        );
    }
}

#[tokio::test]
async fn nothing_in_the_game_layer_can_be_read_as_a_cefr_level() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;
    let forbidden = [
        "level",
        "cefr",
        "estimate",
        "evidence",
        "attempt",
        "score",
        "mastery",
        "skill",
        "grade",
        "band",
        "proficiency",
        "rank",
    ];
    for table in GAME {
        for word in forbidden {
            assert!(!table.contains(word), "table {table} contains {word}");
        }
        for (column, _) in columns(&mut conn, table).await {
            for word in forbidden {
                assert!(!column.contains(word), "{table}.{column} contains {word}");
            }
        }
    }
}

#[tokio::test]
async fn triggers_do_not_cross_between_the_game_layer_and_the_rest() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;
    let triggers: Vec<(String, String, String)> =
        sqlx::query_as("SELECT name, tbl_name, sql FROM sqlite_master WHERE type = 'trigger'")
            .fetch_all(&mut conn)
            .await
            .expect("triggers");
    assert!(
        triggers
            .iter()
            .any(|(name, _, _)| name == "trg_xp_ledger_append_only")
    );
    assert!(
        triggers
            .iter()
            .any(|(name, _, _)| name == "trg_sessions_delete_evidence")
    );

    let game = set(GAME);
    let others: BTreeSet<String> = all_tables(&mut conn)
        .await
        .difference(&game)
        .cloned()
        .collect();
    for (name, on_table, sql) in &triggers {
        let words: BTreeSet<String> = sql
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(str::to_owned)
            .collect();
        if game.contains(on_table) {
            let crossed: Vec<&String> = words.intersection(&others).collect();
            assert!(
                crossed.is_empty(),
                "{name} on a game table names {crossed:?}"
            );
        } else {
            let crossed: Vec<&String> = words.intersection(&game).collect();
            assert!(crossed.is_empty(), "{name} names game tables {crossed:?}");
        }
    }
}

/// Reads the migration text without its comments.
fn sql_without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

#[test]
fn the_game_migration_never_asks_sqlite_for_the_time_or_a_time_zone() {
    let sql = sql_without_comments(include_str!("../migrations/0002_game.sql"));
    for banned in [
        "'now'",
        "current_",
        "localtime",
        "utc",
        "unixepoch",
        "julianday",
        "strftime",
        "datetime(",
    ] {
        assert!(!sql.contains(banned), "0002_game.sql uses {banned}");
    }
    // The one date function is the validity check of a caller-supplied day.
    assert_eq!(sql.matches("date(").count(), 1);
    assert!(sql.contains("date(local_date) = local_date"));
}

#[test]
fn the_game_code_never_reads_the_clock() {
    for (file, source) in [
        ("game.rs", include_str!("../src/game.rs")),
        ("streak.rs", include_str!("../src/streak.rs")),
    ] {
        assert!(!source.contains("Timestamp::now"), "{file} reads the clock");
        assert!(!source.contains("Utc::now"), "{file} reads the clock");
        assert!(!source.contains("SystemTime"), "{file} reads the clock");
    }
}

async fn assessment_counts(conn: &mut SqliteConnection) -> Vec<i64> {
    let mut counts = Vec::new();
    for table in [
        "assessment_attempts",
        "assessment_evidence",
        "skill_estimates",
        "pending_scoring",
    ] {
        counts.push(count(conn, table).await);
    }
    counts
}

async fn game_counts(conn: &mut SqliteConnection) -> Vec<i64> {
    let mut counts = Vec::new();
    for table in [
        "xp_ledger",
        "streak_days",
        "rest_tokens",
        "unlockables",
        "equipped_cosmetics",
    ] {
        counts.push(count(conn, table).await);
    }
    counts
}

#[tokio::test]
async fn game_activity_never_writes_to_the_evidence_path_and_the_reverse() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let mut conn = raw_conn(&db).await;
    let before = assessment_counts(&mut conn).await;
    assert_eq!(before, [0, 0, 0, 0]);

    // XP from every kind of source, including free modes, streaks, tokens and cosmetics.
    for (n, kind) in XpSourceKind::ALL.iter().enumerate() {
        db.game()
            .award_xp(&NewXp {
                profile_id: profile.id,
                created_at: ts(1),
                source_kind: *kind,
                source_id: format!("session:{n}"),
                amount: 10,
                reason: "activity.completed".to_owned(),
            })
            .await
            .expect("xp");
    }
    db.game()
        .grant_rest_token(profile.id, &ts(2), "welcome")
        .await
        .expect("token");
    for d in [1, 2, 4] {
        let date = LocalDate::from_ymd(2026, 10, d).expect("day");
        db.game()
            .record_activity(profile.id, date, &ts(3))
            .await
            .expect("day");
    }
    db.game()
        .unlock(profile.id, "theme.sunset", &ts(4))
        .await
        .expect("unlock");
    db.game()
        .equip_theme(profile.id, Some("theme.sunset"), &ts(5))
        .await
        .expect("equip");
    assert_eq!(assessment_counts(&mut conn).await, before);

    // The other direction: attempts (free-mode included), evidence and estimates
    // leave the game layer exactly as it was.
    let game_before = game_counts(&mut conn).await;
    let xp_before = db.game().xp_total(profile.id).await.expect("xp");
    let session = make_session(&db, profile.id, SessionKind::TextChat).await;
    let free = storage::NewAttempt {
        session_id: Some(session.id),
        origin: AttemptOrigin::FreeMode,
        counts_toward_estimate: false,
        ..new_attempt(profile.id, "resp-free", "task")
    };
    db.attempts()
        .insert(&free)
        .await
        .expect("free-mode attempt");
    db.attempts()
        .insert(&new_attempt(profile.id, "resp-authored", "task"))
        .await
        .expect("authored attempt");
    db.estimates()
        .insert(&NewSkillEstimate {
            profile_id: profile.id,
            skill: "writing".to_owned(),
            level: Some(EstimateLevel::A1),
            status: EstimateStatus::Estimated,
            confidence: Some(0.6),
            evidence_count: 8,
            algorithm_version: "est/1".to_owned(),
            detail: None,
            computed_at: ts(6),
        })
        .await
        .expect("estimate");
    assert_eq!(game_counts(&mut conn).await, game_before);
    assert_eq!(db.game().xp_total(profile.id).await.expect("xp"), xp_before);

    // XP earned in the free-mode session did not turn that session's attempt into
    // evidence: the attempt still does not count.
    let rows = db.attempts().for_session(session.id).await.expect("rows");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].counts_toward_estimate);
    assert_eq!(rows[0].origin, AttemptOrigin::FreeMode);
}
