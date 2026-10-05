#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The schema refuses bad rows by itself. Every statement below is valid except
//! for the one column it overrides, and each is expected to fail with the kind of
//! error that column's constraint produces (so a typo in the SQL cannot make a
//! case pass for the wrong reason).

mod common;

use common::{raw_conn, temp_db};
use sqlx::{AssertSqlSafe, SqliteConnection};

/// Builds `INSERT INTO table (...) VALUES (...)` from a valid base row, replacing
/// or adding the columns in `overrides`. Values are SQL literals.
fn insert(table: &str, base: &[(&str, &str)], overrides: &[(&str, &str)]) -> String {
    let mut columns: Vec<(&str, &str)> = base.to_vec();
    for (name, value) in overrides {
        match columns.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value,
            None => columns.push((name, value)),
        }
    }
    let names: Vec<&str> = columns.iter().map(|(n, _)| *n).collect();
    let values: Vec<&str> = columns.iter().map(|(_, v)| *v).collect();
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        names.join(", "),
        values.join(", ")
    )
}

const T: &str = "'2026-01-01T00:00:00.000Z'";

fn profile(o: &[(&str, &str)]) -> String {
    insert("profiles", &[("display_name", "'p'"), ("created_at", T)], o)
}

fn session(o: &[(&str, &str)]) -> String {
    insert(
        "sessions",
        &[
            ("profile_id", "1"),
            ("kind", "'lesson'"),
            ("app_version", "'0'"),
            ("started_at", T),
        ],
        o,
    )
}

fn turn(o: &[(&str, &str)]) -> String {
    insert(
        "turns",
        &[
            ("session_id", "1"),
            ("seq", "2"),
            ("role", "'learner'"),
            ("input_mode", "'text'"),
            ("text", "'hi'"),
            ("created_at", T),
        ],
        o,
    )
}

fn attempt(o: &[(&str, &str)]) -> String {
    insert(
        "assessment_attempts",
        &[
            ("profile_id", "1"),
            ("activity_id", "'a'"),
            ("activity_type", "'t'"),
            ("response_id", "'r'"),
            ("level", "'A1'"),
            ("skill", "'writing'"),
            ("dimension", "'d'"),
            ("scorer", "'deterministic'"),
            ("scorer_version", "'v'"),
            ("status", "'scored'"),
            ("created_at", T),
        ],
        o,
    )
}

fn estimate(o: &[(&str, &str)]) -> String {
    insert(
        "skill_estimates",
        &[
            ("profile_id", "1"),
            ("skill", "'writing'"),
            ("status", "'estimated'"),
            ("evidence_count", "8"),
            ("algorithm_version", "'est/1'"),
            ("computed_at", T),
        ],
        o,
    )
}

fn xp(o: &[(&str, &str)]) -> String {
    insert(
        "xp_ledger",
        &[
            ("profile_id", "1"),
            ("created_at", T),
            ("source_kind", "'lesson'"),
            ("source_id", "'session:1'"),
            ("amount", "10"),
            ("reason", "'activity.completed'"),
        ],
        o,
    )
}

fn streak_day(o: &[(&str, &str)]) -> String {
    insert(
        "streak_days",
        &[
            ("profile_id", "1"),
            ("local_date", "'2026-10-05'"),
            ("counted", "1"),
            ("recorded_at", T),
        ],
        o,
    )
}

fn equipped(o: &[(&str, &str)]) -> String {
    insert(
        "equipped_cosmetics",
        &[("profile_id", "1"), ("updated_at", T)],
        o,
    )
}

