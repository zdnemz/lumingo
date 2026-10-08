-- 0002_game.sql
-- The game layer: XP, streaks, rest-day tokens, unlockables and equipped cosmetics.
--
-- It is purely cosmetic. Nothing here says anything about how well the learner
-- speaks English, and nothing here can be used to say so.
--
-- Rules this file encodes (the repository in src/game.rs enforces the same rules
-- before a statement reaches the database; the constraints below are the backstop):
--
--   1. Separate from assessment. No table below has a foreign key to or from
--      assessment_attempts, assessment_evidence, skill_estimates or any other
--      assessment table, and no trigger below reads or writes one. The only table
--      outside this layer that a game table references is profiles, the root every
--      learner table hangs from, so "delete all learning data" (delete the profile
--      row) removes the game data together with everything else.
--
--   2. Nothing can be read as a CEFR level. No table or column is called level,
--      score, estimate, skill, mastery or evidence, no value is a CEFR code, and
--      there is no "player level" derived from XP. XP is an open-ended count.
--
--   3. Free-mode activity may earn XP, and every XP row says where it came from in
--      source_kind (the same kinds as sessions.kind, plus 'bonus'). A game row
--      never enters the evidence path: source_id is an opaque label, not a key
--      into any table, so no join connects an XP row to an attempt, and deleting a
--      session leaves the ledger as it was. Because the ledger outlives sessions,
--      source_id and reason are short codes (letters, digits and . _ : / -), never
--      learner text; CHECK constraints refuse anything longer or with spaces.
--
--   4. No clock and no time zone. The caller supplies every timestamp (UTC
--      ISO-8601 text, like the rest of the schema) and every local calendar day
--      (YYYY-MM-DD). No default, trigger or check below asks SQLite for the
--      current time or converts between zones, so streak results depend only on
--      the dates the caller passes in.
--
--   5. Nothing punitive. XP amounts are positive and a ledger row can never be
--      changed. A streak is counted from the days stored: a missed day without a
--      rest-day token simply means the next active day starts a new count at one.
--      The count cannot go negative, and no table records a lost streak, a missed
--      day or a penalty. Rest-day tokens never expire.
--
-- Tables
--   xp_ledger          One row per XP award. Append-only. Total XP is a SUM.
--   rest_tokens        Rest-day (streak freeze) tokens the learner has earned.
--   streak_days        One row per local day that belongs to a streak. counted = 1:
--                      the learner was active that day and it adds one to the count.
--                      counted = 0: a rest day, covered by exactly one token; it
--                      keeps the chain unbroken and adds nothing to the count.
--   unlockables        Cosmetics the learner has unlocked, by catalogue id. The
--                      catalogue (names, art, how to unlock) ships with the program;
--                      only ids that were unlocked are stored. Ids are namespaced:
--                      'accessory.<name>' for the mascot, 'theme.<name>' for themes.
--   equipped_cosmetics The mascot accessory and theme currently worn, at most one
--                      row per profile. Each must be unlocked (composite foreign key).

CREATE TABLE xp_ledger (
    id           INTEGER PRIMARY KEY,
    profile_id   INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    created_at   TEXT NOT NULL,
    source_kind  TEXT NOT NULL CHECK (source_kind IN ('lesson', 'conversation', 'text_chat', 'writing', 'reading', 'drill', 'review', 'checkpoint', 'placement', 'bonus')),
    source_id    TEXT NOT NULL CHECK (length(source_id) BETWEEN 1 AND 64 AND source_id NOT GLOB '*[^A-Za-z0-9._:/-]*'),
    amount       INTEGER NOT NULL CHECK (amount BETWEEN 1 AND 10000),
    reason       TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 64 AND reason NOT GLOB '*[^a-z0-9._-]*'),
    -- The same award cannot be recorded twice, so a retry never doubles XP.
    UNIQUE (profile_id, source_kind, source_id, reason)
) STRICT;

CREATE INDEX idx_xp_ledger_profile_time ON xp_ledger (profile_id, created_at);

CREATE TRIGGER trg_xp_ledger_append_only
BEFORE UPDATE ON xp_ledger
FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'xp_ledger rows are never changed');
END;

CREATE TABLE rest_tokens (
    id          INTEGER PRIMARY KEY,
    profile_id  INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    earned_at   TEXT NOT NULL,
    reason      TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 64 AND reason NOT GLOB '*[^a-z0-9._-]*'),
    UNIQUE (profile_id, id)
) STRICT;

CREATE TABLE streak_days (
    profile_id     INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    local_date     TEXT NOT NULL CHECK (local_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]' AND date(local_date) = local_date),
    counted        INTEGER NOT NULL CHECK (counted IN (0, 1)),
    rest_token_id  INTEGER UNIQUE,
    recorded_at    TEXT NOT NULL,
    PRIMARY KEY (profile_id, local_date),
    -- An active day spends no token. A rest day spends exactly one, once.
    CHECK ((counted = 1) = (rest_token_id IS NULL)),
    FOREIGN KEY (profile_id, rest_token_id) REFERENCES rest_tokens (profile_id, id)
) STRICT;

CREATE TABLE unlockables (
    profile_id     INTEGER NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    unlockable_id  TEXT NOT NULL CHECK (
        length(unlockable_id) <= 64
        AND unlockable_id NOT GLOB '*[^a-z0-9._]*'
        AND (unlockable_id GLOB 'accessory.?*' OR unlockable_id GLOB 'theme.?*')
    ),
    unlocked_at    TEXT NOT NULL,
    PRIMARY KEY (profile_id, unlockable_id)
) STRICT;

CREATE TABLE equipped_cosmetics (
    profile_id        INTEGER PRIMARY KEY REFERENCES profiles (id) ON DELETE CASCADE,
    mascot_accessory  TEXT CHECK (mascot_accessory IS NULL OR mascot_accessory GLOB 'accessory.?*'),
    theme             TEXT CHECK (theme IS NULL OR theme GLOB 'theme.?*'),
    updated_at        TEXT NOT NULL,
    -- A NULL slot is not checked, so nothing equipped is allowed; anything worn
    -- must be an unlocked item of the same profile.
    FOREIGN KEY (profile_id, mascot_accessory) REFERENCES unlockables (profile_id, unlockable_id),
    FOREIGN KEY (profile_id, theme) REFERENCES unlockables (profile_id, unlockable_id)
) STRICT;
