//! Spaced-review scheduler in the style of SM-2, written for this project, and
//! the objective mastery average. Time is whole days since an arbitrary epoch, so
//! the logic is deterministic. Reviews and mastery never feed level estimates.

const START_EASE: f64 = 2.5;
const MIN_EASE: f64 = 1.3;
const MASTERY_WEIGHT: f64 = 0.3;

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewItem {
    pub ease: f64,
    pub interval_days: u32,
    /// Consecutive successful reviews.
    pub repetitions: u32,
    pub lapses: u32,
    /// Day on which the item is next due.
    pub due_day: i64,
}

impl ReviewItem {
    /// A new item is due on `today`.
    pub fn new(today: i64) -> Self {
        Self {
            ease: START_EASE,
            interval_days: 0,
            repetitions: 0,
            lapses: 0,
            due_day: today,
        }
    }

    /// Records one review with a score from 0 to 1. Below 0.6 (quality under 3 on
    /// the 0 to 5 scale) is a lapse: the item restarts at one day and its ease drops.
    pub fn review(&mut self, score: f64, today: i64) {
        let quality = (score.clamp(0.0, 1.0) * 5.0).round();
        let miss = 5.0 - quality;
        self.ease = (self.ease + 0.1 - miss * (0.08 + miss * 0.02)).max(MIN_EASE);
        if quality < 3.0 {
            self.repetitions = 0;
            self.lapses += 1;
            self.interval_days = 1;
        } else {
            self.interval_days = match self.repetitions {
                0 => 1,
                1 => 6,
                _ => (f64::from(self.interval_days) * self.ease).round() as u32,
            };
            self.repetitions += 1;
        }
        self.due_day = today + i64::from(self.interval_days);
    }
}

/// Indexes of items due on or before `today`, the most overdue first.
pub fn due(items: &[ReviewItem], today: i64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..items.len())
        .filter(|&i| items[i].due_day <= today)
        .collect();
    idx.sort_by_key(|&i| (items[i].due_day, i));
    idx
}

/// Moving average of results, newest weighted 0.3. `None` before the first result.
pub fn update_mastery(mastery: Option<f64>, score: f64) -> f64 {
    let score = score.clamp(0.0, 1.0);
    mastery.map_or(score, |m| m + MASTERY_WEIGHT * (score - m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_grow_one_six_then_by_ease() {
        let mut item = ReviewItem::new(0);
        let mut day = 0;
        let mut intervals = Vec::new();
        for _ in 0..4 {
            item.review(1.0, day);
            intervals.push(item.interval_days);
            day = item.due_day;
        }
        assert_eq!(&intervals[..2], [1, 6]);
        assert!(
            intervals[2] > 6 && intervals[3] > intervals[2],
            "{intervals:?}"
        );
        assert_eq!(
            item.due_day,
            1 + 6 + intervals[2] as i64 + intervals[3] as i64
        );
    }

    #[test]
    fn a_lapse_restarts_the_item_and_lowers_ease_but_not_below_the_floor() {
        let mut item = ReviewItem::new(0);
        item.review(1.0, 0);
        item.review(1.0, 1);
        let before = item.ease;
        item.review(0.2, 7);
        assert_eq!(
            (
                item.repetitions,
                item.interval_days,
                item.lapses,
                item.due_day
            ),
            (0, 1, 1, 8)
        );
        assert!(item.ease < before);
        for _ in 0..30 {
            item.review(0.0, 8);
        }
        assert_eq!(item.ease, MIN_EASE);
    }

    #[test]
    fn a_perfect_answer_raises_ease_and_a_barely_passing_one_lowers_it() {
        let mut good = ReviewItem::new(0);
        good.review(1.0, 0);
        assert!(good.ease > START_EASE);
        let mut shaky = ReviewItem::new(0);
        shaky.review(0.6, 0);
        assert!(shaky.ease < START_EASE);
        assert_eq!(shaky.repetitions, 1);
    }

    #[test]
    fn due_lists_items_on_or_before_today_most_overdue_first() {
        let mut items: Vec<ReviewItem> = (0..4).map(|_| ReviewItem::new(0)).collect();
        items[0].due_day = 9;
        items[1].due_day = 3;
        items[2].due_day = 5;
        items[3].due_day = 3;
        assert_eq!(due(&items, 5), [1, 3, 2]);
        assert!(due(&items, 2).is_empty());
    }

    #[test]
    fn mastery_is_a_moving_average_that_starts_at_the_first_result() {
        let m = update_mastery(None, 0.5);
        assert_eq!(m, 0.5);
        let m = update_mastery(Some(m), 1.0);
        assert!((m - 0.65).abs() < 1e-9);
        assert!(update_mastery(Some(0.9), 0.0) < 0.9);
        assert_eq!(update_mastery(None, 7.0), 1.0);
    }
}
