//! Review schedule and objective mastery persistence. The scheduler itself is
//! `assessment-engine::review` (pure, whole-day numbers); this module only
//! stores its rows and the `YYYY-MM-DD` dates they carry.

use crate::db::Database;
use crate::error::{StorageError, classify, not_found};
use crate::models::{NewReviewItem, ObjectiveMastery, ReviewItem, ReviewKind, ReviewUpdate};

impl Database {
    /// Enrols an item into the schedule, or moves an already scheduled item's
    /// due date (its ease, reps and lapses survive). Returns the stored row.
    pub async fn enroll_review_item(
        &self,
        item: &NewReviewItem,
    ) -> Result<ReviewItem, StorageError> {
        sqlx::query(
            "INSERT INTO review_schedule (profile_id, item_kind, item_ref, due_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT (profile_id, item_kind, item_ref) DO UPDATE SET due_at = excluded.due_at",
        )
        .bind(item.profile_id)
        .bind(item.kind.as_str())
        .bind(&item.item_ref)
        .bind(&item.due_at)
        .execute(self.writer())
        .await
        .map_err(|error| classify("review_schedule", error))?;

        let row: ReviewItemRow = sqlx::query_as(
            "SELECT id, profile_id, item_kind, item_ref, due_at, interval_days, ease, reps, \
                    lapses, last_reviewed_at \
             FROM review_schedule WHERE profile_id = ? AND item_kind = ? AND item_ref = ?",
        )
        .bind(item.profile_id)
        .bind(item.kind.as_str())
        .bind(&item.item_ref)
        .fetch_one(self.readers())
        .await?;
        ReviewItem::try_from(row)
    }

    /// Writes the scheduler's numbers back after one review.
    pub async fn record_review(&self, id: i64, update: &ReviewUpdate) -> Result<(), StorageError> {
        let result = sqlx::query(
            "UPDATE review_schedule SET due_at = ?, interval_days = ?, ease = ?, reps = ?, \
                    lapses = ?, last_reviewed_at = ? WHERE id = ?",
        )
        .bind(&update.due_at)
        .bind(update.interval_days)
        .bind(update.ease)
        .bind(update.reps)
        .bind(update.lapses)
        .bind(&update.last_reviewed_at)
        .bind(id)
        .execute(self.writer())
        .await
        .map_err(|error| classify("review_schedule", error))?;

        if result.rows_affected() == 0 {
            return Err(not_found("review_schedule", id));
        }
        Ok(())
    }

    /// The scheduled items of one profile, soonest due first.
    pub async fn review_items(&self, profile_id: i64) -> Result<Vec<ReviewItem>, StorageError> {
        let rows: Vec<ReviewItemRow> = sqlx::query_as(
            "SELECT id, profile_id, item_kind, item_ref, due_at, interval_days, ease, reps, \
                    lapses, last_reviewed_at \
             FROM review_schedule WHERE profile_id = ? ORDER BY due_at, id",
        )
        .bind(profile_id)
        .fetch_all(self.readers())
        .await?;
        rows.into_iter().map(ReviewItem::try_from).collect()
    }

    /// The items due on or before `date` (`YYYY-MM-DD`), most overdue first.
    pub async fn due_review_items(
        &self,
        profile_id: i64,
        date: &str,
    ) -> Result<Vec<ReviewItem>, StorageError> {
        let rows: Vec<ReviewItemRow> = sqlx::query_as(
            "SELECT id, profile_id, item_kind, item_ref, due_at, interval_days, ease, reps, \
                    lapses, last_reviewed_at \
             FROM review_schedule WHERE profile_id = ? AND due_at <= ? ORDER BY due_at, id",
        )
        .bind(profile_id)
        .bind(date)
        .fetch_all(self.readers())
        .await?;
        rows.into_iter().map(ReviewItem::try_from).collect()
    }

    /// Writes one mastery result: the moving average of `assessment-engine`'s
    /// `update_mastery`, with the attempt count and the date it was last
    /// touched. The caller computes the average; this only stores it.
    pub async fn upsert_objective_mastery(
        &self,
        profile_id: i64,
        objective_id: &str,
        mastery: f64,
        at: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO objective_mastery (profile_id, objective_id, mastery, attempts, last_attempt_at) \
             VALUES (?, ?, ?, 1, ?) \
             ON CONFLICT (profile_id, objective_id) DO UPDATE SET \
               mastery = excluded.mastery, \
               attempts = objective_mastery.attempts + 1, \
               last_attempt_at = excluded.last_attempt_at",
        )
        .bind(profile_id)
        .bind(objective_id)
        .bind(mastery)
        .bind(at)
        .execute(self.writer())
        .await
        .map_err(|error| classify("objective_mastery", error))?;
        Ok(())
    }

    /// One objective's mastery row, when it exists.
    pub async fn objective_mastery(
        &self,
        profile_id: i64,
        objective_id: &str,
    ) -> Result<Option<ObjectiveMastery>, StorageError> {
        let row: Option<ObjectiveMasteryRow> = sqlx::query_as(
            "SELECT profile_id, objective_id, mastery, attempts, last_attempt_at \
             FROM objective_mastery WHERE profile_id = ? AND objective_id = ?",
        )
        .bind(profile_id)
        .bind(objective_id)
        .fetch_optional(self.readers())
        .await?;
        Ok(row.map(ObjectiveMastery::from))
    }

    /// Every mastery row of one profile, strongest first.
    pub async fn objective_masteries(
        &self,
        profile_id: i64,
    ) -> Result<Vec<ObjectiveMastery>, StorageError> {
        let rows: Vec<ObjectiveMasteryRow> = sqlx::query_as(
            "SELECT profile_id, objective_id, mastery, attempts, last_attempt_at \
             FROM objective_mastery WHERE profile_id = ? ORDER BY mastery DESC, objective_id",
        )
        .bind(profile_id)
        .fetch_all(self.readers())
        .await?;
        Ok(rows.into_iter().map(ObjectiveMastery::from).collect())
    }
}

#[derive(sqlx::FromRow)]
struct ReviewItemRow {
    id: i64,
    profile_id: i64,
    item_kind: String,
    item_ref: String,
    due_at: String,
    interval_days: f64,
    ease: f64,
    reps: i64,
    lapses: i64,
    last_reviewed_at: Option<String>,
}

impl TryFrom<ReviewItemRow> for ReviewItem {
    type Error = StorageError;

    fn try_from(row: ReviewItemRow) -> Result<Self, Self::Error> {
        use std::str::FromStr;
        Ok(ReviewItem {
            id: row.id,
            profile_id: row.profile_id,
            kind: ReviewKind::from_str(&row.item_kind).map_err(|error| StorageError::Invalid {
                table: "review_schedule",
                detail: error.to_string(),
            })?,
            item_ref: row.item_ref,
            due_at: row.due_at,
            interval_days: row.interval_days,
            ease: row.ease,
            reps: row.reps,
            lapses: row.lapses,
            last_reviewed_at: row.last_reviewed_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ObjectiveMasteryRow {
    profile_id: i64,
    objective_id: String,
    mastery: f64,
    attempts: i64,
    last_attempt_at: Option<String>,
}

impl From<ObjectiveMasteryRow> for ObjectiveMastery {
    fn from(row: ObjectiveMasteryRow) -> Self {
        ObjectiveMastery {
            profile_id: row.profile_id,
            objective_id: row.objective_id,
            mastery: row.mastery,
            attempts: row.attempts,
            last_attempt_at: row.last_attempt_at,
        }
    }
}
