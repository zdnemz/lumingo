//! Stored skill estimates.
//!
//! The estimation code (the assessment crate) is the only writer of `level`.
//! This repository stores and returns what it is given; it computes nothing, and
//! no model output ever reaches it. History is kept: the current estimate of a
//! skill is its latest row by `computed_at`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::db::Database;
use crate::enums::{EstimateLevel, EstimateStatus};
use crate::error::Result;
use crate::row::{enum_col, opt_enum_col, opt_json_col, opt_json_text, timestamp_col};
use crate::time::Timestamp;

/// One computed estimate for one skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillEstimate {
    pub id: i64,
    pub profile_id: i64,
    pub skill: String,
    pub level: Option<EstimateLevel>,
    pub status: EstimateStatus,
    pub confidence: Option<f64>,
    pub evidence_count: i64,
    /// Version of the estimation rule, for example `est/1`.
    pub algorithm_version: String,
    pub detail: Option<Value>,
    pub computed_at: Timestamp,
}

/// An estimate to store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSkillEstimate {
    pub profile_id: i64,
    pub skill: String,
    pub level: Option<EstimateLevel>,
    pub status: EstimateStatus,
    pub confidence: Option<f64>,
    pub evidence_count: i64,
    pub algorithm_version: String,
    pub detail: Option<Value>,
    pub computed_at: Timestamp,
}

impl SkillEstimate {
    fn from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            profile_id: row.try_get("profile_id")?,
            skill: row.try_get("skill")?,
            level: opt_enum_col(row, "level")?,
            status: enum_col(row, "status")?,
            confidence: row.try_get("confidence")?,
            evidence_count: row.try_get("evidence_count")?,
            algorithm_version: row.try_get("algorithm_version")?,
            detail: opt_json_col(row, "detail_json")?,
            computed_at: timestamp_col(row, "computed_at")?,
        })
    }
}

/// Queries on the `skill_estimates` table.
pub struct Estimates<'a> {
    db: &'a Database,
}

impl Database {
    /// Estimate queries.
    pub fn estimates(&self) -> Estimates<'_> {
        Estimates { db: self }
    }
}

impl Estimates<'_> {
    /// Appends an estimate. Earlier rows stay as history.
    pub async fn insert(&self, new: &NewSkillEstimate) -> Result<SkillEstimate> {
        let row = sqlx::query(
            "INSERT INTO skill_estimates \
             (profile_id, skill, level, status, confidence, evidence_count, algorithm_version, \
              detail_json, computed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING *",
        )
        .bind(new.profile_id)
        .bind(&new.skill)
        .bind(new.level.map(|l| l.as_str()))
        .bind(new.status.as_str())
        .bind(new.confidence)
        .bind(new.evidence_count)
        .bind(&new.algorithm_version)
        .bind(opt_json_text(new.detail.as_ref()))
        .bind(new.computed_at.to_string())
        .fetch_one(self.db.writer())
        .await?;
        SkillEstimate::from_row(&row)
    }

    /// The current estimate of a skill: its latest row.
    pub async fn latest(&self, profile_id: i64, skill: &str) -> Result<Option<SkillEstimate>> {
        let row = sqlx::query(
            "SELECT * FROM skill_estimates WHERE profile_id = ?1 AND skill = ?2 \
             ORDER BY computed_at DESC, id DESC LIMIT 1",
        )
        .bind(profile_id)
        .bind(skill)
        .fetch_optional(self.db.reader())
        .await?;
        row.as_ref().map(SkillEstimate::from_row).transpose()
    }

    /// The current estimate of every skill that has one, ordered by skill.
    pub async fn latest_per_skill(&self, profile_id: i64) -> Result<Vec<SkillEstimate>> {
        let rows = sqlx::query(
            "SELECT current.* FROM skill_estimates AS current \
             WHERE current.profile_id = ?1 AND current.id = ( \
                 SELECT newest.id FROM skill_estimates AS newest \
                 WHERE newest.profile_id = current.profile_id AND newest.skill = current.skill \
                 ORDER BY newest.computed_at DESC, newest.id DESC LIMIT 1) \
             ORDER BY current.skill",
        )
        .bind(profile_id)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(SkillEstimate::from_row).collect()
    }

    /// Every estimate of a skill, oldest first.
    pub async fn history(&self, profile_id: i64, skill: &str) -> Result<Vec<SkillEstimate>> {
        let rows = sqlx::query(
            "SELECT * FROM skill_estimates WHERE profile_id = ?1 AND skill = ?2 \
             ORDER BY computed_at, id",
        )
        .bind(profile_id)
        .bind(skill)
        .fetch_all(self.db.reader())
        .await?;
        rows.iter().map(SkillEstimate::from_row).collect()
    }
}
