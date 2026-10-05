#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use common::{make_profile, temp_db, ts};
use storage::{Database, LocalDate, NewXp, StorageError, XpAward, XpSourceKind};

fn day(month: u32, day: u32) -> LocalDate {
    LocalDate::from_ymd(2026, month, day).expect("real day")
}

fn oct(d: u32) -> LocalDate {
    day(10, d)
}

/// Records an active day. The timestamp is deliberately unrelated to the date:
/// only the calendar day the caller passes decides anything.
async fn practise(db: &Database, profile_id: i64, date: LocalDate) -> storage::ActivityRecorded {
    db.game()
        .record_activity(profile_id, date, &ts(7))
        .await
        .expect("record activity")
}

#[tokio::test]
async fn consecutive_days_build_a_streak() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    for (n, d) in [1, 2, 3, 4].into_iter().enumerate() {
        let recorded = practise(&db, profile.id, oct(d)).await;
        assert!(recorded.newly_recorded);
        assert_eq!(recorded.status.current, u32::try_from(n + 1).unwrap());
        assert!(recorded.status.active_today);
    }
    let status = db
        .game()
        .streak_status(profile.id, oct(4))
        .await
        .expect("status");
    assert_eq!((status.current, status.longest), (4, 4));
}

#[tokio::test]
async fn recording_the_same_day_twice_changes_nothing() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    db.game()
        .grant_rest_token(profile.id, &ts(1), "welcome")
        .await
        .expect("token");
    let first = practise(&db, profile.id, oct(1)).await;
    let second = practise(&db, profile.id, oct(1)).await;

    assert!(first.newly_recorded);
    assert!(!second.newly_recorded);
    assert_eq!(second.tokens_spent, 0);
    assert_eq!(first.status, second.status);
    assert_eq!(second.status.current, 1);
    assert_eq!(
        db.game().streak_days(profile.id).await.expect("days").len(),
        1
    );
    assert_eq!(second.status.rest_tokens_available, 1);
}

#[tokio::test]
async fn a_missed_day_without_a_token_restarts_the_count_quietly() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    for d in [1, 2, 3] {
        practise(&db, profile.id, oct(d)).await;
    }

    // Day 4 and 5 pass with no activity. Before practising, the count reads zero.
    let idle = db
        .game()
        .streak_status(profile.id, oct(6))
        .await
        .expect("status");
    assert_eq!((idle.current, idle.longest), (0, 3));

    let back = practise(&db, profile.id, oct(6)).await;
    assert_eq!(back.status.current, 1);
    assert_eq!(back.status.longest, 3);
    assert_eq!(back.tokens_spent, 0);
    // No row exists for the missed days: nothing records a loss.
    let days: Vec<LocalDate> = db
        .game()
        .streak_days(profile.id)
        .await
        .expect("days")
        .iter()
        .map(|d| d.local_date)
        .collect();
    assert_eq!(days, [oct(1), oct(2), oct(3), oct(6)]);
}

#[tokio::test]
async fn a_rest_token_keeps_the_streak_across_a_missed_day() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    db.game()
        .grant_rest_token(profile.id, &ts(1), "milestone-7")
        .await
        .expect("token");
    for d in [1, 2, 3] {
        practise(&db, profile.id, oct(d)).await;
    }

    // Before practising on day 5 the app can already tell the streak is safe.
    let before = db
        .game()
        .streak_status(profile.id, oct(5))
        .await
        .expect("status");
    assert_eq!((before.current, before.tokens_needed_to_continue), (3, 1));

    let after = practise(&db, profile.id, oct(5)).await;
    assert_eq!(after.tokens_spent, 1);
    assert_eq!(after.status.current, 4);
    assert_eq!(after.status.rest_tokens_available, 0);

    let days = db.game().streak_days(profile.id).await.expect("days");
    let shape: Vec<(LocalDate, bool)> = days.iter().map(|d| (d.local_date, d.counted)).collect();
    assert_eq!(
        shape,
        [
            (oct(1), true),
            (oct(2), true),
            (oct(3), true),
            (oct(4), false),
            (oct(5), true)
        ]
    );
}

#[tokio::test]
async fn a_token_is_spent_only_when_there_are_enough_for_the_whole_gap() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    db.game()
        .grant_rest_token(profile.id, &ts(1), "one")
        .await
        .expect("token");
    practise(&db, profile.id, oct(1)).await;
    practise(&db, profile.id, oct(2)).await;

    // Days 3 and 4 were missed and there is only one token: it is not wasted.
    let back = practise(&db, profile.id, oct(5)).await;
    assert_eq!(back.tokens_spent, 0);
    assert_eq!(back.status.current, 1);
    assert_eq!(back.status.rest_tokens_available, 1);
}

