-- 0001_init.sql
-- Draft of the first migration. Copy to crates/storage/migrations/ when the storage
-- crate is created (ROADMAP task S4-01). Tested on SQLite 3.45.
--
-- Conventions
--   * The application opens every connection with: PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;
--   * Timestamps are UTC ISO-8601 text, for example 2026-10-04T08:15:30.123Z.
--   * Booleans are INTEGER 0 or 1.
--   * JSON columns are TEXT and must pass json_valid().
--   * API keys are never stored here. They come from the environment (.env) or from
--     providers.toml in the app data directory. This database only records which of the two.
--   * Three groups of tables are kept apart on purpose:
--       conversation content   (sessions, turns, turn_analysis, error_events, generated_content, audio_clips)
--       assessment data        (assessment_attempts, assessment_evidence, skill_estimates, ...)
--       technical logs         (llm_calls, perf_samples): these never hold learner text.

-- ---------------------------------------------------------------------------
-- Settings and profile
-- ---------------------------------------------------------------------------

CREATE TABLE app_settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
) STRICT;

-- v1 has exactly one row. The table and the profile_id columns exist so that
-- multi-profile support can be added later without a data migration.
CREATE TABLE profiles (
    id            INTEGER PRIMARY KEY,
    display_name  TEXT NOT NULL,
    ui_language   TEXT NOT NULL DEFAULT 'id' CHECK (ui_language IN ('id', 'en')),
    l1            TEXT NOT NULL DEFAULT 'id',
    l1_help_mode  TEXT NOT NULL DEFAULT 'auto' CHECK (l1_help_mode IN ('auto', 'on', 'off')),
    created_at    TEXT NOT NULL
) STRICT;

-- ---------------------------------------------------------------------------
-- Curriculum index (the content itself lives in JSON files)
-- ---------------------------------------------------------------------------

CREATE TABLE curriculum_versions (
    id               INTEGER PRIMARY KEY,
    content_version  TEXT NOT NULL UNIQUE,
    schema_version   TEXT NOT NULL,
    unit_count       INTEGER NOT NULL,
    manifest_sha256  TEXT NOT NULL,
    installed_at     TEXT NOT NULL
) STRICT;

CREATE TABLE units (
    id                     TEXT PRIMARY KEY,
    curriculum_version_id  INTEGER NOT NULL REFERENCES curriculum_versions (id),
    level                  TEXT NOT NULL CHECK (level IN ('A1', 'A2', 'B1', 'B2', 'C1', 'C2')),
    sequence               INTEGER NOT NULL CHECK (sequence BETWEEN 1 AND 30),
    title_en               TEXT NOT NULL,
    file_sha256            TEXT NOT NULL,
    UNIQUE (level, sequence)
) STRICT;

CREATE TABLE objectives (
    id         TEXT PRIMARY KEY,            -- "<unit id>/<objective id>"
    unit_id    TEXT NOT NULL REFERENCES units (id) ON DELETE CASCADE,
    skill      TEXT NOT NULL,
    can_do_en  TEXT NOT NULL
) STRICT;

CREATE INDEX idx_objectives_unit ON objectives (unit_id);

-- ---------------------------------------------------------------------------
-- Providers and local models
-- ---------------------------------------------------------------------------

CREATE TABLE provider_profiles (
    id                 INTEGER PRIMARY KEY,
    name               TEXT NOT NULL UNIQUE,
    protocol           TEXT NOT NULL CHECK (protocol IN ('openai_chat', 'anthropic_messages')),
    base_url           TEXT NOT NULL,
    model              TEXT NOT NULL,
    key_source         TEXT NOT NULL DEFAULT 'file' CHECK (key_source IN ('env', 'file')),  -- where the key is read from, never the key itself
    capabilities_json  TEXT CHECK (capabilities_json IS NULL OR json_valid(capabilities_json)),
    probed_at          TEXT,
    qualified_at       TEXT,                -- set when the scorer qualification test passed
    is_active          INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    created_at         TEXT NOT NULL
) STRICT;

-- At most one active provider profile.
CREATE UNIQUE INDEX idx_provider_single_active ON provider_profiles (is_active) WHERE is_active = 1;

CREATE TABLE models_installed (
    id            TEXT PRIMARY KEY,         -- id from models/manifest.toml
    role          TEXT NOT NULL CHECK (role IN ('vad', 'stt', 'tts', 'pron')),
    version       TEXT NOT NULL,
    path          TEXT NOT NULL,
    sha256        TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL,
    license       TEXT NOT NULL,
    installed_at  TEXT NOT NULL
) STRICT;

