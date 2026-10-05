//! Learning state that is numbers only: unit progress, objective mastery, error
//! counts and the review schedule.
//!
//! These tables are written by the crates that own the rules (progress, mastery
//! and spaced repetition). This module stores what it is given and reads it back;
//! it computes nothing. None of them holds learner text, which is why they
//! survive when a session is deleted.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{ReviewKind, UnitStatus};
use crate::error::{Result, StorageError};
use crate::row::{enum_col, opt_timestamp_col, timestamp_col};
use crate::time::Timestamp;

/// Where one unit stands for one profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitProgress {
    pub profile_id: i64,
    pub unit_id: String,
    pub status: UnitStatus,
    /// Best checkpoint score, 0 to 1.
    pub best_checkpoint: Option<f64>,
    pub updated_at: Timestamp,
}

/// Mastery of one objective, as computed by the mastery rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveMastery {
    pub profile_id: i64,
    /// `<unit id>/<objective id>`.
    pub objective_id: String,
    /// 0 to 1.
    pub mastery: f64,
    pub attempts: i64,
    pub last_attempt_at: Option<Timestamp>,
}

/// How often an error category was seen. Numbers only, so the pattern survives
/// when the sessions that showed it are deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorStat {
    pub profile_id: i64,
    pub category: String,
    pub count: i64,
    pub last_seen: Option<Timestamp>,
}

/// A spaced-review item and its schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: i64,
    pub profile_id: i64,
    pub item_kind: ReviewKind,
    /// `<unit id>/<item id>`.
    pub item_ref: String,
    pub due_at: Timestamp,
    pub interval_days: f64,
    pub ease: f64,
    pub reps: i64,
    pub lapses: i64,
    pub last_reviewed_at: Option<Timestamp>,
}

/// A review item to store or update. One row exists per profile, kind and ref.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewReviewItem {
    pub profile_id: i64,
    pub item_kind: ReviewKind,
    pub item_ref: String,
    pub due_at: Timestamp,
    pub interval_days: f64,
    pub ease: f64,
    pub reps: i64,
    pub lapses: i64,
    pub last_reviewed_at: Option<Timestamp>,
}

impl UnitProgress {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            profile_id: row.try_get("profile_id")?,
            unit_id: row.try_get("unit_id")?,
            status: enum_col(row, "status")?,
            best_checkpoint: row.try_get("best_checkpoint")?,
            updated_at: timestamp_col(row, "updated_at")?,
        })
    }
}

impl ObjectiveMastery {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            profile_id: row.try_get("profile_id")?,
            objective_id: row.try_get("objective_id")?,
            mastery: row.try_get("mastery")?,
            attempts: row.try_get("attempts")?,
            last_attempt_at: opt_timestamp_col(row, "last_attempt_at")?,
        })
    }
}

impl ErrorStat {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            profile_id: row.try_get("profile_id")?,
            category: row.try_get("category")?,
            count: row.try_get("count")?,
            last_seen: opt_timestamp_col(row, "last_seen")?,
        })
    }
}

impl ReviewItem {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            profile_id: row.try_get("profile_id")?,
            item_kind: enum_col(row, "item_kind")?,
            item_ref: row.try_get("item_ref")?,
            due_at: timestamp_col(row, "due_at")?,
            interval_days: row.try_get("interval_days")?,
            ease: row.try_get("ease")?,
            reps: row.try_get("reps")?,
            lapses: row.try_get("lapses")?,
            last_reviewed_at: opt_timestamp_col(row, "last_reviewed_at")?,
        })
    }
}

/// Queries on `unit_progress`.
pub struct UnitProgressRepo<'a> {
    db: &'a Database,
}

/// Queries on `objective_mastery`.
pub struct MasteryRepo<'a> {
    db: &'a Database,
}

/// Queries on `error_stats`.
pub struct ErrorStats<'a> {
    db: &'a Database,
}

/// Queries on `review_schedule`.
pub struct ReviewSchedule<'a> {
    db: &'a Database,
}

impl Database {
    /// Unit progress queries.
    pub fn unit_progress(&self) -> UnitProgressRepo<'_> {
        UnitProgressRepo { db: self }
    }

    /// Objective mastery queries.
    pub fn mastery(&self) -> MasteryRepo<'_> {
        MasteryRepo { db: self }
    }

    /// Error count queries.
    pub fn error_stats(&self) -> ErrorStats<'_> {
        ErrorStats { db: self }
    }

    /// Review schedule queries.
    pub fn review_schedule(&self) -> ReviewSchedule<'_> {
        ReviewSchedule { db: self }
    }
}

impl UnitProgressRepo<'_> {
    /// Stores the state of a unit, replacing the earlier row.
    pub async fn set(&self, progress: &UnitProgress) -> Result<()> {
        sqlx::query(
            "INSERT INTO unit_progress (profile_id, unit_id, status, best_checkpoint, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (profile_id, unit_id) DO UPDATE SET status = excluded.status, \
             best_checkpoint = excluded.best_checkpoint, updated_at = excluded.updated_at",
        )
        .bind(progress.profile_id)
        .bind(&progress.unit_id)
        .bind(progress.status.as_str())
        .bind(progress.best_checkpoint)
        .bind(progress.updated_at.to_string())
        .execute(self.db.writer())
        .await?;
        Ok(())
    }

    /// One unit's state.
    pub async fn get(&self, profile_id: i64, unit_id: &str) -> Result<Option<UnitProgress>> {
        let row = sqlx::query("SELECT * FROM unit_progress WHERE profile_id = ?1 AND unit_id = ?2")
            .bind(profile_id)
            .bind(unit_id)
            .fetch_optional(self.db.reader())
            .await?;
        row.as_ref().map(UnitProgress::from_row).transpose()
    }

    /// Every unit state of a profile, ordered by unit id.
    pub async fn list(&self, profile_id: i64) -> Result<Vec<UnitProgress>> {
        let rows =
            sqlx::query("SELECT * FROM unit_progress WHERE profile_id = ?1 ORDER BY unit_id")
                .bind(profile_id)
                .fetch_all(self.db.reader())
                .await?;
        rows.iter().map(UnitProgress::from_row).collect()
    }
}

