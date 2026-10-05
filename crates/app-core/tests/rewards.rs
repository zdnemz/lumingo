#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! The cosmetic game layer through the core.

mod common;

use app_core::AppCore;
use app_core::api::{CosmeticSlot, EquipRequest, ErrorCode, XpSourceKind};
use app_core::error::CoreError;
use common::{config, test_core};

fn equip(slot: CosmeticSlot, id: Option<&str>) -> EquipRequest {
    EquipRequest {
        slot,
        id: id.map(str::to_owned),
    }
}

fn cosmetic<'a>(state: &'a app_core::api::GameState, id: &str) -> &'a app_core::api::CosmeticView {
    state
        .cosmetics
        .iter()
        .find(|c| c.id == id)
        .expect("cosmetic")
}

#[tokio::test]
async fn a_new_learner_starts_at_rank_one_with_the_first_hat_unlocked() {
    let t = test_core().await;
    let state = t.core.game_state().await.unwrap();
    assert_eq!(state.xp_total, 0);
    assert!(state.xp_by_source.is_empty());
    assert_eq!(state.rank.rank, 1);
    assert_eq!(state.rank.next_threshold, Some(100));
    assert_eq!(state.streak.current, 0);
    assert!(state.streak_days.is_empty());
    assert_eq!(state.equipped.accessory, None);
    assert_eq!(state.equipped.theme, None);
    assert_eq!(state.cosmetics.len(), game::UNLOCKS.len());
    assert!(cosmetic(&state, "accessory-cap").unlocked);
    assert!(state.cosmetics.iter().filter(|c| c.unlocked).count() == 1);
}

#[tokio::test]
async fn practice_earns_the_xp_of_its_kind_once_and_extends_the_streak() {
    let t = test_core().await;
    let outcome = t
        .core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    assert_eq!(
        outcome.xp_awarded,
        i64::from(game::award(game::SparkSource::Activity))
    );
    assert_eq!(outcome.xp_total, outcome.xp_awarded);
    assert!(!outcome.rank_up && outcome.new_unlocks.is_empty());
    assert_eq!(outcome.streak.current, 1);
    assert!(outcome.streak.active_today);

    // The same practice again adds nothing.
    let again = t
        .core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    assert_eq!(again.xp_awarded, 0);
    assert_eq!(again.xp_total, outcome.xp_total);
    assert_eq!(again.streak.current, 1);

    let state = t.core.game_state().await.unwrap();
    assert_eq!(state.xp_total, outcome.xp_total);
    assert_eq!(state.xp_by_source.len(), 1);
    assert_eq!(state.xp_by_source[0].source, XpSourceKind::Lesson);
    assert_eq!(state.streak_days.len(), 1);
    assert_eq!(state.streak_days[0].date, "2026-10-05");
    assert!(state.streak_days[0].counted);
}

#[tokio::test]
async fn free_modes_earn_xp_but_never_touch_assessment() {
    let t = test_core().await;
    for (kind, id) in [
        (XpSourceKind::Writing, "session:2"),
        (XpSourceKind::Reading, "session:3"),
        (XpSourceKind::TextChat, "session:4"),
    ] {
        let outcome = t.core.record_practice(kind, id).await.unwrap();
        assert!(outcome.xp_awarded > 0, "{kind:?}");
    }
    let progress = t.core.progress().await.unwrap();
    assert!(progress.estimates.is_empty(), "XP is not evidence");
    assert!(progress.objectives.is_empty() && progress.units.is_empty());
    let attempts = t
        .core
        .database()
        .attempts()
        .for_skill_since(
            t.core.profile_id(),
            "writing",
            &storage::Timestamp::parse("2000-01-01T00:00:00.000Z").unwrap(),
        )
        .await
        .unwrap();
    assert!(attempts.is_empty());
}

#[tokio::test]
async fn a_new_rank_unlocks_its_cosmetics_and_says_so_once() {
    let t = test_core().await;
    let first = t
        .core
        .record_practice(XpSourceKind::Checkpoint, "session:10")
        .await
        .unwrap();
    assert!(!first.rank_up, "50 XP is still rank 1");
    let second = t
        .core
        .record_practice(XpSourceKind::Checkpoint, "session:11")
        .await
        .unwrap();
    assert_eq!(second.xp_total, 100);
    assert!(second.rank_up);
    assert_eq!(second.rank.rank, 2);
    let mut unlocked = second.new_unlocks.clone();
    unlocked.sort_unstable();
    assert_eq!(unlocked, ["accessory-headphones", "theme-forest"]);

    let third = t
        .core
        .record_practice(XpSourceKind::Lesson, "session:12")
        .await
        .unwrap();
    assert!(!third.rank_up && third.new_unlocks.is_empty());
}