#[tokio::test]
async fn tokens_are_spent_oldest_first_and_only_once() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let old = db
        .game()
        .grant_rest_token(profile.id, &ts(1), "old")
        .await
        .expect("old");
    let new = db
        .game()
        .grant_rest_token(profile.id, &ts(2), "new")
        .await
        .expect("new");
    practise(&db, profile.id, oct(1)).await;
    let back = practise(&db, profile.id, oct(3)).await;
    assert_eq!(back.tokens_spent, 1);

    let status = db
        .game()
        .streak_status(profile.id, oct(3))
        .await
        .expect("status");
    assert_eq!(status.rest_tokens_available, 1);
    // The oldest token went first; the newer one is still free.
    let left = db
        .game()
        .available_rest_tokens(profile.id)
        .await
        .expect("tokens");
    assert_eq!(left, vec![new]);
    assert_ne!(left[0].id, old.id);
    // A later gap spends the remaining one, never the first again.
    let later = practise(&db, profile.id, oct(5)).await;
    assert_eq!(later.tokens_spent, 1);
    assert_eq!(later.status.rest_tokens_available, 0);
    assert!(
        db.game()
            .available_rest_tokens(profile.id)
            .await
            .expect("tokens")
            .is_empty()
    );
}

#[tokio::test]
async fn the_streak_depends_only_on_the_dates_passed_in() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    // Same timestamp for every call, dates across a month and a year end.
    let dates = [
        LocalDate::from_ymd(2026, 12, 30).expect("day"),
        LocalDate::from_ymd(2026, 12, 31).expect("day"),
        LocalDate::from_ymd(2027, 1, 1).expect("day"),
    ];
    let mut last = None;
    for date in dates {
        last = Some(practise(&db, profile.id, date).await);
    }
    assert_eq!(last.expect("recorded").status.current, 3);
    // Asking about a day in the past reads only the days up to it.
    let past = db
        .game()
        .streak_status(profile.id, dates[0])
        .await
        .expect("status");
    assert_eq!(past.current, 1);
}

#[tokio::test]
async fn a_back_dated_day_is_stored_without_spending_tokens() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    db.game()
        .grant_rest_token(profile.id, &ts(1), "spare")
        .await
        .expect("token");
    practise(&db, profile.id, oct(1)).await;
    practise(&db, profile.id, oct(10)).await;

    let back = practise(&db, profile.id, oct(5)).await;
    assert!(back.newly_recorded);
    assert_eq!(back.tokens_spent, 0);
    assert_eq!(back.status.rest_tokens_available, 1);
}

fn xp(profile_id: i64, kind: XpSourceKind, source: &str, amount: i64, reason: &str) -> NewXp {
    NewXp {
        profile_id,
        created_at: ts(50),
        source_kind: kind,
        source_id: source.to_owned(),
        amount,
        reason: reason.to_owned(),
    }
}

#[tokio::test]
async fn xp_totals_add_up_overall_and_by_source_including_free_modes() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let game = db.game();
    assert_eq!(game.xp_total(profile.id).await.expect("total"), 0);

    for award in [
        xp(
            profile.id,
            XpSourceKind::Lesson,
            "session:1",
            20,
            "activity.completed",
        ),
        xp(
            profile.id,
            XpSourceKind::Lesson,
            "session:2",
            30,
            "activity.completed",
        ),
        xp(
            profile.id,
            XpSourceKind::TextChat,
            "session:3",
            10,
            "free.chat",
        ),
        xp(
            profile.id,
            XpSourceKind::Writing,
            "session:4",
            15,
            "free.writing",
        ),
        xp(
            profile.id,
            XpSourceKind::Bonus,
            "streak:7",
            25,
            "streak.seven",
        ),
    ] {
        assert!(matches!(
            game.award_xp(&award).await.expect("award"),
            XpAward::Awarded(_)
        ));
    }

    assert_eq!(game.xp_total(profile.id).await.expect("total"), 100);
    let totals = game.xp_totals(profile.id).await.expect("totals");
    assert_eq!(totals.total, 100);
    let shape: Vec<(XpSourceKind, i64)> = totals
        .by_source
        .iter()
        .map(|s| (s.source_kind, s.amount))
        .collect();
    assert_eq!(
        shape,
        [
            (XpSourceKind::Lesson, 50),
            (XpSourceKind::TextChat, 10),
            (XpSourceKind::Writing, 15),
            (XpSourceKind::Bonus, 25),
        ]
    );
    assert_eq!(game.xp_since(profile.id, &ts(51)).await.expect("since"), 0);
    assert_eq!(
        game.xp_since(profile.id, &ts(50)).await.expect("since"),
        100
    );
    assert_eq!(
        game.xp_entries(profile.id, 2).await.expect("entries").len(),
        2
    );
}

