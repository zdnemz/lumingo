//! The game layer: XP, streaks, rest-day tokens and cosmetics.
//!
//! It is purely cosmetic and kept apart from assessment on purpose (see the
//! header of `migrations/0002_game.sql`). Nothing here reads or writes attempts,
//! evidence or estimates, nothing here is a CEFR level, and there is no state
//! that punishes the learner: XP only goes up, and a missed day only means the
//! next active day starts a new count.
//!
//! Streaks are computed from calendar days the caller supplies. This module never
//! looks at the clock or the time zone.

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::db::Database;
use crate::enums::XpSourceKind;
use crate::error::{Result, StorageError};
use crate::row::{enum_col, timestamp_col};
use crate::streak::{Day, StreakStatus, evaluate};
use crate::time::{LocalDate, Timestamp};

/// Largest XP award in one row. Matches the CHECK in the schema.
pub const MAX_XP_AWARD: i64 = 10_000;

const MAX_CODE_LEN: usize = 64;

/// `source_id` and `reason` outlive sessions, so they are short codes and never
/// learner text. The same limits are CHECK constraints in the schema.
fn check_code(value: &str, what: &'static str, allowed: impl Fn(char) -> bool) -> Result<()> {
    if value.is_empty() || value.len() > MAX_CODE_LEN || !value.chars().all(allowed) {
        return Err(StorageError::Rule(what));
    }
    Ok(())
}

fn check_source_id(value: &str) -> Result<()> {
    check_code(
        value,
        "an XP source id is a short code of letters, digits and . _ : / -",
        |c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'),
    )
}

fn check_reason(value: &str) -> Result<()> {
    check_code(
        value,
        "a reason is a short lowercase code of letters, digits and . _ -",
        |c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'),
    )
}

/// Unlockable ids are `accessory.<name>` or `theme.<name>` in lowercase.
fn check_unlockable_id(value: &str) -> Result<()> {
    check_code(
        value,
        "a cosmetic id is lowercase letters, digits, . and _",
        |c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_'),
    )?;
    let named = |prefix: &str| {
        value
            .strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty())
    };
    if named("accessory.") || named("theme.") {
        Ok(())
    } else {
        Err(StorageError::Rule(
            "a cosmetic id starts with accessory. or theme. and has a name",
        ))
    }
}

/// An XP award to record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewXp {
    pub profile_id: i64,
    pub created_at: Timestamp,
    /// Where the XP came from. Free-mode activity may earn XP.
    pub source_kind: XpSourceKind,
    /// An opaque label for the source, such as `session:42`. Not a key into any
    /// table.
    pub source_id: String,
    /// XP awarded, 1 to [`MAX_XP_AWARD`].
    pub amount: i64,
    /// A short code the UI maps to a sentence, such as `activity.completed`.
    pub reason: String,
}

/// A recorded XP award.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XpEntry {
    pub id: i64,
    pub profile_id: i64,
    pub created_at: Timestamp,
    pub source_kind: XpSourceKind,
    pub source_id: String,
    pub amount: i64,
    pub reason: String,
}

/// What [`Game::award_xp`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum XpAward {
    /// The award was new and is now in the ledger.
    Awarded(XpEntry),
    /// The same source and reason were already recorded, so nothing changed.
    AlreadyAwarded,
}

/// XP earned from one kind of source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct XpBySource {
    pub source_kind: XpSourceKind,
    pub amount: i64,
}

/// XP totals of a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XpTotals {
    pub total: i64,
    /// One entry per source kind that has earned XP, in the order of
    /// [`XpSourceKind::ALL`].
    pub by_source: Vec<XpBySource>,
}

/// A rest-day (streak freeze) token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestToken {
    pub id: i64,
    pub profile_id: i64,
    pub earned_at: Timestamp,
    pub reason: String,
}

/// One day that belongs to a streak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreakDay {
    pub local_date: LocalDate,
    /// True for an active day, false for a rest day covered by a token.
    pub counted: bool,
    pub recorded_at: Timestamp,
}

/// What [`Game::record_activity`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRecorded {
    /// False when the day was already recorded: the call changed nothing.
    pub newly_recorded: bool,
    /// Rest-day tokens used to keep the streak alive across missed days.
    pub tokens_spent: u32,
    /// The streak after this call.
    pub status: StreakStatus,
}

/// An unlocked cosmetic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unlocked {
    pub unlockable_id: String,
    pub unlocked_at: Timestamp,
}

/// The cosmetics currently worn. `None` in a slot means nothing is worn there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquippedCosmetics {
    pub profile_id: i64,
    pub mascot_accessory: Option<String>,
    pub theme: Option<String>,
    pub updated_at: Timestamp,
}