impl MasteryRepo<'_> {
    /// Stores the mastery of an objective, replacing the earlier row.
    pub async fn set(&self, mastery: &ObjectiveMastery) -> Result<()> {
        sqlx::query(
            "INSERT INTO objective_mastery (profile_id, objective_id, mastery, attempts, last_attempt_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (profile_id, objective_id) DO UPDATE SET mastery = excluded.mastery, \
             attempts = excluded.attempts, last_attempt_at = excluded.last_attempt_at",
        )
        .bind(mastery.profile_id)
        .bind(&mastery.objective_id)
        .bind(mastery.mastery)
        .bind(mastery.attempts)
        .bind(mastery.last_attempt_at.map(|t| t.to_string()))
        .execute(self.db.writer())
        .await?;
        Ok(())
    }

    /// One objective's mastery.
    pub async fn get(
        &self,
        profile_id: i64,
        objective_id: &str,
    ) -> Result<Option<ObjectiveMastery>> {
        let row = sqlx::query(
            "SELECT * FROM objective_mastery WHERE profile_id = ?1 AND objective_id = ?2",
        )
        .bind(profile_id)
        .bind(objective_id)
        .fetch_optional(self.db.reader())
        .await?;
        row.as_ref().map(ObjectiveMastery::from_row).transpose()
    }

    /// Every objective's mastery for a profile, ordered by objective id.
    pub async fn list(&self, profile_id: i64) -> Result<Vec<ObjectiveMastery>> {
        let rows = sqlx::query(
            "SELECT * FROM objective_mastery WHERE profile_id = ?1 ORDER BY objective_id",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(ObjectiveMastery::from_row).collect()
    }
}

impl ErrorStats<'_> {
    /// Adds `by` sightings of an error category.
    ///
    /// `category` is a code from the error catalog, never the learner's words:
    /// this table outlives the sessions that produced the errors.
    pub async fn add(
        &self,
        profile_id: i64,
        category: &str,
        by: u32,
        seen_at: &Timestamp,
    ) -> Result<()> {
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/');
        if category.is_empty() || category.len() > 64 || !category.chars().all(plain) {
            return Err(StorageError::Rule(
                "an error category is a short catalog code, not text",
            ));
        }
        sqlx::query(
            "INSERT INTO error_stats (profile_id, category, count, last_seen) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (profile_id, category) DO UPDATE SET count = count + excluded.count, \
             last_seen = excluded.last_seen",
        )
        .bind(profile_id)
        .bind(category)
        .bind(i64::from(by))
        .bind(seen_at.to_string())
        .execute(self.db.writer())
        .await?;
        Ok(())
    }

    /// Error categories of a profile, most frequent first.
    pub async fn list(&self, profile_id: i64) -> Result<Vec<ErrorStat>> {
        let rows = sqlx::query(
            "SELECT * FROM error_stats WHERE profile_id = ?1 ORDER BY count DESC, category",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(ErrorStat::from_row).collect()
    }
}

impl ReviewSchedule<'_> {
    /// Stores an item's schedule, replacing the row of the same profile, kind and
    /// ref, and returns it.
    pub async fn put(&self, item: &NewReviewItem) -> Result<ReviewItem> {
        let row = sqlx::query(
            "INSERT INTO review_schedule \
             (profile_id, item_kind, item_ref, due_at, interval_days, ease, reps, lapses, last_reviewed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT (profile_id, item_kind, item_ref) DO UPDATE SET due_at = excluded.due_at, \
             interval_days = excluded.interval_days, ease = excluded.ease, reps = excluded.reps, \
             lapses = excluded.lapses, last_reviewed_at = excluded.last_reviewed_at \
             RETURNING *",
        )
        .bind(item.profile_id)
        .bind(item.item_kind.as_str())
        .bind(&item.item_ref)
        .bind(item.due_at.to_string())
        .bind(item.interval_days)
        .bind(item.ease)
        .bind(item.reps)
        .bind(item.lapses)
        .bind(item.last_reviewed_at.map(|t| t.to_string()))
        .fetch_one(self.db.writer())
        .await?;
        ReviewItem::from_row(&row)
    }

    /// One item's schedule.
    pub async fn get(
        &self,
        profile_id: i64,
        item_kind: ReviewKind,
        item_ref: &str,
    ) -> Result<Option<ReviewItem>> {
        let row = sqlx::query(
            "SELECT * FROM review_schedule WHERE profile_id = ?1 AND item_kind = ?2 AND item_ref = ?3",
        )
        .bind(profile_id)
        .bind(item_kind.as_str())
        .bind(item_ref)
        .fetch_optional(self.db.reader())
        .await?;
        row.as_ref().map(ReviewItem::from_row).transpose()
    }

    /// Items due at or before `now`, most overdue first.
    pub async fn due(
        &self,
        profile_id: i64,
        now: &Timestamp,
        limit: u32,
    ) -> Result<Vec<ReviewItem>> {
        let rows = sqlx::query(
            "SELECT * FROM review_schedule WHERE profile_id = ?1 AND due_at <= ?2 \
             ORDER BY due_at, id LIMIT ?3",
        )
        .bind(profile_id)
        .bind(now.to_string())
        .bind(i64::from(limit))
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(ReviewItem::from_row).collect()
    }
}
