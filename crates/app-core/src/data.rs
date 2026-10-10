//! Data operations: delete one session, delete everything, export.
//!
//! Order matters for deletes. The database cannot delete files, so recordings go
//! first and the rows second: a crash in between leaves a row that points at a
//! missing file, which is harmless, instead of a recording nobody knows about.
//! Afterwards the database file is rewritten, so deleted text does not stay in
//! unused pages.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};
use storage::Timestamp;

use crate::api::{DeleteDataResult, DeleteSessionResult, ExportBundle};
use crate::core::{AppCore, ensure_profile};
use crate::error::{CoreError, CoreResult};
use crate::settings;

/// Layout version of the export.
const EXPORT_FORMAT_VERSION: u32 = 1;

/// The four skills a level estimate exists for. The export reads the attempts
/// of each, because storage lists attempts by skill; `estimates` recomputes the
/// estimate of each. The order matches `assessment_engine::Skill::ALL`.
pub(crate) const ESTIMATE_SKILLS: [&str; 4] = ["listening", "speaking", "reading", "writing"];

/// Where a stored recording path points, or `None` when it would leave the data
/// directory. Stored paths are relative to the data directory; an absolute path
/// is accepted only inside it. Anything with `..` is refused, so a damaged row
/// can never make the program delete a file elsewhere.
fn recording_file(data_dir: &Path, stored: &str) -> Option<PathBuf> {
    let path = Path::new(stored);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_dir.join(path)
    };
    if full.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    full.starts_with(data_dir).then_some(full)
}

fn to_json<T: Serialize>(value: &T) -> CoreResult<Value> {
    serde_json::to_value(value)
        .map_err(|_| CoreError::Internal("a row could not be written as JSON".to_owned()))
}

impl AppCore {
    /// Deletes the files in `stored_paths` that are inside the data directory.
    /// A file that is already gone is fine; any other failure stops the delete
    /// before the rows go.
    async fn remove_recordings(&self, stored_paths: &[String]) -> CoreResult<u32> {
        let mut removed = 0_u32;
        for stored in stored_paths {
            let Some(file) = recording_file(&self.config.data_dir, stored) else {
                tracing::warn!(
                    "a recording path points outside the data directory; it was left alone"
                );
                continue;
            };
            match tokio::fs::remove_file(&file).await {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(CoreError::io("deleting a recording", &error)),
            }
        }
        Ok(removed)
    }