/// One valid row of everything the cases point at.
const FIXTURE: &str = "\
INSERT INTO profiles (id, display_name, created_at) VALUES (1, 'p', '2026-01-01T00:00:00.000Z'); \
INSERT INTO sessions (id, profile_id, kind, app_version, started_at) \
  VALUES (1, 1, 'lesson', '0', '2026-01-01T00:00:00.000Z'); \
INSERT INTO turns (id, session_id, seq, role, input_mode, text, created_at) \
  VALUES (1, 1, 1, 'learner', 'text', 'hi', '2026-01-01T00:00:00.000Z'); \
INSERT INTO assessment_attempts (id, profile_id, activity_id, activity_type, response_id, level, \
  skill, dimension, scorer, scorer_version, status, created_at) \
  VALUES (1, 1, 'a', 't', 'r', 'A1', 'writing', 'd', 'deterministic', 'v', 'scored', '2026-01-01T00:00:00.000Z'); \
INSERT INTO curriculum_versions (id, content_version, schema_version, unit_count, manifest_sha256, installed_at) \
  VALUES (1, 'v', '1', 1, 'm', '2026-01-01T00:00:00.000Z'); \
INSERT INTO units (id, curriculum_version_id, level, sequence, title_en, file_sha256) \
  VALUES ('a1-u01', 1, 'A1', 1, 't', 's'); \
INSERT INTO unlockables (profile_id, unlockable_id, unlocked_at) \
  VALUES (1, 'accessory.beret', '2026-01-01T00:00:00.000Z'); \
INSERT INTO rest_tokens (id, profile_id, earned_at, reason) \
  VALUES (1, 1, '2026-01-01T00:00:00.000Z', 'welcome');";

async fn expect_refused(conn: &mut SqliteConnection, label: &str, sql: String, fragment: &str) {
    let result = sqlx::query(AssertSqlSafe(sql.clone()))
        .execute(&mut *conn)
        .await;
    let error = result.expect_err(&format!("{label} should be refused: {sql}"));
    let message = error.to_string();
    assert!(
        message.contains(fragment),
        "{label}: expected an error containing {fragment:?}, got {message:?}"
    );
}