#[tokio::test]
async fn the_same_award_is_not_counted_twice() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let award = xp(
        profile.id,
        XpSourceKind::Reading,
        "session:9",
        12,
        "free.reading",
    );
    assert!(matches!(
        db.game().award_xp(&award).await.expect("first"),
        XpAward::Awarded(_)
    ));
    assert_eq!(
        db.game().award_xp(&award).await.expect("retry"),
        XpAward::AlreadyAwarded
    );
    assert_eq!(db.game().xp_total(profile.id).await.expect("total"), 12);

    // A different reason for the same source is a different award.
    let bonus = xp(
        profile.id,
        XpSourceKind::Reading,
        "session:9",
        5,
        "free.reading.finished",
    );
    assert!(matches!(
        db.game().award_xp(&bonus).await.expect("bonus"),
        XpAward::Awarded(_)
    ));
    assert_eq!(db.game().xp_total(profile.id).await.expect("total"), 17);
}

#[tokio::test]
async fn xp_awards_cannot_be_zero_negative_huge_or_learner_text() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let bad = [
        xp(profile.id, XpSourceKind::Lesson, "session:1", 0, "ok"),
        xp(profile.id, XpSourceKind::Lesson, "session:1", -5, "ok"),
        xp(profile.id, XpSourceKind::Lesson, "session:1", 10_001, "ok"),
        xp(
            profile.id,
            XpSourceKind::Lesson,
            "I went to the market",
            5,
            "ok",
        ),
        xp(
            profile.id,
            XpSourceKind::Lesson,
            "session:1",
            5,
            "Great job on the essay!",
        ),
        xp(profile.id, XpSourceKind::Lesson, "", 5, "ok"),
    ];
    for award in &bad {
        assert!(
            matches!(db.game().award_xp(award).await, Err(StorageError::Rule(_))),
            "{award:?}"
        );
    }
    assert_eq!(db.game().xp_total(profile.id).await.expect("total"), 0);
}

#[tokio::test]
async fn only_unlocked_cosmetics_of_the_right_kind_can_be_worn() {
    let (_dir, db) = temp_db().await;
    let profile = make_profile(&db).await;
    let game = db.game();

    assert!(
        game.unlock(profile.id, "accessory.beret", &ts(1))
            .await
            .expect("unlock")
    );
    assert!(
        !game
            .unlock(profile.id, "accessory.beret", &ts(2))
            .await
            .expect("again")
    );
    assert!(
        game.unlock(profile.id, "theme.sunset", &ts(3))
            .await
            .expect("unlock")
    );

    // Not unlocked, wrong slot, malformed.
    assert!(matches!(
        game.equip_accessory(profile.id, Some("accessory.crown"), &ts(4))
            .await,
        Err(StorageError::Rule(_))
    ));
    assert!(matches!(
        game.equip_accessory(profile.id, Some("theme.sunset"), &ts(4))
            .await,
        Err(StorageError::Rule(_))
    ));
    assert!(matches!(
        game.unlock(profile.id, "A1 badge", &ts(4)).await,
        Err(StorageError::Rule(_))
    ));
    assert_eq!(game.equipped(profile.id).await.expect("equipped"), None);

    game.equip_accessory(profile.id, Some("accessory.beret"), &ts(5))
        .await
        .expect("equip");
    game.equip_theme(profile.id, Some("theme.sunset"), &ts(6))
        .await
        .expect("equip");
    let worn = game
        .equipped(profile.id)
        .await
        .expect("equipped")
        .expect("row");
    assert_eq!(worn.mascot_accessory.as_deref(), Some("accessory.beret"));
    assert_eq!(worn.theme.as_deref(), Some("theme.sunset"));

    game.equip_accessory(profile.id, None, &ts(7))
        .await
        .expect("take off");
    let worn = game
        .equipped(profile.id)
        .await
        .expect("equipped")
        .expect("row");
    assert_eq!(worn.mascot_accessory, None);
    assert_eq!(worn.theme.as_deref(), Some("theme.sunset"));
    assert_eq!(game.unlocked(profile.id).await.expect("unlocked").len(), 2);
}
