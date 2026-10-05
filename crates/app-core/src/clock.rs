//! The clock of the core.
//!
//! `storage` never reads the time: every timestamp and every calendar day is
//! passed in. The core passes them from a [`Clock`], so tests can fix "now" and
//! "today" and streak rules stay deterministic.

use chrono::Datelike;
use storage::{LocalDate, Timestamp};

use crate::error::{CoreError, CoreResult};

/// Where "now" and "today" come from.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Timestamp;
    /// The learner's current calendar day, in the learner's time zone. It fails
    /// only if the platform returns a date that does not exist.
    fn today(&self) -> CoreResult<LocalDate>;
}

/// The real clock: UTC instants and the machine's local calendar day.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn today(&self) -> CoreResult<LocalDate> {
        let local = chrono::Local::now().date_naive();
        LocalDate::from_ymd(local.year(), local.month(), local.day())
            .ok_or_else(|| CoreError::Internal("the local calendar day is out of range".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UTC calendar day of an instant. The stored timestamp form starts with
    /// `YYYY-MM-DD`.
    fn utc_day(instant: &Timestamp) -> LocalDate {
        let text = instant.to_string();
        LocalDate::parse(&text[..10]).expect("a timestamp starts with a date")
    }

    #[test]
    fn the_system_clock_gives_a_day_within_a_day_of_the_utc_day() {
        let clock = SystemClock;
        let today = clock.today().expect("today");
        let utc = utc_day(&clock.now());
        let difference = today.days_since(utc).abs();
        assert!(difference <= 1, "{today} vs {utc}");
    }
}