-- ---------------------------------------------------------------------------
-- Conversation content
-- ---------------------------------------------------------------------------

CREATE TABLE sessions (
    id                   INTEGER PRIMARY KEY,
    profile_id           INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    kind                 TEXT NOT NULL CHECK (kind IN ('lesson', 'conversation', 'text_chat', 'writing', 'reading', 'drill', 'review', 'checkpoint', 'placement')),
    unit_id              TEXT REFERENCES units (id),
    activity_id          TEXT,
    mode                 TEXT CHECK (mode IS NULL OR mode IN ('fluency', 'accuracy')),
    status               TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'completed', 'aborted')),
    provider_profile_id  INTEGER REFERENCES provider_profiles (id) ON DELETE SET NULL,
    summary_json         TEXT CHECK (summary_json IS NULL OR json_valid(summary_json)),
    app_version          TEXT NOT NULL,
    started_at           TEXT NOT NULL,
    ended_at             TEXT
) STRICT;

CREATE INDEX idx_sessions_profile_started ON sessions (profile_id, started_at);

CREATE TABLE turns (
    id                 INTEGER PRIMARY KEY,
    session_id         INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    seq                INTEGER NOT NULL,
    role               TEXT NOT NULL CHECK (role IN ('learner', 'tutor')),
    input_mode         TEXT NOT NULL CHECK (input_mode IN ('voice', 'text', 'none')),
    text               TEXT NOT NULL,
    stt_text           TEXT,               -- original STT output when the learner corrected the transcript
    edited_by_learner  INTEGER NOT NULL DEFAULT 0 CHECK (edited_by_learner IN (0, 1)),
    speech_ms          INTEGER,            -- voiced time inside the turn, from the VAD
    pause_ms           INTEGER,            -- silent time inside the turn, from the VAD
    word_count         INTEGER,
    created_at         TEXT NOT NULL,
    UNIQUE (session_id, seq)
) STRICT;

CREATE TABLE turn_analysis (
    turn_id              INTEGER PRIMARY KEY REFERENCES turns (id) ON DELETE CASCADE,  -- a spoken turn, a chat message, or a writing draft
    analysis_json        TEXT NOT NULL CHECK (json_valid(analysis_json)),
    contract_version     TEXT NOT NULL,
    ladder_level         INTEGER NOT NULL CHECK (ladder_level BETWEEN 1 AND 4),
    model                TEXT NOT NULL,
    created_at           TEXT NOT NULL
) STRICT;

CREATE TABLE error_events (
    id          INTEGER PRIMARY KEY,
    turn_id     INTEGER NOT NULL REFERENCES turns (id) ON DELETE CASCADE,
    profile_id  INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    category    TEXT NOT NULL,
    quote       TEXT NOT NULL,
    correction  TEXT NOT NULL,
    severity    TEXT NOT NULL CHECK (severity IN ('minor', 'major', 'blocking')),
    addressed   INTEGER NOT NULL DEFAULT 0 CHECK (addressed IN (0, 1)),
    created_at  TEXT NOT NULL
) STRICT;

CREATE INDEX idx_error_events_turn ON error_events (turn_id);

-- Content the LLM generated for a session: graded reading passages and extra practice items.
-- Kept so a learner can reopen a text and so every generated item can be traced to its model.
CREATE TABLE generated_content (
    id                INTEGER PRIMARY KEY,
    session_id        INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    kind              TEXT NOT NULL CHECK (kind IN ('reading_passage', 'practice_items')),
    content_json      TEXT NOT NULL CHECK (json_valid(content_json)),
    contract_version  TEXT NOT NULL,
    model             TEXT NOT NULL,
    created_at        TEXT NOT NULL
) STRICT;

CREATE INDEX idx_generated_session ON generated_content (session_id);

-- Only filled when the learner opts in to keeping recordings.
CREATE TABLE audio_clips (
    id           INTEGER PRIMARY KEY,
    turn_id      INTEGER NOT NULL REFERENCES turns (id) ON DELETE CASCADE,
    path         TEXT NOT NULL,
    duration_ms  INTEGER NOT NULL,
    created_at   TEXT NOT NULL
) STRICT;

-- ---------------------------------------------------------------------------
-- Assessment data
-- ---------------------------------------------------------------------------

