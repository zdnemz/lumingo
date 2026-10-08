//! The cosmetic game layer: XP, streaks, rest tokens and cosmetics.
//!
//! Nothing here is a score or a level of English. Free-mode practice may earn
//! XP; XP never reaches an estimate, and no estimate reaches XP.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::mirror::api_enum;

api_enum! {
    /// Where an XP award came from.
    XpSourceKind <=> storage::XpSourceKind {
        Lesson => "lesson",
        Conversation => "conversation",
        TextChat => "text_chat",
        Writing => "writing",
        Reading => "reading",
        Drill => "drill",
        Review => "review",
        Checkpoint => "checkpoint",
        Placement => "placement",
        Bonus => "bonus",
    }
}

api_enum! {
    /// What a cosmetic is.
    CosmeticKind <=> game::CosmeticKind { Theme => "theme", Accessory => "accessory" }
}

/// Where a cosmetic is worn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CosmeticSlot {
    /// What Lumi wears.
    Accessory,
    /// The colour theme of the interface.
    Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct XpBySourceView {
    pub source: XpSourceKind,
    pub amount: i64,
}

/// Where the learner stands on the rank ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RankView {
    /// 1 to 6.
    pub rank: u8,
    /// Progress toward the next rank, 0 to 100.
    pub progress_percent: u8,
    /// The XP at which the next rank starts, `None` at the top rank.
    pub next_threshold: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StreakView {
    /// Active days in the chain that practice today would extend.
    pub current: u32,
    pub longest: u32,
    pub active_today: bool,
    /// Rest-day tokens held and not used.
    pub rest_tokens_available: u32,
    /// Tokens that practice today would use to keep the chain alive.
    pub tokens_needed_to_continue: u32,
}

/// One day of the streak calendar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StreakDayView {
    /// `YYYY-MM-DD`, the learner's own calendar day.
    pub date: String,
    /// True for a day of practice, false for a rest day covered by a token.
    pub counted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CosmeticView {
    /// For example `accessory-cap` or `theme-forest`.
    pub id: String,
    pub kind: CosmeticKind,
    /// The rank at which it unlocks.
    pub rank: u8,
    pub unlocked: bool,
    pub unlocked_at: Option<String>,
    pub equipped: bool,
}

/// What is worn now. `None` means nothing is worn in that slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EquippedView {
    pub accessory: Option<String>,
    pub theme: Option<String>,
}

/// `GET /api/game`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GameState {
    pub xp_total: i64,
    pub xp_by_source: Vec<XpBySourceView>,
    pub rank: RankView,
    pub streak: StreakView,
    /// The last 90 days that belong to a streak, oldest first.
    pub streak_days: Vec<StreakDayView>,
    /// Every cosmetic of the catalogue, unlocked or not.
    pub cosmetics: Vec<CosmeticView>,
    pub equipped: EquippedView,
}

/// `POST /api/game/equip`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct EquipRequest {
    pub slot: CosmeticSlot,
    /// The cosmetic to wear, or `null` to take the slot's item off.
    pub id: Option<String>,
}

/// What recording a finished piece of practice did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PracticeOutcome {
    /// XP added now. 0 when the same practice was recorded before.
    pub xp_awarded: i64,
    pub xp_total: i64,
    pub rank: RankView,
    pub rank_up: bool,
    /// Cosmetics unlocked by this call.
    pub new_unlocks: Vec<String>,
    pub streak: StreakView,
}
