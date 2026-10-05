//! The streak rules as a pure function of the stored days.
//!
//! Dates are calendar days chosen by the caller, so the result never depends on
//! the clock or the time zone. Nothing here can go negative or record a loss: a
//! missed day that no token covers just means the count is zero until the next
//! active day starts a new one.

use serde::{Deserialize, Serialize};

use crate::time::LocalDate;

/// One stored streak day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Day {
    pub date: LocalDate,
    /// True for an active day, false for a rest day covered by a token.
    pub counted: bool,
}

/// Where a streak stands on a given day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreakStatus {
    /// Active days in the chain that practice today would extend. Zero when there
    /// is no chain to extend (never practised, or a gap no token covers).
    pub current: u32,
    /// The longest chain ever, counted the same way.
    pub longest: u32,
    /// Whether today already has an active day.
    pub active_today: bool,
    /// Rest-day tokens the learner holds and has not used.
    pub rest_tokens_available: u32,
    /// Tokens that practice today would use to keep the chain alive. Zero when the
    /// chain is alive without them, or when it cannot be saved.
    pub tokens_needed_to_continue: u32,
}

/// Evaluates the stored days as of `today`.
///
/// `days` holds every stored day in any order. Days after `today` are ignored.
pub(crate) fn evaluate(days: &[Day], today: LocalDate, tokens_available: u32) -> StreakStatus {
    let mut days: Vec<Day> = days.iter().copied().filter(|d| d.date <= today).collect();
    days.sort_by_key(|d| d.date);
    days.dedup_by_key(|d| d.date);

    let longest = longest_chain(&days);
    let Some(last) = days.last().copied() else {
        return StreakStatus {
            current: 0,
            longest,
            active_today: false,
            rest_tokens_available: tokens_available,
            tokens_needed_to_continue: 0,
        };
    };

    let chain = chain_ending_at(&days, days.len() - 1);
    // Days since the last stored day. 0 is today, 1 is yesterday.
    let since_last = today.days_since(last.date);
    let (current, needed) = match since_last {
        0 | 1 => (chain, 0),
        gap => {
            let missed = u32::try_from(gap - 1).unwrap_or(u32::MAX);
            if missed <= tokens_available {
                (chain, missed)
            } else {
                (0, 0)
            }
        }
    };
    StreakStatus {
        current,
        longest,
        active_today: since_last == 0,
        rest_tokens_available: tokens_available,
        tokens_needed_to_continue: needed,
    }
}

/// Active days in the run of consecutive dates that ends at `days[end]`.
fn chain_ending_at(days: &[Day], end: usize) -> u32 {
    let mut count = u32::from(days[end].counted);
    let mut index = end;
    while index > 0 && days[index].date.days_since(days[index - 1].date) == 1 {
        index -= 1;
        count += u32::from(days[index].counted);
    }
    count
}

fn longest_chain(days: &[Day]) -> u32 {
    let mut longest = 0;
    let mut run = 0;
    for (index, day) in days.iter().enumerate() {
        let continues = index > 0 && day.date.days_since(days[index - 1].date) == 1;
        let carried = if continues { run } else { 0 };
        run = carried + u32::from(day.counted);
        longest = longest.max(run);
    }
    longest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(day: u32) -> LocalDate {
        // Days of a 31-day month, so tests read as "day 5", "day 6".
        LocalDate::from_ymd(2026, 10, day).expect("real day")
    }

    fn active(day: u32) -> Day {
        Day {
            date: date(day),
            counted: true,
        }
    }

    fn rest(day: u32) -> Day {
        Day {
            date: date(day),
            counted: false,
        }
    }

    fn status(days: &[Day], today: u32, tokens: u32) -> (u32, u32, bool, u32) {
        let s = evaluate(days, date(today), tokens);
        (
            s.current,
            s.longest,
            s.active_today,
            s.tokens_needed_to_continue,
        )
    }

    #[test]
    fn no_days_means_no_streak() {
        assert_eq!(status(&[], 5, 0), (0, 0, false, 0));
    }

    #[test]
    fn consecutive_days_count_up_and_today_is_flagged() {
        let days = [active(3), active(4), active(5)];
        assert_eq!(status(&days, 5, 0), (3, 3, true, 0));
    }

    #[test]
    fn a_streak_stays_alive_until_the_end_of_the_next_day() {
        let days = [active(3), active(4), active(5)];
        // Day 6: not practised yet, yesterday counted, so it is still 3.
        assert_eq!(status(&days, 6, 0), (3, 3, false, 0));
    }

    #[test]
    fn a_missed_day_without_a_token_restarts_the_count_and_keeps_the_record() {
        let days = [active(1), active(2), active(3), active(4)];
        // Day 6: day 5 was missed and there is no token.
        assert_eq!(status(&days, 6, 0), (0, 4, false, 0));
        // After practising on day 6 the new chain is one day long.
        let after = [active(1), active(2), active(3), active(4), active(6)];
        assert_eq!(status(&after, 6, 0), (1, 4, true, 0));
    }

    #[test]
    fn a_token_preserves_the_streak_across_one_missed_day() {
        let days = [active(1), active(2), active(3)];
        // Day 5, one missed day (4) and one token: practising today keeps the chain.
        assert_eq!(status(&days, 5, 1), (3, 3, false, 1));
        // Once the rest day is stored, the chain continues through it.
        let bridged = [active(1), active(2), active(3), rest(4), active(5)];
        assert_eq!(status(&bridged, 5, 0), (4, 4, true, 0));
    }

    #[test]
    fn a_rest_day_does_not_add_to_the_count() {
        let days = [active(1), rest(2), active(3)];
        assert_eq!(status(&days, 3, 0), (2, 2, true, 0));
    }

    #[test]
    fn tokens_must_cover_the_whole_gap_or_none_are_used() {
        let days = [active(1), active(2)];
        // Two missed days (3 and 4), one token: not enough, so the count restarts.
        assert_eq!(status(&days, 5, 1), (0, 2, false, 0));
        // Two tokens are enough.
        assert_eq!(status(&days, 5, 2), (2, 2, false, 2));
    }

    #[test]
    fn the_longest_chain_survives_a_later_restart() {
        let days = [active(1), active(2), active(3), active(10), active(11)];
        assert_eq!(status(&days, 11, 0), (2, 3, true, 0));
    }

    #[test]
    fn days_after_today_and_duplicates_are_ignored() {
        let days = [active(4), active(5), active(5), active(20)];
        assert_eq!(status(&days, 5, 0), (2, 2, true, 0));
    }

    #[test]
    fn month_and_year_ends_are_consecutive() {
        let days = [
            Day {
                date: LocalDate::from_ymd(2026, 12, 31).expect("day"),
                counted: true,
            },
            Day {
                date: LocalDate::from_ymd(2027, 1, 1).expect("day"),
                counted: true,
            },
        ];
        let today = LocalDate::from_ymd(2027, 1, 1).expect("day");
        assert_eq!(evaluate(&days, today, 0).current, 2);
    }
}