CREATE TABLE assessment_attempts (
    id                      INTEGER PRIMARY KEY,
    profile_id              INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    session_id              INTEGER REFERENCES sessions (id) ON DELETE SET NULL,
    unit_id                 TEXT,
    activity_id             TEXT NOT NULL,
    activity_type           TEXT NOT NULL,
    response_id             TEXT NOT NULL,   -- one id per submitted response; its dimension rows share it
    origin                  TEXT NOT NULL DEFAULT 'authored' CHECK (origin IN ('authored', 'generated', 'free_mode')),
    level                   TEXT NOT NULL CHECK (level IN ('A1', 'A2', 'B1', 'B2', 'C1', 'C2')),
    skill                   TEXT NOT NULL,
    dimension               TEXT NOT NULL,
    scorer                  TEXT NOT NULL CHECK (scorer IN ('deterministic', 'rubric_llm', 'pron_engine')),
    scorer_version          TEXT NOT NULL,   -- rubric id and version, contract version, model id, or engine version
    raw_score               REAL,
    max_score               REAL,
    normalized              REAL CHECK (normalized IS NULL OR (normalized >= 0 AND normalized <= 1)),
    confidence              REAL CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    status                  TEXT NOT NULL CHECK (status IN ('scored', 'pending_llm', 'insufficient', 'needs_review', 'rejected')),
    counts_toward_estimate  INTEGER NOT NULL DEFAULT 1 CHECK (counts_toward_estimate IN (0, 1)),
    created_at              TEXT NOT NULL
) STRICT;

CREATE INDEX idx_attempts_profile_skill ON assessment_attempts (profile_id, skill, level, created_at);
CREATE INDEX idx_attempts_session ON assessment_attempts (session_id);
CREATE INDEX idx_attempts_response ON assessment_attempts (response_id);

CREATE TABLE assessment_evidence (
    id          INTEGER PRIMARY KEY,
    attempt_id  INTEGER NOT NULL REFERENCES assessment_attempts (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('response_text', 'quote', 'metric', 'scorer_reason')),
    content     TEXT,
    data_json   TEXT CHECK (data_json IS NULL OR json_valid(data_json)),
    created_at  TEXT NOT NULL
) STRICT;

CREATE INDEX idx_evidence_attempt ON assessment_evidence (attempt_id);

CREATE TABLE pron_results (
    id              INTEGER PRIMARY KEY,
    attempt_id      INTEGER REFERENCES assessment_attempts (id) ON DELETE CASCADE,
    turn_id         INTEGER REFERENCES turns (id) ON DELETE CASCADE,
    mode            TEXT NOT NULL CHECK (mode IN ('drill', 'free_speech')),
    word            TEXT NOT NULL,
    word_index      INTEGER NOT NULL,
    phone_expected  TEXT NOT NULL,
    phone_heard     TEXT,
    gop             REAL NOT NULL,
    flagged         INTEGER NOT NULL CHECK (flagged IN (0, 1)),
    start_ms        INTEGER NOT NULL,
    end_ms          INTEGER NOT NULL,
    engine_version  TEXT NOT NULL,
    CHECK (attempt_id IS NOT NULL OR turn_id IS NOT NULL)
) STRICT;

CREATE INDEX idx_pron_attempt ON pron_results (attempt_id);
CREATE INDEX idx_pron_turn ON pron_results (turn_id);

-- History is kept. The current estimate for a skill is the row with the latest computed_at.
CREATE TABLE skill_estimates (
    id                 INTEGER PRIMARY KEY,
    profile_id         INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    skill              TEXT NOT NULL,
    level              TEXT CHECK (level IS NULL OR level IN ('pre-A1', 'A1', 'A2', 'B1', 'B2', 'C1', 'C2')),
    status             TEXT NOT NULL CHECK (status IN ('estimated', 'insufficient_evidence', 'placement_only')),
    confidence         REAL CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    evidence_count     INTEGER NOT NULL,
    algorithm_version  TEXT NOT NULL,
    detail_json        TEXT CHECK (detail_json IS NULL OR json_valid(detail_json)),
    computed_at        TEXT NOT NULL
) STRICT;

CREATE INDEX idx_estimates_profile_skill ON skill_estimates (profile_id, skill, computed_at);

CREATE TABLE objective_mastery (
    profile_id       INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    objective_id     TEXT NOT NULL,
    mastery          REAL NOT NULL CHECK (mastery >= 0 AND mastery <= 1),
    attempts         INTEGER NOT NULL DEFAULT 0,
    last_attempt_at  TEXT,
    PRIMARY KEY (profile_id, objective_id)
) STRICT;

-- Numbers only, so the pattern survives when a session's text is deleted.
CREATE TABLE error_stats (
    profile_id  INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    category    TEXT NOT NULL,
    count       INTEGER NOT NULL DEFAULT 0,
    last_seen   TEXT,
    PRIMARY KEY (profile_id, category)
) STRICT;