impl XpEntry {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            profile_id: row.try_get("profile_id")?,
            created_at: timestamp_col(row, "created_at")?,
            source_kind: enum_col(row, "source_kind")?,
            source_id: row.try_get("source_id")?,
            amount: row.try_get("amount")?,
            reason: row.try_get("reason")?,
        })
    }
}

impl RestToken {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            profile_id: row.try_get("profile_id")?,
            earned_at: timestamp_col(row, "earned_at")?,
            reason: row.try_get("reason")?,
        })
    }
}

/// Queries on the game tables.
pub struct Game<'a> {
    db: &'a Database,
}

impl Database {
    /// Game-layer queries.
    pub fn game(&self) -> Game<'_> {
        Game { db: self }
    }
}

impl Game<'_> {
    // ---- XP -------------------------------------------------------------

    /// Records an XP award. Recording the same source and reason again is not an
    /// error and adds nothing, so a retry never doubles XP.
    pub async fn award_xp(&self, new: &NewXp) -> Result<XpAward> {
        check_source_id(&new.source_id)?;
        check_reason(&new.reason)?;
        if !(1..=MAX_XP_AWARD).contains(&new.amount) {
            return Err(StorageError::Rule("an XP award is between 1 and 10000"));
        }
        let row = sqlx::query(
            "INSERT INTO xp_ledger (profile_id, created_at, source_kind, source_id, amount, reason) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (profile_id, source_kind, source_id, reason) DO NOTHING RETURNING *",
        )
        .bind(new.profile_id)
        .bind(new.created_at.to_string())
        .bind(new.source_kind.as_str())
        .bind(&new.source_id)
        .bind(new.amount)
        .bind(&new.reason)
        .fetch_optional(self.db.writer())
        .await?;
        match row {
            Some(row) => Ok(XpAward::Awarded(XpEntry::from_row(&row)?)),
            None => Ok(XpAward::AlreadyAwarded),
        }
    }

    /// Total XP of a profile.
    pub async fn xp_total(&self, profile_id: i64) -> Result<i64> {
        let total = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM xp_ledger WHERE profile_id = ?1",
        )
        .bind(profile_id)
        .fetch_one(self.db.reader())
        .await?;
        Ok(total)
    }

    /// XP earned at or after `since`.
    pub async fn xp_since(&self, profile_id: i64, since: &Timestamp) -> Result<i64> {
        let total = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM xp_ledger WHERE profile_id = ?1 AND created_at >= ?2",
        )
        .bind(profile_id)
        .bind(since.to_string())
        .fetch_one(self.db.reader())
        .await?;
        Ok(total)
    }

    /// The total and the share of each kind of source.
    pub async fn xp_totals(&self, profile_id: i64) -> Result<XpTotals> {
        let rows = sqlx::query(
            "SELECT source_kind, SUM(amount) AS amount FROM xp_ledger WHERE profile_id = ?1 \
             GROUP BY source_kind",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        let mut sums = Vec::with_capacity(rows.len());
        for row in &rows {
            sums.push(XpBySource {
                source_kind: enum_col(row, "source_kind")?,
                amount: row.try_get("amount")?,
            });
        }
        let by_source: Vec<XpBySource> = XpSourceKind::ALL
            .iter()
            .filter_map(|kind| sums.iter().find(|s| s.source_kind == *kind).copied())
            .collect();
        let total = by_source.iter().map(|s| s.amount).sum();
        Ok(XpTotals { total, by_source })
    }

    /// The latest awards, newest first.
    pub async fn xp_entries(&self, profile_id: i64, limit: u32) -> Result<Vec<XpEntry>> {
        let rows = sqlx::query(
            "SELECT * FROM xp_ledger WHERE profile_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
        )
        .bind(profile_id)
        .bind(i64::from(limit))
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(XpEntry::from_row).collect()
    }

    // ---- Streaks --------------------------------------------------------

    /// Grants a rest-day token. The caller decides when tokens are earned and
    /// guards against granting the same one twice.
    pub async fn grant_rest_token(
        &self,
        profile_id: i64,
        earned_at: &Timestamp,
        reason: &str,
    ) -> Result<RestToken> {
        check_reason(reason)?;
        let row = sqlx::query(
            "INSERT INTO rest_tokens (profile_id, earned_at, reason) VALUES (?1, ?2, ?3) RETURNING *",
        )
        .bind(profile_id)
        .bind(earned_at.to_string())
        .bind(reason)
        .fetch_one(self.db.writer())
        .await?;
        RestToken::from_row(&row)
    }

    /// The tokens the learner holds and has not used, oldest first.
    pub async fn available_rest_tokens(&self, profile_id: i64) -> Result<Vec<RestToken>> {
        let rows = sqlx::query(
            "SELECT t.* FROM rest_tokens AS t \
             LEFT JOIN streak_days AS s ON s.rest_token_id = t.id \
             WHERE t.profile_id = ?1 AND s.rest_token_id IS NULL ORDER BY t.earned_at, t.id",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(RestToken::from_row).collect()
    }

    /// Where the streak stands on `today`, a calendar day the caller chose.
    pub async fn streak_status(&self, profile_id: i64, today: LocalDate) -> Result<StreakStatus> {
        let mut conn = self.db.reader().acquire().await?;
        status_on(&mut conn, profile_id, today).await
    }

    /// Records that the learner was active on `today`.
    ///
    /// Calling it again for the same day changes nothing. When days were missed
    /// since the last active day, rest-day tokens (oldest first) cover them if
    /// there are enough for every missed day; otherwise none are used and the
    /// count restarts at one. A day earlier than the newest recorded day is
    /// stored as an active day without touching tokens.
    pub async fn record_activity(
        &self,
        profile_id: i64,
        today: LocalDate,
        now: &Timestamp,
    ) -> Result<ActivityRecorded> {
        let mut tx = self.db.begin_write().await?;
        let already: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM streak_days WHERE profile_id = ?1 AND local_date = ?2",
        )
        .bind(profile_id)
        .bind(today.to_string())
        .fetch_optional(&mut *tx)
        .await?;

        let mut tokens_spent = 0;
        if already.is_none() {
            let days = load_days(&mut tx, profile_id).await?;
            let newest = days.iter().map(|d| d.date).max();
            if let Some(last) = newest.filter(|newest| *newest < today) {
                let missed = today.days_since(last) - 1;
                if missed >= 1 {
                    let tokens = unspent_token_ids(&mut tx, profile_id).await?;
                    if i64::try_from(tokens.len()).is_ok_and(|held| held >= missed) {
                        for (offset, token_id) in (1..=missed).zip(tokens) {
                            let date = u32::try_from(offset)
                                .ok()
                                .and_then(|n| last.plus_days(n))
                                .ok_or(StorageError::Rule("the date is out of range"))?;
                            insert_day(&mut tx, profile_id, date, Some(token_id), now).await?;
                            tokens_spent += 1;
                        }
                    }
                }
            }
            insert_day(&mut tx, profile_id, today, None, now).await?;
        }
        let status = status_on(&mut tx, profile_id, today).await?;
        tx.commit().await?;
        Ok(ActivityRecorded {
            newly_recorded: already.is_none(),
            tokens_spent,
            status,
        })
    }

    /// Every stored streak day, oldest first, for a calendar view.
    pub async fn streak_days(&self, profile_id: i64) -> Result<Vec<StreakDay>> {
        let rows = sqlx::query(
            "SELECT local_date, counted, recorded_at FROM streak_days WHERE profile_id = ?1 \
             ORDER BY local_date",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter()
            .map(|row| {
                let text: String = row.try_get("local_date")?;
                Ok(StreakDay {
                    local_date: LocalDate::parse(&text)?,
                    counted: row.try_get("counted")?,
                    recorded_at: timestamp_col(row, "recorded_at")?,
                })
            })
            .collect()
    }

    // ---- Cosmetics ------------------------------------------------------

    /// Unlocks a cosmetic. Returns whether it was new.
    pub async fn unlock(
        &self,
        profile_id: i64,
        unlockable_id: &str,
        unlocked_at: &Timestamp,
    ) -> Result<bool> {
        check_unlockable_id(unlockable_id)?;
        let result = sqlx::query(
            "INSERT INTO unlockables (profile_id, unlockable_id, unlocked_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (profile_id, unlockable_id) DO NOTHING",
        )
        .bind(profile_id)
        .bind(unlockable_id)
        .bind(unlocked_at.to_string())
        .execute(self.db.writer())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Everything the profile has unlocked, in the order it was unlocked.
    pub async fn unlocked(&self, profile_id: i64) -> Result<Vec<Unlocked>> {
        let rows = sqlx::query(
            "SELECT unlockable_id, unlocked_at FROM unlockables WHERE profile_id = ?1 \
             ORDER BY unlocked_at, unlockable_id",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter()
            .map(|row| {
                Ok(Unlocked {
                    unlockable_id: row.try_get("unlockable_id")?,
                    unlocked_at: timestamp_col(row, "unlocked_at")?,
                })
            })
            .collect()
    }

    /// The cosmetics currently worn, or `None` when none was ever chosen.
    pub async fn equipped(&self, profile_id: i64) -> Result<Option<EquippedCosmetics>> {
        let row = sqlx::query("SELECT * FROM equipped_cosmetics WHERE profile_id = ?1")
            .bind(profile_id)
            .fetch_optional(self.db.reader())
            .await?;
        row.map(|row| {
            Ok(EquippedCosmetics {
                profile_id: row.try_get("profile_id")?,
                mascot_accessory: row.try_get("mascot_accessory")?,
                theme: row.try_get("theme")?,
                updated_at: timestamp_col(&row, "updated_at")?,
            })
        })
        .transpose()
    }

    /// Puts an unlocked accessory on the mascot, or takes it off with `None`.
    pub async fn equip_accessory(
        &self,
        profile_id: i64,
        accessory: Option<&str>,
        at: &Timestamp,
    ) -> Result<()> {
        self.require_wearable(profile_id, accessory, "accessory.")
            .await?;
        sqlx::query(
            "INSERT INTO equipped_cosmetics (profile_id, mascot_accessory, updated_at) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT (profile_id) DO UPDATE SET mascot_accessory = excluded.mascot_accessory, \
             updated_at = excluded.updated_at",
        )
        .bind(profile_id)
        .bind(accessory)
        .bind(at.to_string())
        .execute(self.db.writer())
        .await?;
        Ok(())
    }

    /// Switches to an unlocked theme, or back to the default with `None`.
    pub async fn equip_theme(
        &self,
        profile_id: i64,
        theme: Option<&str>,
        at: &Timestamp,
    ) -> Result<()> {
        self.require_wearable(profile_id, theme, "theme.").await?;
        sqlx::query(
            "INSERT INTO equipped_cosmetics (profile_id, theme, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (profile_id) DO UPDATE SET theme = excluded.theme, \
             updated_at = excluded.updated_at",
        )
        .bind(profile_id)
        .bind(theme)
        .bind(at.to_string())
        .execute(self.db.writer())
        .await?;
        Ok(())
    }

    /// Refuses an id of the wrong kind or one the profile has not unlocked, with
    /// a clear message. The composite foreign key is the backstop.
    async fn require_wearable(
        &self,
        profile_id: i64,
        id: Option<&str>,
        prefix: &'static str,
    ) -> Result<()> {
        let Some(id) = id else { return Ok(()) };
        check_unlockable_id(id)?;
        if !id.starts_with(prefix) {
            return Err(StorageError::Rule("that cosmetic does not fit this slot"));
        }
        let owned: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM unlockables WHERE profile_id = ?1 AND unlockable_id = ?2",
        )
        .bind(profile_id)
        .bind(id)
        .fetch_optional(self.db.reader())
        .await?;
        if owned.is_none() {
            return Err(StorageError::Rule("only an unlocked cosmetic can be worn"));
        }
        Ok(())
    }
}

async fn load_days(conn: &mut SqliteConnection, profile_id: i64) -> Result<Vec<Day>> {
    let rows = sqlx::query("SELECT local_date, counted FROM streak_days WHERE profile_id = ?1")
        .bind(profile_id)
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|row| {
            let text: String = row.try_get("local_date")?;
            Ok(Day {
                date: LocalDate::parse(&text)?,
                counted: row.try_get("counted")?,
            })
        })
        .collect()
}

/// Ids of tokens that no streak day has used, oldest first.
async fn unspent_token_ids(conn: &mut SqliteConnection, profile_id: i64) -> Result<Vec<i64>> {
    let ids = sqlx::query_scalar(
        "SELECT t.id FROM rest_tokens AS t \
         LEFT JOIN streak_days AS s ON s.rest_token_id = t.id \
         WHERE t.profile_id = ?1 AND s.rest_token_id IS NULL ORDER BY t.earned_at, t.id",
    )
    .bind(profile_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids)
}

async fn insert_day(
    conn: &mut SqliteConnection,
    profile_id: i64,
    date: LocalDate,
    rest_token_id: Option<i64>,
    recorded_at: &Timestamp,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO streak_days (profile_id, local_date, counted, rest_token_id, recorded_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(profile_id)
    .bind(date.to_string())
    .bind(rest_token_id.is_none())
    .bind(rest_token_id)
    .bind(recorded_at.to_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn status_on(
    conn: &mut SqliteConnection,
    profile_id: i64,
    today: LocalDate,
) -> Result<StreakStatus> {
    let days = load_days(conn, profile_id).await?;
    let held = unspent_token_ids(conn, profile_id).await?.len();
    Ok(evaluate(
        &days,
        today,
        u32::try_from(held).unwrap_or(u32::MAX),
    ))
}