    /// Rewrites the database file. Failure is reported, not fatal: the rows are
    /// already gone.
    async fn compact_after_delete(&self) -> bool {
        match self.db.compact().await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(error = %error, "the database could not be compacted after a delete");
                false
            }
        }
    }

    fn refuse_while_a_session_runs(&self, only: Option<i64>) -> CoreResult<()> {
        let Some(sessions) = self.sessions.get() else {
            return Ok(());
        };
        match (sessions.active_session(), only) {
            (Some(active), Some(wanted)) if active == wanted => Err(CoreError::Conflict(
                "that session is running; stop it first".to_owned(),
            )),
            (Some(_), None) => Err(CoreError::Conflict(
                "a session is running; stop it first".to_owned(),
            )),
            _ => Ok(()),
        }
    }

    /// Deletes one session: its turns, drafts, analysis, generated content,
    /// recordings and free-speech pronunciation rows. Scores and estimates stay;
    /// the text that backed them is removed. XP stays too: it never pointed at a
    /// session.
    pub async fn delete_session(&self, id: i64) -> CoreResult<DeleteSessionResult> {
        self.ensure_running()?;
        self.refuse_while_a_session_runs(Some(id))?;
        let session = self
            .db
            .sessions()
            .get(id)
            .await?
            .filter(|s| s.profile_id == self.profile_id())
            .ok_or(CoreError::NotFound { what: "session" })?;
        let paths = self.db.sessions().audio_paths(session.id).await?;
        let audio_files_removed = self.remove_recordings(&paths).await?;
        self.db.sessions().delete(session.id).await?;
        let compacted = self.compact_after_delete().await;
        Ok(DeleteSessionResult {
            audio_files_removed,
            compacted,
        })
    }

    /// Deletes everything about the learner and starts a fresh profile that
    /// keeps only the interface language. Provider profiles, the other settings
    /// and the unit index are not learning data and stay.
    pub async fn delete_all_data(&self) -> CoreResult<DeleteDataResult> {
        self.ensure_running()?;
        self.refuse_while_a_session_runs(None)?;
        let _settings_lock = self.settings_write.lock().await;
        let old_id = self.profile_id();
        let old = self
            .db
            .profiles()
            .get(old_id)
            .await?
            .ok_or(CoreError::NotFound { what: "profile" })?;
        let paths = self.db.audio_clips().paths_for_profile(old_id).await?;
        let audio_files_removed = self.remove_recordings(&paths).await?;
        self.db.profiles().delete(old_id).await?;

        let fresh = ensure_profile(&self.db, self.clock(), Some(old.ui_language)).await?;
        self.profile_id
            .store(fresh.id, std::sync::atomic::Ordering::SeqCst);
        let loaded = settings::load(&self.db, &fresh).await?;
        *self
            .settings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = loaded;
        self.sync_unlocks().await?;
        let compacted = self.compact_after_delete().await;
        self.publish_snapshot();
        Ok(DeleteDataResult {
            audio_files_removed,
            compacted,
        })
    }

    /// Everything stored about the learner, as JSON, without any key.
    pub async fn export(&self) -> CoreResult<ExportBundle> {
        let profile_id = self.profile_id();
        let db = &self.db;
        let epoch = Timestamp::parse("1970-01-01T00:00:00.000Z")
            .map_err(|_| CoreError::Internal("the epoch is not a timestamp".to_owned()))?;
        let far_future = Timestamp::parse("9999-12-31T23:59:59.999Z")
            .map_err(|_| CoreError::Internal("the end of time is not a timestamp".to_owned()))?;

        let profile = db
            .profiles()
            .get(profile_id)
            .await?
            .ok_or(CoreError::NotFound { what: "profile" })?;

        let mut sessions = Vec::new();
        for session in db.sessions().list_for_profile(profile_id, u32::MAX).await? {
            let turns = db.turns().list(session.id).await?;
            let mut turn_rows = Vec::with_capacity(turns.len());
            for turn in &turns {
                turn_rows.push(json!({
                    "turn": to_json(turn)?,
                    "analysis": to_json(&db.analysis().get(turn.id).await?)?,
                    "error_events": to_json(&db.analysis().error_events(turn.id).await?)?,
                    "pron_results": to_json(&db.pron_results().for_turn(turn.id).await?)?,
                }));
            }
            sessions.push(json!({
                "session": to_json(&session)?,
                "turns": turn_rows,
                "generated_content": to_json(&db.generated_content().for_session(session.id).await?)?,
            }));
        }

        let mut attempts = Vec::new();
        for skill in ESTIMATE_SKILLS {
            for attempt in db
                .attempts()
                .for_skill_since(profile_id, skill, &epoch)
                .await?
            {
                attempts.push(json!({
                    "attempt": to_json(&attempt)?,
                    "evidence": to_json(&db.evidence().for_attempt(attempt.id).await?)?,
                    "pron_results": to_json(&db.pron_results().for_attempt(attempt.id).await?)?,
                }));
            }
        }

        let latest = db.estimates().latest_per_skill(profile_id).await?;
        let mut estimate_history = Vec::new();
        for estimate in &latest {
            estimate_history.push(json!({
                "skill": estimate.skill,
                "history": to_json(&db.estimates().history(profile_id, &estimate.skill).await?)?,
            }));
        }

        let game = db.game();
        let data = json!({
            "profile": to_json(&profile)?,
            "settings": to_json(&self.settings())?,
            "providers": self.providers.exportable(),
            "sessions": sessions,
            "attempts": attempts,
            "progress": {
                "units": to_json(&db.unit_progress().list(profile_id).await?)?,
                "objective_mastery": to_json(&db.mastery().list(profile_id).await?)?,
                "error_stats": to_json(&db.error_stats().list(profile_id).await?)?,
                "review_schedule": to_json(
                    &db.review_schedule().due(profile_id, &far_future, u32::MAX).await?
                )?,
            },
            "estimates": estimate_history,
            "game": {
                "xp": to_json(&game.xp_entries(profile_id, u32::MAX).await?)?,
                "streak_days": to_json(&game.streak_days(profile_id).await?)?,
                "rest_tokens_unused": to_json(&game.available_rest_tokens(profile_id).await?)?,
                "unlocked": to_json(&game.unlocked(profile_id).await?)?,
                "equipped": to_json(&game.equipped(profile_id).await?)?,
            },
        });
        Ok(ExportBundle {
            format_version: EXPORT_FORMAT_VERSION,
            exported_at: self.clock().now().to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_are_only_ever_inside_the_data_directory() {
        let data = Path::new("/data/lumingo");
        // (stored path, expected file)
        let cases = [
            ("audio/1.wav", Some("/data/lumingo/audio/1.wav")),
            (
                "/data/lumingo/audio/2.wav",
                Some("/data/lumingo/audio/2.wav"),
            ),
            ("../outside.wav", None),
            ("audio/../../outside.wav", None),
            ("/etc/passwd", None),
            ("/data/lumingo/../etc/passwd", None),
        ];
        for (stored, expected) in cases {
            assert_eq!(
                recording_file(data, stored),
                expected.map(PathBuf::from),
                "{stored}"
            );
        }
    }
}
