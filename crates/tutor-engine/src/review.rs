//! Spaced review of vocabulary, grammar points and pronunciation targets.
//!
//! A small scheduler in the style of SM-2, written for this project. Days are
//! plain day numbers supplied by the caller (for example days since a fixed
//! date, in the learner's local calendar), so nothing here depends on a clock
//! or a time zone and tests are exact.

use serde::{Deserialize, Serialize};

/// How well an item was recalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    /// Not recalled. The item starts again.
    Again,
    /// Recalled with effort.
    Hard,
    Good,
    Easy,
}

impl Grade {
    /// Maps a 0 to 1 score from an objective item to a grade. A score that is not
    /// a number is a failed measurement and counts as not recalled: comparing NaN
    /// with the bands would otherwise grade it as the easiest.
    pub fn from_score(score: f64) -> Self {
        if score.is_nan() || score < 0.5 {
            Self::Again
        } else if score < 0.7 {
            Self::Hard
        } else if score < 0.9 {
            Self::Good
        } else {
            Self::Easy
        }
    }
}

const START_EASE: f64 = 2.5;
const MIN_EASE: f64 = 1.3;
/// Day gaps for the first two successful reviews, before the ease takes over.
const FIRST_INTERVAL: u32 = 1;
const SECOND_INTERVAL: u32 = 3;
/// A year is the longest gap, so nothing is scheduled out of sight.
const MAX_INTERVAL: u32 = 365;

/// Where one review item stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewState {
    /// How many times in a row it was recalled.
    pub repetitions: u32,
    /// Days until the next review, as scheduled at the last review.
    pub interval_days: u32,
    pub ease: f64,
    /// The day number on which it is next due.
    pub due: i64,
    /// How many times it was forgotten.
    pub lapses: u32,
}

impl ReviewState {
    /// A new item is due on the day it is added.
    pub fn new(today: i64) -> Self {
        Self {
            repetitions: 0,
            interval_days: 0,
            ease: START_EASE,
            due: today,
            lapses: 0,
        }
    }

    /// The state after reviewing on day `today` with `grade`.
    #[must_use]
    pub fn reviewed(&self, grade: Grade, today: i64) -> Self {
        let mut next = self.clone();
        match grade {
            Grade::Again => {
                next.lapses += 1;
                next.repetitions = 0;
                next.interval_days = 1;
                next.ease = (self.ease - 0.2).max(MIN_EASE);
            }
            Grade::Hard => {
                next.repetitions += 1;
                next.interval_days = scale(self.interval_days.max(1), 1.2);
                next.ease = (self.ease - 0.15).max(MIN_EASE);
            }
            Grade::Good => {
                next.repetitions += 1;
                next.interval_days = match self.repetitions {
                    0 => FIRST_INTERVAL,
                    1 => SECOND_INTERVAL,
                    _ => scale(self.interval_days.max(1), self.ease),
                };
            }
            Grade::Easy => {
                next.repetitions += 1;
                next.interval_days = match self.repetitions {
                    0 => SECOND_INTERVAL,
                    _ => scale(self.interval_days.max(1), self.ease * 1.3),
                };
                next.ease = self.ease + 0.15;
            }
        }
        next.interval_days = next.interval_days.clamp(1, MAX_INTERVAL);
        next.due = today + i64::from(next.interval_days);
        next
    }

    pub fn is_due(&self, today: i64) -> bool {
        self.due <= today
    }
}

fn scale(days: u32, factor: f64) -> u32 {
    (f64::from(days) * factor).round() as u32
}

/// Positions of the items due on `today`, most overdue first. Ties keep their
/// original order, so the list is stable between calls.
pub fn due_order(states: &[ReviewState], today: i64) -> Vec<usize> {
    let mut due: Vec<usize> = (0..states.len())
        .filter(|i| states[*i].is_due(today))
        .collect();
    due.sort_by_key(|i| states[*i].due);
    due
}

/// Weight of the newest result in the mastery average.
const MASTERY_WEIGHT: f64 = 0.3;

