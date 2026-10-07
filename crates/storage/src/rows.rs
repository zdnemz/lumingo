//! Raw row shapes and the conversion to the public types.
//!
//! The row structs keep enum columns as TEXT so a stored value this build does
//! not know becomes a typed [`StorageError::Invalid`] instead of a string that
//! leaks through the API.

use std::str::FromStr;

use crate::error::StorageError;
use crate::models::*;

/// Parses a stored TEXT value into its enum, naming the table in the error.
fn parse_enum<T>(table: &'static str, value: String) -> Result<T, StorageError>
where
    T: FromStr<Err = UnknownValue>,
{
    match value.parse::<T>() {
        Ok(parsed) => Ok(parsed),
        Err(error) => Err(StorageError::Invalid {
            table,
            detail: error.to_string(),
        }),
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct ProfileRow {
    pub id: i64,
    pub display_name: String,
    pub ui_language: String,
    pub l1: String,
    pub l1_help_mode: String,
    pub created_at: String,
}

impl TryFrom<ProfileRow> for Profile {
    type Error = StorageError;

    fn try_from(row: ProfileRow) -> Result<Self, Self::Error> {
        Ok(Profile {
            id: row.id,
            display_name: row.display_name,
            ui_language: parse_enum("profiles", row.ui_language)?,
            l1: row.l1,
            l1_help_mode: parse_enum("profiles", row.l1_help_mode)?,
            created_at: row.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct SessionRow {
    pub id: i64,
    pub profile_id: i64,
    pub kind: String,
    pub unit_id: Option<String>,
    pub activity_id: Option<String>,
    pub mode: Option<String>,
    pub status: String,
    pub provider_profile_id: Option<i64>,
    pub summary_json: Option<String>,
    pub app_version: String,
    pub started_at: String,
    pub ended_at: Option<String>,
}

impl TryFrom<SessionRow> for Session {
    type Error = StorageError;

    fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
        Ok(Session {
            id: row.id,
            profile_id: row.profile_id,
            kind: parse_enum("sessions", row.kind)?,
            unit_id: row.unit_id,
            activity_id: row.activity_id,
            mode: row
                .mode
                .map(|mode| parse_enum("sessions", mode))
                .transpose()?,
            status: parse_enum("sessions", row.status)?,
            provider_profile_id: row.provider_profile_id,
            summary_json: row.summary_json,
            app_version: row.app_version,
            started_at: row.started_at,
            ended_at: row.ended_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct TurnRow {
    pub id: i64,
    pub session_id: i64,
    pub seq: i64,
    pub role: String,
    pub input_mode: String,
    pub text: String,
    pub stt_text: Option<String>,
    pub edited_by_learner: bool,
    pub speech_ms: Option<i64>,
    pub pause_ms: Option<i64>,
    pub word_count: Option<i64>,
    pub created_at: String,
}

impl TryFrom<TurnRow> for Turn {
    type Error = StorageError;

    fn try_from(row: TurnRow) -> Result<Self, Self::Error> {
        Ok(Turn {
            id: row.id,
            session_id: row.session_id,
            seq: row.seq,
            role: parse_enum("turns", row.role)?,
            input_mode: parse_enum("turns", row.input_mode)?,
            text: row.text,
            stt_text: row.stt_text,
            edited_by_learner: row.edited_by_learner,
            speech_ms: row.speech_ms,
            pause_ms: row.pause_ms,
            word_count: row.word_count,
            created_at: row.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct AttemptRow {
    pub id: i64,
    pub profile_id: i64,
    pub session_id: Option<i64>,
    pub unit_id: Option<String>,
    pub activity_id: String,
    pub activity_type: String,
    pub response_id: String,
    pub origin: String,
    pub level: String,
    pub skill: String,
    pub dimension: String,
    pub scorer: String,
    pub scorer_version: String,
    pub raw_score: Option<f64>,
    pub max_score: Option<f64>,
    pub normalized: Option<f64>,
    pub confidence: Option<f64>,
    pub status: String,
    pub counts_toward_estimate: bool,
    pub created_at: String,
}

impl TryFrom<AttemptRow> for Attempt {
    type Error = StorageError;

    fn try_from(row: AttemptRow) -> Result<Self, Self::Error> {
        Ok(Attempt {
            id: row.id,
            profile_id: row.profile_id,
            session_id: row.session_id,
            unit_id: row.unit_id,
            activity_id: row.activity_id,
            activity_type: row.activity_type,
            response_id: row.response_id,
            origin: parse_enum("assessment_attempts", row.origin)?,
            level: parse_enum("assessment_attempts", row.level)?,
            skill: parse_enum("assessment_attempts", row.skill)?,
            dimension: row.dimension,
            scorer: parse_enum("assessment_attempts", row.scorer)?,
            scorer_version: row.scorer_version,
            raw_score: row.raw_score,
            max_score: row.max_score,
            normalized: row.normalized,
            confidence: row.confidence,
            status: parse_enum("assessment_attempts", row.status)?,
            counts_toward_estimate: row.counts_toward_estimate,
            created_at: row.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct EvidenceRow {
    pub id: i64,
    pub attempt_id: i64,
    pub kind: String,
    pub content: Option<String>,
    pub data_json: Option<String>,
    pub created_at: String,
}

impl TryFrom<EvidenceRow> for Evidence {
    type Error = StorageError;

    fn try_from(row: EvidenceRow) -> Result<Self, Self::Error> {
        Ok(Evidence {
            id: row.id,
            attempt_id: row.attempt_id,
            kind: parse_enum("assessment_evidence", row.kind)?,
            content: row.content,
            data_json: row.data_json,
            created_at: row.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct PendingScoringRow {
    pub id: i64,
    pub attempt_id: i64,
    pub payload_json: String,
    pub tries: i64,
    pub created_at: String,
}

impl TryFrom<PendingScoringRow> for PendingScoring {
    type Error = StorageError;

    fn try_from(row: PendingScoringRow) -> Result<Self, Self::Error> {
        Ok(PendingScoring {
            id: row.id,
            attempt_id: row.attempt_id,
            payload_json: row.payload_json,
            tries: row.tries,
            created_at: row.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Level;

    #[test]
    fn an_unknown_stored_value_becomes_a_typed_error_naming_the_table() {
        // A file written by a newer build could hold a level this one does not
        // know even though the CHECK constraint exists today.
        let error = parse_enum::<Level>("assessment_attempts", "C3".to_owned()).unwrap_err();
        match error {
            StorageError::Invalid { table, detail } => {
                assert_eq!(table, "assessment_attempts");
                assert!(detail.contains("C3"), "detail was {detail}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn a_known_stored_value_parses() {
        assert_eq!(
            parse_enum::<Level>("assessment_attempts", "B1".to_owned()).unwrap(),
            Level::B1
        );
    }
}