#[tokio::test]
async fn check_unique_not_null_strict_and_foreign_key_constraints_refuse_bad_rows() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;
    sqlx::raw_sql(FIXTURE)
        .execute(&mut conn)
        .await
        .expect("fixture");

    const CHECK: &str = "CHECK constraint failed";
    const FK: &str = "FOREIGN KEY constraint failed";
    const UNIQUE: &str = "UNIQUE constraint failed";
    const NOT_NULL: &str = "NOT NULL constraint failed";
    const STRICT: &str = "cannot store";

    let cases: Vec<(&str, String, &str)> = vec![
        // Settings and profile
        ("profile language", profile(&[("ui_language", "'fr'")]), CHECK),
        ("profile l1 help", profile(&[("l1_help_mode", "'maybe'")]), CHECK),
        ("setting without a value", "INSERT INTO app_settings (key, value, updated_at) VALUES ('k', NULL, 't')".to_owned(), NOT_NULL),
        // Curriculum index
        ("unit level", insert("units", &[("id", "'x1'"), ("curriculum_version_id", "1"), ("level", "'A1'"), ("sequence", "2"), ("title_en", "'t'"), ("file_sha256", "'s'")], &[("level", "'D1'")]), CHECK),
        ("unit sequence below range", insert("units", &[("id", "'x1'"), ("curriculum_version_id", "1"), ("level", "'A1'"), ("sequence", "2"), ("title_en", "'t'"), ("file_sha256", "'s'")], &[("sequence", "0")]), CHECK),
        ("unit sequence above range", insert("units", &[("id", "'x1'"), ("curriculum_version_id", "1"), ("level", "'A1'"), ("sequence", "2"), ("title_en", "'t'"), ("file_sha256", "'s'")], &[("sequence", "31")]), CHECK),
        ("unit in a missing version", insert("units", &[("id", "'x1'"), ("curriculum_version_id", "1"), ("level", "'A1'"), ("sequence", "2"), ("title_en", "'t'"), ("file_sha256", "'s'")], &[("curriculum_version_id", "99")]), FK),
        ("two units in one slot", insert("units", &[("id", "'x1'"), ("curriculum_version_id", "1"), ("level", "'A1'"), ("sequence", "1"), ("title_en", "'t'"), ("file_sha256", "'s'")], &[]), UNIQUE),
        ("objective of a missing unit", "INSERT INTO objectives (id, unit_id, skill, can_do_en) VALUES ('nope/o1', 'nope', 'speaking', 'x')".to_owned(), FK),
        // Providers and models
        ("provider protocol", insert("provider_profiles", &[("name", "'n'"), ("protocol", "'openai_chat'"), ("base_url", "'u'"), ("model", "'m'"), ("created_at", T)], &[("protocol", "'grpc'")]), CHECK),
        ("provider key source", insert("provider_profiles", &[("name", "'n'"), ("protocol", "'openai_chat'"), ("base_url", "'u'"), ("model", "'m'"), ("created_at", T)], &[("key_source", "'keychain'")]), CHECK),
        ("provider capabilities not JSON", insert("provider_profiles", &[("name", "'n'"), ("protocol", "'openai_chat'"), ("base_url", "'u'"), ("model", "'m'"), ("created_at", T)], &[("capabilities_json", "'not json'")]), CHECK),
        ("provider active flag", insert("provider_profiles", &[("name", "'n'"), ("protocol", "'openai_chat'"), ("base_url", "'u'"), ("model", "'m'"), ("created_at", T)], &[("is_active", "2")]), CHECK),
        ("model role", insert("models_installed", &[("id", "'m'"), ("role", "'vad'"), ("version", "'1'"), ("path", "'p'"), ("sha256", "'s'"), ("size_bytes", "1"), ("license", "'MIT'"), ("installed_at", T)], &[("role", "'llm'")]), CHECK),
        // Conversation content
        ("session kind", session(&[("kind", "'quiz'")]), CHECK),
        ("session status", session(&[("status", "'paused'")]), CHECK),
        ("session mode", session(&[("mode", "'speed'")]), CHECK),
        ("session of a missing profile", session(&[("profile_id", "99")]), FK),
        ("session of a missing unit", session(&[("unit_id", "'nope'")]), FK),
        ("session summary not JSON", session(&[("summary_json", "'x'")]), CHECK),
        ("turn role", turn(&[("role", "'robot'")]), CHECK),
        ("turn input mode", turn(&[("input_mode", "'telepathy'")]), CHECK),
        ("turn edited flag", turn(&[("edited_by_learner", "2")]), CHECK),
        ("turn of a missing session", turn(&[("session_id", "99")]), FK),
        ("turn number reused", turn(&[("seq", "1")]), UNIQUE),
        ("turn text into an integer column", turn(&[("speech_ms", "'long'")]), STRICT),
        ("analysis ladder too low", insert("turn_analysis", &[("turn_id", "1"), ("analysis_json", "'{}'"), ("contract_version", "'c'"), ("ladder_level", "1"), ("model", "'m'"), ("created_at", T)], &[("ladder_level", "0")]), CHECK),
        ("analysis ladder too high", insert("turn_analysis", &[("turn_id", "1"), ("analysis_json", "'{}'"), ("contract_version", "'c'"), ("ladder_level", "1"), ("model", "'m'"), ("created_at", T)], &[("ladder_level", "5")]), CHECK),
        ("analysis not JSON", insert("turn_analysis", &[("turn_id", "1"), ("analysis_json", "'{}'"), ("contract_version", "'c'"), ("ladder_level", "1"), ("model", "'m'"), ("created_at", T)], &[("analysis_json", "'oops'")]), CHECK),
        ("analysis of a missing turn", insert("turn_analysis", &[("turn_id", "1"), ("analysis_json", "'{}'"), ("contract_version", "'c'"), ("ladder_level", "1"), ("model", "'m'"), ("created_at", T)], &[("turn_id", "99")]), FK),
        ("error severity", insert("error_events", &[("turn_id", "1"), ("profile_id", "1"), ("category", "'c'"), ("quote", "'q'"), ("correction", "'c'"), ("severity", "'minor'"), ("created_at", T)], &[("severity", "'fatal'")]), CHECK),
        ("generated kind", insert("generated_content", &[("session_id", "1"), ("kind", "'reading_passage'"), ("content_json", "'{}'"), ("contract_version", "'c'"), ("model", "'m'"), ("created_at", T)], &[("kind", "'quiz'")]), CHECK),
        ("generated content not JSON", insert("generated_content", &[("session_id", "1"), ("kind", "'reading_passage'"), ("content_json", "'{}'"), ("contract_version", "'c'"), ("model", "'m'"), ("created_at", T)], &[("content_json", "'oops'")]), CHECK),
        ("clip of a missing turn", insert("audio_clips", &[("turn_id", "99"), ("path", "'p'"), ("duration_ms", "1"), ("created_at", T)], &[]), FK),
        // Assessment data
        ("attempt origin", attempt(&[("origin", "'tutor'")]), CHECK),
        ("attempt level", attempt(&[("level", "'A0'")]), CHECK),
        ("attempt scorer", attempt(&[("scorer", "'human'")]), CHECK),
        ("attempt status", attempt(&[("status", "'done'")]), CHECK),
        ("attempt score above one", attempt(&[("normalized", "1.5")]), CHECK),
        ("attempt score below zero", attempt(&[("normalized", "-0.1")]), CHECK),
        ("attempt confidence above one", attempt(&[("confidence", "2")]), CHECK),
        ("attempt counting flag", attempt(&[("counts_toward_estimate", "2")]), CHECK),
        ("attempt of a missing profile", attempt(&[("profile_id", "99")]), FK),
        ("attempt of a missing session", attempt(&[("session_id", "99")]), FK),
        ("evidence kind", insert("assessment_evidence", &[("attempt_id", "1"), ("kind", "'quote'"), ("created_at", T)], &[("kind", "'screenshot'")]), CHECK),
        ("evidence of a missing attempt", insert("assessment_evidence", &[("attempt_id", "1"), ("kind", "'quote'"), ("created_at", T)], &[("attempt_id", "99")]), FK),
        ("evidence data not JSON", insert("assessment_evidence", &[("attempt_id", "1"), ("kind", "'metric'"), ("created_at", T)], &[("data_json", "'oops'")]), CHECK),
        ("pronunciation with neither attempt nor turn", insert("pron_results", &[("mode", "'drill'"), ("word", "'w'"), ("word_index", "0"), ("phone_expected", "'p'"), ("gop", "0.5"), ("flagged", "0"), ("start_ms", "0"), ("end_ms", "1"), ("engine_version", "'v'")], &[]), CHECK),
        ("pronunciation mode", insert("pron_results", &[("attempt_id", "1"), ("mode", "'drill'"), ("word", "'w'"), ("word_index", "0"), ("phone_expected", "'p'"), ("gop", "0.5"), ("flagged", "0"), ("start_ms", "0"), ("end_ms", "1"), ("engine_version", "'v'")], &[("mode", "'karaoke'")]), CHECK),
        ("estimate level", estimate(&[("level", "'A0'")]), CHECK),
        ("estimate level pre-A2", estimate(&[("level", "'pre-A2'")]), CHECK),
        ("estimate status", estimate(&[("status", "'maybe'")]), CHECK),
        ("estimate confidence", estimate(&[("confidence", "1.1")]), CHECK),
        ("estimate of a missing profile", estimate(&[("profile_id", "99")]), FK),
        ("mastery above one", insert("objective_mastery", &[("profile_id", "1"), ("objective_id", "'o'"), ("mastery", "1.5")], &[]), CHECK),
        ("review item kind", insert("review_schedule", &[("profile_id", "1"), ("item_kind", "'game'"), ("item_ref", "'r'"), ("due_at", T)], &[]), CHECK),
        ("unit progress status", insert("unit_progress", &[("profile_id", "1"), ("unit_id", "'u'"), ("status", "'done'"), ("updated_at", T)], &[]), CHECK),
        ("unit progress checkpoint", insert("unit_progress", &[("profile_id", "1"), ("unit_id", "'u'"), ("status", "'passed'"), ("best_checkpoint", "2"), ("updated_at", T)], &[]), CHECK),
        ("pending payload not JSON", insert("pending_scoring", &[("attempt_id", "1"), ("payload_json", "'oops'"), ("created_at", T)], &[]), CHECK),
        ("pending for a missing attempt", insert("pending_scoring", &[("attempt_id", "99"), ("payload_json", "'{}'"), ("created_at", T)], &[]), FK),
        // Technical logs
        ("call type", insert("llm_calls", &[("call_type", "'chat'"), ("model", "'m'"), ("outcome", "'ok'"), ("started_at", T)], &[]), CHECK),
        ("call outcome", insert("llm_calls", &[("call_type", "'probe'"), ("model", "'m'"), ("outcome", "'fine'"), ("started_at", T)], &[]), CHECK),
        ("call ladder", insert("llm_calls", &[("call_type", "'probe'"), ("model", "'m'"), ("outcome", "'ok'"), ("started_at", T), ("ladder_level", "7")], &[]), CHECK),
        ("sample text into a real column", insert("perf_samples", &[("metric", "'m'"), ("value_ms", "'fast'"), ("profile_tag", "'dev'"), ("created_at", T)], &[]), STRICT),
        // Game layer
        ("xp amount zero", xp(&[("amount", "0")]), CHECK),
        ("xp amount negative", xp(&[("amount", "-5")]), CHECK),
        ("xp amount huge", xp(&[("amount", "10001")]), CHECK),
        ("xp source kind", xp(&[("source_kind", "'quiz'")]), CHECK),
        ("xp source id with spaces", xp(&[("source_id", "'I went to the market'")]), CHECK),
        ("xp source id empty", xp(&[("source_id", "''")]), CHECK),
        ("xp reason with capitals and spaces", xp(&[("reason", "'Great job!'")]), CHECK),
        ("xp of a missing profile", xp(&[("profile_id", "99")]), FK),
        ("rest token reason with spaces", insert("rest_tokens", &[("profile_id", "1"), ("earned_at", T), ("reason", "'a b'")], &[]), CHECK),
        ("streak day that does not exist", streak_day(&[("local_date", "'2026-02-30'")]), CHECK),
        ("streak day not zero padded", streak_day(&[("local_date", "'2026-2-3'")]), CHECK),
        ("streak day with a time", streak_day(&[("local_date", "'2026-02-03T10:00'")]), CHECK),
        ("streak day counted flag", streak_day(&[("counted", "2")]), CHECK),
        ("active day that spends a token", streak_day(&[("rest_token_id", "1")]), CHECK),
        ("rest day without a token", streak_day(&[("counted", "0")]), CHECK),
        ("rest day with a missing token", streak_day(&[("counted", "0"), ("rest_token_id", "99")]), FK),
        ("cosmetic outside the two namespaces", insert("unlockables", &[("profile_id", "1"), ("unlocked_at", T)], &[("unlockable_id", "'badge.a1'")]), CHECK),
        ("cosmetic id with capitals", insert("unlockables", &[("profile_id", "1"), ("unlocked_at", T)], &[("unlockable_id", "'accessory.Beret'")]), CHECK),
        ("cosmetic id without a name", insert("unlockables", &[("profile_id", "1"), ("unlocked_at", T)], &[("unlockable_id", "'accessory.'")]), CHECK),
        ("cosmetic id with a space", insert("unlockables", &[("profile_id", "1"), ("unlocked_at", T)], &[("unlockable_id", "'accessory.a b'")]), CHECK),
        ("wearing something not unlocked", equipped(&[("mascot_accessory", "'accessory.crown'")]), FK),
        ("a theme in the accessory slot", equipped(&[("mascot_accessory", "'theme.sunset'")]), CHECK),
        ("an accessory in the theme slot", equipped(&[("theme", "'accessory.beret'")]), CHECK),
    ];

    for (label, sql, fragment) in cases {
        expect_refused(&mut conn, label, sql, fragment).await;
    }

    // The fixture itself is valid and the refused rows left nothing behind.
    let units: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM units")
        .fetch_one(&mut conn)
        .await
        .expect("count");
    let profiles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM profiles")
        .fetch_one(&mut conn)
        .await
        .expect("count");
    assert_eq!((units, profiles), (1, 1));
}

