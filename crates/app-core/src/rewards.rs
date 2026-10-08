//! The cosmetic game layer: XP, streaks, rest tokens, unlockables and the
//! cosmetics worn, through the `game` rules and `storage`.
//!
//! This layer is kept apart from assessment on purpose. It reads no attempt,
//! evidence or estimate and writes none, and nothing here is a level of English.
//! The rules of the `game` crate call XP "sparks"; the product and the API say
//! XP.
//!
//! Ids: the `game` catalogue writes `accessory-cap`, the database requires
//! `accessory.cap`. The API uses the catalogue form; the two are converted at
//! this boundary and nowhere else.

use game::{Cosmetic, SparkSource, UNLOCKS, rank_for};
use storage::{StreakStatus, XpAward};

use crate::api::{
    CosmeticKind, CosmeticSlot, CosmeticView, EquipRequest, EquippedView, GameState,
    PracticeOutcome, RankView, StreakDayView, StreakView, XpBySourceView, XpSourceKind,
};
use crate::core::AppCore;
use crate::error::{CoreError, CoreResult};

/// How many days of the streak calendar `GET /api/game` returns.
const CALENDAR_DAYS: i64 = 90;
const PRACTICE_REASON: &str = "practice.completed";

/// The catalogue id as the database stores it.
fn stored_id(id: &str) -> String {
    id.replacen('-', ".", 1)
}

/// The database id as the catalogue and the API write it.
fn catalogue_id(stored: &str) -> String {
    stored.replacen('.', "-", 1)
}

fn rank_view(xp: i64) -> RankView {
    let info = rank_for(u32::try_from(xp.max(0)).unwrap_or(u32::MAX));
    RankView {
        rank: info.rank,
        progress_percent: info.progress_percent,
        next_threshold: info.next_threshold,
    }
}

fn streak_view(status: StreakStatus) -> StreakView {
    StreakView {
        current: status.current,
        longest: status.longest,
        active_today: status.active_today,
        rest_tokens_available: status.rest_tokens_available,
        tokens_needed_to_continue: status.tokens_needed_to_continue,
    }
}

/// The amount of a kind of practice comes from the `game` rules, which depend
/// on the kind and on nothing else (not on the score).
fn spark_source(kind: XpSourceKind) -> Option<SparkSource> {
    match kind {
        XpSourceKind::Lesson => Some(SparkSource::Activity),
        XpSourceKind::Conversation | XpSourceKind::TextChat => Some(SparkSource::Conversation),
        XpSourceKind::Writing => Some(SparkSource::FreeWriting),
        XpSourceKind::Reading => Some(SparkSource::FreeReading),
        XpSourceKind::Drill => Some(SparkSource::Drill),
        XpSourceKind::Review => Some(SparkSource::Review),
        XpSourceKind::Checkpoint => Some(SparkSource::Checkpoint),
        XpSourceKind::Placement => Some(SparkSource::Placement),
        XpSourceKind::Bonus => None,
    }
}

fn find_cosmetic(id: &str) -> Option<&'static Cosmetic> {
    UNLOCKS.iter().find(|cosmetic| cosmetic.id == id)
}

impl AppCore {
    /// XP, rank, streak, rest tokens and cosmetics.
    pub async fn game_state(&self) -> CoreResult<GameState> {
        let profile = self.profile_id();
        let today = self.clock().today()?;
        let game = self.db.game();

        let totals = game.xp_totals(profile).await?;
        let streak = game.streak_status(profile, today).await?;
        let streak_days = game
            .streak_days(profile)
            .await?
            .into_iter()
            .filter(|day| today.days_since(day.local_date) < CALENDAR_DAYS)
            .map(|day| StreakDayView {
                date: day.local_date.to_string(),
                counted: day.counted,
            })
            .collect();
        let unlocked = game.unlocked(profile).await?;
        let worn = game.equipped(profile).await?;
        let equipped = EquippedView {
            accessory: worn
                .as_ref()
                .and_then(|w| w.mascot_accessory.as_deref())
                .map(catalogue_id),
            theme: worn
                .as_ref()
                .and_then(|w| w.theme.as_deref())
                .map(catalogue_id),
        };
        let cosmetics = UNLOCKS
            .iter()
            .map(|cosmetic| {
                let entry = unlocked
                    .iter()
                    .find(|u| catalogue_id(&u.unlockable_id) == cosmetic.id);
                CosmeticView {
                    id: cosmetic.id.to_owned(),
                    kind: cosmetic.kind.into(),
                    rank: cosmetic.rank,
                    unlocked: entry.is_some(),
                    unlocked_at: entry.map(|u| u.unlocked_at.to_string()),
                    equipped: equipped.accessory.as_deref() == Some(cosmetic.id)
                        || equipped.theme.as_deref() == Some(cosmetic.id),
                }
            })
            .collect();
        Ok(GameState {
            xp_total: totals.total,
            xp_by_source: totals
                .by_source
                .into_iter()
                .map(|s| XpBySourceView {
                    source: s.source_kind.into(),
                    amount: s.amount,
                })
                .collect(),
            rank: rank_view(totals.total),
            streak: streak_view(streak),
            streak_days,
            cosmetics,
            equipped,
        })
    }

