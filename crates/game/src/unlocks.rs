use serde::{Deserialize, Serialize};

use crate::rank_for;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CosmeticKind {
    /// A colour theme for the whole interface.
    Theme,
    /// Something Lumi wears.
    Accessory,
}

/// One unlockable. The `id` is stored in the database and used by the
/// interface to pick a theme or a sprite, so ids never change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cosmetic {
    pub id: &'static str,
    pub kind: CosmeticKind,
    /// The rank at which it unlocks.
    pub rank: u8,
}

/// Every unlockable. Cosmetic only: none of these changes what is taught,
/// scored, or estimated.
pub const UNLOCKS: [Cosmetic; 8] = [
    Cosmetic {
        id: "accessory-cap",
        kind: CosmeticKind::Accessory,
        rank: 1,
    },
    Cosmetic {
        id: "theme-forest",
        kind: CosmeticKind::Theme,
        rank: 2,
    },
    Cosmetic {
        id: "accessory-headphones",
        kind: CosmeticKind::Accessory,
        rank: 2,
    },
    Cosmetic {
        id: "theme-ember",
        kind: CosmeticKind::Theme,
        rank: 3,
    },
    Cosmetic {
        id: "accessory-wizard",
        kind: CosmeticKind::Accessory,
        rank: 3,
    },
    Cosmetic {
        id: "accessory-bow",
        kind: CosmeticKind::Accessory,
        rank: 4,
    },
    Cosmetic {
        id: "accessory-glasses",
        kind: CosmeticKind::Accessory,
        rank: 5,
    },
    Cosmetic {
        id: "accessory-crown",
        kind: CosmeticKind::Accessory,
        rank: 6,
    },
];

/// Everything unlocked at or below a rank.
pub fn unlocked_at_rank(rank: u8) -> Vec<&'static Cosmetic> {
    UNLOCKS.iter().filter(|item| item.rank <= rank).collect()
}

/// Everything unlocked by a total number of sparks.
pub fn unlocked_for_sparks(sparks: u32) -> Vec<&'static Cosmetic> {
    unlocked_at_rank(rank_for(sparks).rank)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_ranks_are_on_the_ladder() {
        let mut ids: Vec<&str> = UNLOCKS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), UNLOCKS.len());
        assert!(UNLOCKS.iter().all(|c| (1..=6).contains(&c.rank)));
    }

    #[test]
    fn a_new_learner_has_the_first_hat_only() {
        let ids: Vec<&str> = unlocked_for_sparks(0).iter().map(|c| c.id).collect();
        assert_eq!(ids, ["accessory-cap"]);
    }

    #[test]
    fn unlocks_only_grow_with_sparks_and_nothing_is_ever_taken_away() {
        let mut previous = 0;
        for sparks in (0..=3200).step_by(10) {
            let count = unlocked_for_sparks(sparks).len();
            assert!(count >= previous, "at {sparks}");
            previous = count;
        }
        assert_eq!(unlocked_for_sparks(3000).len(), UNLOCKS.len());
    }

    #[test]
    fn the_themes_match_the_themes_the_interface_ships() {
        let themes: Vec<&str> = UNLOCKS
            .iter()
            .filter(|c| c.kind == CosmeticKind::Theme)
            .map(|c| c.id.trim_start_matches("theme-"))
            .collect();
        assert_eq!(themes, ["forest", "ember"]);
    }
}