#[tokio::test]
async fn at_most_one_provider_can_be_active() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;
    let provider = |name: &str, active: &str| {
        insert(
            "provider_profiles",
            &[
                ("name", name),
                ("protocol", "'openai_chat'"),
                ("base_url", "'u'"),
                ("model", "'m'"),
                ("created_at", T),
            ],
            &[("is_active", active)],
        )
    };
    sqlx::query(AssertSqlSafe(provider("'a'", "1")))
        .execute(&mut conn)
        .await
        .expect("first active");
    sqlx::query(AssertSqlSafe(provider("'b'", "0")))
        .execute(&mut conn)
        .await
        .expect("inactive");
    expect_refused(
        &mut conn,
        "second active provider",
        provider("'c'", "1"),
        "UNIQUE constraint failed",
    )
    .await;
}

#[tokio::test]
async fn the_xp_ledger_cannot_be_edited_or_double_counted() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;
    sqlx::raw_sql(FIXTURE)
        .execute(&mut conn)
        .await
        .expect("fixture");
    sqlx::query(AssertSqlSafe(xp(&[])))
        .execute(&mut conn)
        .await
        .expect("first award");

    expect_refused(
        &mut conn,
        "same award twice",
        xp(&[]),
        "UNIQUE constraint failed",
    )
    .await;
    expect_refused(
        &mut conn,
        "editing a ledger row",
        "UPDATE xp_ledger SET amount = 9999".to_owned(),
        "xp_ledger rows are never changed",
    )
    .await;
    let total: i64 = sqlx::query_scalar("SELECT SUM(amount) FROM xp_ledger")
        .fetch_one(&mut conn)
        .await
        .expect("sum");
    assert_eq!(total, 10);
}