    /// Wears an unlocked cosmetic, or takes the slot's item off with `id: null`.
    pub async fn equip(&self, request: EquipRequest) -> CoreResult<GameState> {
        self.ensure_running()?;
        let profile = self.profile_id();
        let now = self.clock().now();
        let stored = match request.id.as_deref() {
            None => None,
            Some(id) => {
                let cosmetic = find_cosmetic(id)
                    .ok_or_else(|| CoreError::InvalidInput("unknown cosmetic".to_owned()))?;
                let fits = matches!(
                    (request.slot, CosmeticKind::from(cosmetic.kind)),
                    (CosmeticSlot::Accessory, CosmeticKind::Accessory)
                        | (CosmeticSlot::Theme, CosmeticKind::Theme)
                );
                if !fits {
                    return Err(CoreError::InvalidInput(
                        "that cosmetic does not fit this slot".to_owned(),
                    ));
                }
                Some(stored_id(id))
            }
        };
        match request.slot {
            CosmeticSlot::Accessory => {
                self.db
                    .game()
                    .equip_accessory(profile, stored.as_deref(), &now)
                    .await?;
            }
            CosmeticSlot::Theme => {
                self.db
                    .game()
                    .equip_theme(profile, stored.as_deref(), &now)
                    .await?;
            }
        }
        self.game_state().await
    }

    /// Records one finished piece of practice: XP by the kind of practice, today
    /// on the streak, and the cosmetics the new rank unlocks. Called by session
    /// code when something completes; there is no HTTP route for it, so the
    /// browser cannot claim XP.
    ///
    /// `source_id` is a short code such as `session:42` (letters, digits and
    /// `. _ : / -`). Recording the same practice again adds nothing.
    pub async fn record_practice(
        &self,
        kind: XpSourceKind,
        source_id: &str,
    ) -> CoreResult<PracticeOutcome> {
        self.ensure_running()?;
        let spark = spark_source(kind).ok_or_else(|| {
            CoreError::InvalidInput("a bonus is not a kind of practice".to_owned())
        })?;
        let profile = self.profile_id();
        let now = self.clock().now();
        let today = self.clock().today()?;
        let game = self.db.game();

        let before = game.xp_total(profile).await?;
        let award = game
            .award_xp(&storage::NewXp {
                profile_id: profile,
                created_at: now,
                source_kind: kind.into(),
                source_id: source_id.to_owned(),
                amount: i64::from(game::award(spark)),
                reason: PRACTICE_REASON.to_owned(),
            })
            .await?;
        let xp_awarded = match award {
            XpAward::Awarded(entry) => entry.amount,
            XpAward::AlreadyAwarded => 0,
        };
        let recorded = game.record_activity(profile, today, &now).await?;
        let total = before + xp_awarded;
        let new_unlocks = self.unlock_for_rank(total).await?;
        Ok(PracticeOutcome {
            xp_awarded,
            xp_total: total,
            rank: rank_view(total),
            rank_up: rank_view(total).rank > rank_view(before).rank,
            new_unlocks,
            streak: streak_view(recorded.status),
        })
    }

    /// Grants one rest-day token. When and how often tokens are earned is a
    /// product decision that is not made yet, so the core grants none by itself;
    /// whoever decides the rule calls this.
    pub async fn grant_rest_token(&self, reason: &str) -> CoreResult<()> {
        self.ensure_running()?;
        self.db
            .game()
            .grant_rest_token(self.profile_id(), &self.clock().now(), reason)
            .await?;
        Ok(())
    }

    /// Unlocks every cosmetic of the current rank that is not unlocked yet.
    /// Idempotent. Called at start-up, after practice, and after the profile is
    /// recreated.
    pub(crate) async fn sync_unlocks(&self) -> CoreResult<()> {
        let total = self.db.game().xp_total(self.profile_id()).await?;
        self.unlock_for_rank(total).await.map(|_| ())
    }

    /// Returns the catalogue ids that were new.
    async fn unlock_for_rank(&self, xp_total: i64) -> CoreResult<Vec<String>> {
        let rank = rank_view(xp_total).rank;
        let now = self.clock().now();
        let mut fresh = Vec::new();
        for cosmetic in game::unlocked_at_rank(rank) {
            let is_new = self
                .db
                .game()
                .unlock(self.profile_id(), &stored_id(cosmetic.id), &now)
                .await?;
            if is_new {
                fresh.push(cosmetic.id.to_owned());
            }
        }
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_convert_both_ways_for_the_whole_catalogue() {
        for cosmetic in UNLOCKS {
            let stored = stored_id(cosmetic.id);
            assert!(
                stored.starts_with("accessory.") || stored.starts_with("theme."),
                "{stored}"
            );
            assert_eq!(catalogue_id(&stored), cosmetic.id);
        }
    }

    #[test]
    fn every_kind_of_practice_but_a_bonus_has_an_amount() {
        for kind in storage::XpSourceKind::ALL {
            let api: XpSourceKind = (*kind).into();
            assert_eq!(
                spark_source(api).is_some(),
                *kind != storage::XpSourceKind::Bonus,
                "{kind}"
            );
        }
    }

    #[test]
    fn a_negative_total_is_rank_one() {
        assert_eq!(rank_view(-5).rank, 1);
        assert_eq!(rank_view(i64::MAX).rank, 6);
    }
}