/// Objective mastery of an item or a grammar point: a moving average of its
/// results, so recent work counts more than old work. `None` for something that
/// has not been practised yet. A score that is not a number is no result: the
/// mastery stays as it was, or 0 when there was none.
pub fn update_mastery(previous: Option<f64>, score: f64) -> f64 {
    if score.is_nan() {
        return previous.unwrap_or(0.0);
    }
    let score = score.clamp(0.0, 1.0);
    match previous {
        None => score,
        Some(old) => MASTERY_WEIGHT * score + (1.0 - MASTERY_WEIGHT) * old,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_follow_the_score_bands() {
        assert_eq!(Grade::from_score(0.0), Grade::Again);
        assert_eq!(Grade::from_score(0.49), Grade::Again);
        assert_eq!(Grade::from_score(0.5), Grade::Hard);
        assert_eq!(Grade::from_score(0.7), Grade::Good);
        assert_eq!(Grade::from_score(0.9), Grade::Easy);
    }

    #[test]
    fn good_answers_stretch_the_interval_more_each_time() {
        let mut state = ReviewState::new(0);
        let mut day = 0;
        let mut gaps = Vec::new();
        for _ in 0..6 {
            state = state.reviewed(Grade::Good, day);
            gaps.push(state.interval_days);
            day = state.due;
        }
        assert_eq!(
            gaps[..3],
            [1, 3, 8],
            "1 day, then 3, then 3 times the ease of 2.5"
        );
        assert!(gaps.windows(2).all(|pair| pair[1] > pair[0]), "{gaps:?}");
    }

    #[test]
    fn forgetting_resets_the_run_counts_a_lapse_and_lowers_the_ease() {
        let mut state = ReviewState::new(0);
        for day in [0, 1, 4] {
            state = state.reviewed(Grade::Good, day);
        }
        let ease_before = state.ease;
        let after = state.reviewed(Grade::Again, 12);
        assert_eq!(after.repetitions, 0);
        assert_eq!(after.interval_days, 1);
        assert_eq!(after.lapses, 1);
        assert_eq!(after.due, 13);
        assert!(after.ease < ease_before);
    }

    #[test]
    fn the_ease_never_falls_below_its_floor() {
        let mut state = ReviewState::new(0);
        for day in 0..30 {
            state = state.reviewed(Grade::Again, day);
        }
        assert!((state.ease - 1.3).abs() < 1e-9);
        assert_eq!(state.lapses, 30);
        let hard = (0..30).fold(ReviewState::new(0), |s, day| s.reviewed(Grade::Hard, day));
        assert!((hard.ease - 1.3).abs() < 1e-9);
    }

    #[test]
    fn easy_grows_the_gap_faster_than_good_and_raises_the_ease() {
        let start = ReviewState::new(0)
            .reviewed(Grade::Good, 0)
            .reviewed(Grade::Good, 1);
        let good = start.reviewed(Grade::Good, 4);
        let easy = start.reviewed(Grade::Easy, 4);
        assert!(easy.interval_days > good.interval_days);
        assert!(easy.ease > good.ease);
    }

    #[test]
    fn hard_grows_the_gap_only_a_little() {
        let state = ReviewState::new(0)
            .reviewed(Grade::Good, 0)
            .reviewed(Grade::Good, 1);
        let hard = state.reviewed(Grade::Hard, 4);
        assert_eq!(hard.interval_days, 4, "3 days times 1.2 rounds to 4");
        assert!(hard.ease < state.ease);
    }

    #[test]
    fn the_interval_is_never_longer_than_a_year_or_shorter_than_a_day() {
        let mut state = ReviewState::new(0);
        for day in 0..40 {
            state = state.reviewed(Grade::Easy, day);
            assert!((1..=365).contains(&state.interval_days));
        }
        assert_eq!(state.interval_days, 365);
    }

    #[test]
    fn a_new_item_is_due_today_and_a_reviewed_one_is_not() {
        let state = ReviewState::new(10);
        assert!(state.is_due(10));
        assert!(!state.reviewed(Grade::Good, 10).is_due(10));
    }

    #[test]
    fn due_items_come_most_overdue_first_and_future_ones_are_left_out() {
        let mut a = ReviewState::new(0);
        a.due = 5;
        let mut b = ReviewState::new(0);
        b.due = 2;
        let mut c = ReviewState::new(0);
        c.due = 20;
        let mut d = ReviewState::new(0);
        d.due = 2;
        assert_eq!(due_order(&[a, b, c, d], 10), [1, 3, 0]);
    }

    #[test]
    fn mastery_is_a_moving_average_that_favours_recent_results() {
        assert_eq!(update_mastery(None, 1.0), 1.0);
        let after_miss = update_mastery(Some(1.0), 0.0);
        assert!((after_miss - 0.7).abs() < 1e-9);
        let recovered = update_mastery(Some(after_miss), 1.0);
        assert!(recovered > after_miss);
        assert!(update_mastery(Some(0.5), 5.0) <= 1.0, "scores are clamped");
    }

    #[test]
    fn a_score_that_is_not_a_number_is_not_recalled_and_is_no_mastery_result() {
        assert_eq!(Grade::from_score(f64::NAN), Grade::Again);
        assert_eq!(update_mastery(Some(0.6), f64::NAN), 0.6);
        assert_eq!(update_mastery(None, f64::NAN), 0.0);
        assert_eq!(Grade::from_score(f64::INFINITY), Grade::Easy);
        assert_eq!(Grade::from_score(f64::NEG_INFINITY), Grade::Again);
        assert_eq!(update_mastery(Some(0.5), f64::INFINITY), 0.3 + 0.7 * 0.5);
    }

    #[test]
    fn after_a_lapse_the_item_climbs_the_same_first_steps_again_with_a_lower_ease() {
        let mut state = ReviewState::new(0);
        let mut day = 0;
        for _ in 0..4 {
            state = state.reviewed(Grade::Good, day);
            day = state.due;
        }
        let grown = state.interval_days;
        assert!(grown > 3);
        let ease_before = state.ease;
        state = state.reviewed(Grade::Again, day);
        day = state.due;
        assert_eq!(
            (state.repetitions, state.interval_days, state.lapses),
            (0, 1, 1)
        );
        let mut gaps = Vec::new();
        for _ in 0..3 {
            state = state.reviewed(Grade::Good, day);
            gaps.push(state.interval_days);
            day = state.due;
        }
        assert_eq!(gaps[..2], [1, 3], "back to the first two steps");
        assert!(gaps[2] > 3);
        assert!(
            state.ease < ease_before,
            "a lapse is remembered in the ease"
        );
        assert_eq!(state.lapses, 1);
    }

    #[test]
    fn every_grade_schedules_at_least_a_day_ahead_and_stamps_the_due_day() {
        for grade in [Grade::Again, Grade::Hard, Grade::Good, Grade::Easy] {
            let state = ReviewState::new(100).reviewed(grade, 100);
            assert!(state.interval_days >= 1, "{grade:?}");
            assert_eq!(state.due, 100 + i64::from(state.interval_days), "{grade:?}");
            assert!(!state.is_due(100), "{grade:?}");
            assert!(state.is_due(state.due), "due on the day itself");
            assert!(state.is_due(state.due + 5), "and after it");
        }
    }

    #[test]
    fn the_first_easy_review_skips_the_first_step() {
        let state = ReviewState::new(0).reviewed(Grade::Easy, 0);
        assert_eq!(state.interval_days, 3);
        assert!(state.ease > 2.5);
    }

    #[test]
    fn reviewing_never_changes_the_state_it_started_from() {
        let before = ReviewState::new(7);
        let copy = before.clone();
        let _ = before.reviewed(Grade::Good, 7);
        assert_eq!(before, copy);
    }

    #[test]
    fn the_due_list_holds_exactly_the_items_due_by_the_day_and_is_stable_between_calls() {
        let at = |due: i64| {
            let mut state = ReviewState::new(0);
            state.due = due;
            state
        };
        let states = [at(3), at(10), at(10), at(11), at(-4)];
        let on_ten = due_order(&states, 10);
        assert_eq!(
            on_ten,
            [4, 0, 1, 2],
            "most overdue first, ties in input order, tomorrow left out"
        );
        assert_eq!(due_order(&states, 10), on_ten);
        assert_eq!(due_order(&states, 11), [4, 0, 1, 2, 3]);
        assert!(due_order(&states, -5).is_empty());
        assert!(due_order(&[], 10).is_empty());
    }

    #[test]
    fn mastery_stays_between_zero_and_one_for_any_run_of_results_and_follows_recent_work() {
        let runs: [&[f64]; 4] = [
            &[1.0; 20],
            &[0.0; 20],
            &[1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0],
            &[0.2, 0.9, 5.0, -3.0, 0.5, 1.0],
        ];
        for run in runs {
            let mut mastery = None;
            for score in run {
                let next = update_mastery(mastery, *score);
                assert!((0.0..=1.0).contains(&next), "{run:?}");
                mastery = Some(next);
            }
        }
        // The same results in the opposite order end differently: recent work counts more.
        let rising = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
            .iter()
            .fold(None, |m, s| Some(update_mastery(m, *s)))
            .unwrap();
        let falling = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0]
            .iter()
            .fold(None, |m, s| Some(update_mastery(m, *s)))
            .unwrap();
        assert!(rising > falling);
        // A constant result is a fixed point, and mastery moves toward it from either side.
        assert!((update_mastery(Some(0.8), 0.8) - 0.8).abs() < 1e-12);
        assert!(update_mastery(Some(0.2), 0.8) > 0.2 && update_mastery(Some(0.2), 0.8) < 0.8);
        assert!(update_mastery(Some(0.95), 0.8) < 0.95 && update_mastery(Some(0.95), 0.8) > 0.8);
    }
}