#[tokio::test]
async fn a_rest_token_covers_one_day_of_its_own_profile_only() {
    let (_dir, db) = temp_db().await;
    let mut conn = raw_conn(&db).await;
    sqlx::raw_sql(FIXTURE)
        .execute(&mut conn)
        .await
        .expect("fixture");
    sqlx::raw_sql(
        "INSERT INTO profiles (id, display_name, created_at) VALUES (2, 'other', '2026-01-01T00:00:00.000Z'); \
         INSERT INTO rest_tokens (id, profile_id, earned_at, reason) \
           VALUES (2, 1, '2026-01-01T00:00:00.000Z', 'spare');",
    )
    .execute(&mut conn)
    .await
    .expect("second profile");

    let rest_day = |profile: &str, date: &str, token: &str| {
        streak_day(&[
            ("profile_id", profile),
            ("local_date", date),
            ("counted", "0"),
            ("rest_token_id", token),
        ])
    };
    sqlx::query(AssertSqlSafe(rest_day("1", "'2026-10-04'", "1")))
        .execute(&mut conn)
        .await
        .expect("token 1 covers a day");
    expect_refused(
        &mut conn,
        "token reused on another day",
        rest_day("1", "'2026-10-06'", "1"),
        "UNIQUE constraint failed",
    )
    .await;
    expect_refused(
        &mut conn,
        "unspent token of another profile",
        rest_day("2", "'2026-10-04'", "2"),
        "FOREIGN KEY constraint failed",
    )
    .await;
}