#[tokio::test]
async fn equipping_needs_an_unlocked_cosmetic_that_fits_the_slot() {
    let t = test_core().await;
    let worn = t
        .core
        .equip(equip(CosmeticSlot::Accessory, Some("accessory-cap")))
        .await
        .unwrap();
    assert_eq!(worn.equipped.accessory.as_deref(), Some("accessory-cap"));
    assert!(cosmetic(&worn, "accessory-cap").equipped);

    let cases = [
        ("locked", CosmeticSlot::Accessory, "accessory-crown"),
        ("wrong slot", CosmeticSlot::Theme, "accessory-cap"),
        ("unknown", CosmeticSlot::Accessory, "accessory-nothing"),
        (
            "a theme not yet unlocked",
            CosmeticSlot::Theme,
            "theme-forest",
        ),
    ];
    for (label, slot, id) in cases {
        let error = t.core.equip(equip(slot, Some(id))).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidInput, "{label}: {error:?}");
    }
    // The refused calls changed nothing.
    let state = t.core.game_state().await.unwrap();
    assert_eq!(state.equipped.accessory.as_deref(), Some("accessory-cap"));
    assert_eq!(state.equipped.theme, None);

    // After the rank is reached the theme can be worn, and taken off.
    t.core
        .record_practice(XpSourceKind::Checkpoint, "session:20")
        .await
        .unwrap();
    t.core
        .record_practice(XpSourceKind::Checkpoint, "session:21")
        .await
        .unwrap();
    let themed = t
        .core
        .equip(equip(CosmeticSlot::Theme, Some("theme-forest")))
        .await
        .unwrap();
    assert_eq!(themed.equipped.theme.as_deref(), Some("theme-forest"));
    let bare = t
        .core
        .equip(equip(CosmeticSlot::Accessory, None))
        .await
        .unwrap();
    assert_eq!(bare.equipped.accessory, None);
    assert_eq!(bare.equipped.theme.as_deref(), Some("theme-forest"));
}

#[tokio::test]
async fn bad_practice_requests_are_refused() {
    let t = test_core().await;
    let bonus = t
        .core
        .record_practice(XpSourceKind::Bonus, "session:1")
        .await
        .unwrap_err();
    assert!(matches!(bonus, CoreError::InvalidInput(_)), "{bonus:?}");
    // A source id is a short code, never text.
    let text = t
        .core
        .record_practice(XpSourceKind::Lesson, "I wrote this sentence")
        .await
        .unwrap_err();
    assert_eq!(text.code(), ErrorCode::InvalidInput, "{text:?}");
    assert_eq!(t.core.game_state().await.unwrap().xp_total, 0);
}

#[tokio::test]
async fn a_missed_day_without_a_token_starts_again_and_nothing_goes_negative() {
    let t = test_core().await;
    t.core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    t.clock.set("2026-10-06T08:00:00.000Z", "2026-10-06");
    let next = t
        .core
        .record_practice(XpSourceKind::Lesson, "session:2")
        .await
        .unwrap();
    assert_eq!(next.streak.current, 2);

    // Three days pass.
    t.clock.set("2026-10-10T08:00:00.000Z", "2026-10-10");
    let state = t.core.game_state().await.unwrap();
    assert_eq!(state.streak.current, 0, "no token covers the gap");
    assert_eq!(state.streak.longest, 2, "the best chain is kept");
    assert!(state.xp_total > 0, "XP never goes down");
    let back = t
        .core
        .record_practice(XpSourceKind::Lesson, "session:3")
        .await
        .unwrap();
    assert_eq!(back.streak.current, 1);
}

#[tokio::test]
async fn a_rest_token_covers_one_missed_day() {
    let t = test_core().await;
    t.core
        .record_practice(XpSourceKind::Lesson, "session:1")
        .await
        .unwrap();
    t.core.grant_rest_token("gift").await.unwrap();
    assert_eq!(
        t.core
            .game_state()
            .await
            .unwrap()
            .streak
            .rest_tokens_available,
        1
    );

    // One day is skipped.
    t.clock.set("2026-10-07T08:00:00.000Z", "2026-10-07");
    let state = t.core.game_state().await.unwrap();
    assert_eq!(state.streak.tokens_needed_to_continue, 1);
    let outcome = t
        .core
        .record_practice(XpSourceKind::Lesson, "session:2")
        .await
        .unwrap();
    assert_eq!(outcome.streak.current, 2, "the rest day keeps the chain");
    assert_eq!(outcome.streak.rest_tokens_available, 0);
    let days = t.core.game_state().await.unwrap().streak_days;
    assert_eq!(days.len(), 3);
    assert!(!days[1].counted, "the covered day is a rest day");
}

#[tokio::test]
async fn game_state_survives_a_restart() {
    let t = test_core().await;
    for n in 0..2 {
        t.core
            .record_practice(XpSourceKind::Checkpoint, &format!("session:{n}"))
            .await
            .unwrap();
    }
    t.core
        .equip(equip(CosmeticSlot::Theme, Some("theme-forest")))
        .await
        .unwrap();
    let before = t.core.game_state().await.unwrap();
    t.core.close().await.unwrap();

    let again = AppCore::open(config(&t.dir, &t.clock, &[])).await.unwrap();
    assert_eq!(again.game_state().await.unwrap(), before);
}