CREATE TABLE review_schedule (
    id                INTEGER PRIMARY KEY,
    profile_id        INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    item_kind         TEXT NOT NULL CHECK (item_kind IN ('vocab', 'grammar', 'pron')),
    item_ref          TEXT NOT NULL,       -- "<unit id>/<item id>"
    due_at            TEXT NOT NULL,
    interval_days     REAL NOT NULL DEFAULT 0,
    ease              REAL NOT NULL DEFAULT 2.5,
    reps              INTEGER NOT NULL DEFAULT 0,
    lapses            INTEGER NOT NULL DEFAULT 0,
    last_reviewed_at  TEXT,
    UNIQUE (profile_id, item_kind, item_ref)
) STRICT;

CREATE INDEX idx_review_due ON review_schedule (profile_id, due_at);

CREATE TABLE unit_progress (
    profile_id        INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    unit_id           TEXT NOT NULL,
    status            TEXT NOT NULL CHECK (status IN ('locked', 'available', 'in_progress', 'passed', 'skipped')),
    best_checkpoint   REAL CHECK (best_checkpoint IS NULL OR (best_checkpoint >= 0 AND best_checkpoint <= 1)),
    updated_at        TEXT NOT NULL,
    PRIMARY KEY (profile_id, unit_id)
) STRICT;

-- Productive responses written while offline wait here until a provider is reachable.
CREATE TABLE pending_scoring (
    id            INTEGER PRIMARY KEY,
    attempt_id    INTEGER NOT NULL UNIQUE REFERENCES assessment_attempts (id) ON DELETE CASCADE,
    payload_json  TEXT NOT NULL CHECK (json_valid(payload_json)),
    tries         INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
) STRICT;

-- ---------------------------------------------------------------------------
-- Technical logs. No learner text, no prompts, no responses.
-- ---------------------------------------------------------------------------

CREATE TABLE llm_calls (
    id                   INTEGER PRIMARY KEY,
    provider_profile_id  INTEGER REFERENCES provider_profiles (id) ON DELETE SET NULL,
    call_type            TEXT NOT NULL CHECK (call_type IN ('tutor_turn', 'turn_analysis', 'rubric_score', 'practice_gen', 'reading_gen', 'explain', 'probe')),
    model                TEXT NOT NULL,
    ladder_level         INTEGER CHECK (ladder_level IS NULL OR ladder_level BETWEEN 1 AND 4),
    ttft_ms              INTEGER,
    total_ms             INTEGER,
    input_tokens         INTEGER,
    output_tokens        INTEGER,
    http_status          INTEGER,
    outcome              TEXT NOT NULL CHECK (outcome IN ('ok', 'repaired', 'invalid_output', 'timeout', 'rate_limited', 'refused', 'cancelled', 'error')),
    started_at           TEXT NOT NULL
) STRICT;

CREATE INDEX idx_llm_calls_started ON llm_calls (started_at);

CREATE TABLE perf_samples (
    id           INTEGER PRIMARY KEY,
    session_id   INTEGER REFERENCES sessions (id) ON DELETE SET NULL,
    turn_seq     INTEGER,
    metric       TEXT NOT NULL,            -- for example endpoint_ms, stt_ms, llm_first_sentence_ms, tts_first_ms, e2e_ms
    value_ms     REAL NOT NULL,
    profile_tag  TEXT NOT NULL,            -- hardware profile label, for example dev or floor
    created_at   TEXT NOT NULL
) STRICT;

CREATE INDEX idx_perf_metric ON perf_samples (metric, created_at);

-- ---------------------------------------------------------------------------
-- Deleting a session removes its text everywhere but keeps the numeric scores.
-- turns, turn_analysis, error_events, generated_content, audio_clips and free-speech pron_results go
-- through ON DELETE CASCADE. Evidence text of attempts made in that session is
-- removed by this trigger. The attempt rows stay, with session_id set to NULL.
-- The application must also delete the audio files listed in audio_clips before
-- it deletes the session row.
-- ---------------------------------------------------------------------------

CREATE TRIGGER trg_sessions_delete_evidence
BEFORE DELETE ON sessions
FOR EACH ROW
BEGIN
    DELETE FROM assessment_evidence
    WHERE attempt_id IN (SELECT id FROM assessment_attempts WHERE session_id = OLD.id);
    DELETE FROM pending_scoring
    WHERE attempt_id IN (SELECT id FROM assessment_attempts WHERE session_id = OLD.id);
END;
