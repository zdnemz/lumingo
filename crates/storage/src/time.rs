//! The two time types the database stores.
//!
//! `Timestamp` is an instant in UTC with a fixed text form, so that comparing the
//! stored text orders rows by time ("the current estimate is the latest by
//! `computed_at`"). `LocalDate` is a calendar day with no time zone: the caller
//! decides what "today" is, so streak rules never depend on the machine's clock
//! or zone and tests stay deterministic.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Days, NaiveDate, SecondsFormat, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// A time value that could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    /// The text is not an RFC 3339 instant.
    #[error("not an RFC 3339 timestamp")]
    Timestamp,
    /// The text is not a calendar day written as YYYY-MM-DD.
    #[error("not a calendar day written as YYYY-MM-DD")]
    Date,
}

/// An instant in UTC, kept to millisecond precision.
///
/// The stored form is always `2026-10-04T08:15:30.123Z`: fixed width, so text
/// order equals time order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    /// Parses an RFC 3339 instant with any offset and converts it to UTC.
    pub fn parse(text: &str) -> Result<Self, TimeError> {
        let parsed = DateTime::parse_from_rfc3339(text).map_err(|_| TimeError::Timestamp)?;
        Ok(Self::from_utc(parsed.with_timezone(&Utc)))
    }

    /// The current instant. The only place in this crate that reads the clock.
    pub fn now() -> Self {
        Self::from_utc(Utc::now())
    }

    /// Builds a timestamp from whole seconds since the Unix epoch.
    pub fn from_unix_seconds(seconds: i64) -> Result<Self, TimeError> {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .map(Self::from_utc)
            .ok_or(TimeError::Timestamp)
    }

    /// The instant `days` earlier. Used for windows such as "the last 180 days".
    pub fn minus_days(&self, days: u32) -> Result<Self, TimeError> {
        self.0
            .checked_sub_days(Days::new(u64::from(days)))
            .map(Self::from_utc)
            .ok_or(TimeError::Timestamp)
    }

    /// Whole seconds since the Unix epoch, as the estimation code wants it.
    pub fn unix_seconds(&self) -> i64 {
        self.0.timestamp()
    }

    fn from_utc(value: DateTime<Utc>) -> Self {
        // Truncate below a millisecond so equal text means equal value.
        let nanos = value.nanosecond() / 1_000_000 * 1_000_000;
        Self(value.with_nanosecond(nanos).unwrap_or(value))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

impl FromStr for Timestamp {
    type Err = TimeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl TryFrom<String> for Timestamp {
    type Error = TimeError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Timestamp> for String {
    fn from(value: Timestamp) -> Self {
        value.to_string()
    }
}

impl Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A calendar day with no time zone, written `YYYY-MM-DD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate(NaiveDate);

impl LocalDate {
    /// Builds a day from year, month and day. `None` when the day does not exist.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        NaiveDate::from_ymd_opt(year, month, day).map(Self)
    }

    /// Parses the strict `YYYY-MM-DD` form.
    pub fn parse(text: &str) -> Result<Self, TimeError> {
        let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| TimeError::Date)?;
        // chrono accepts "2026-1-5"; the stored form is always zero padded.
        if date.format("%Y-%m-%d").to_string() == text {
            Ok(Self(date))
        } else {
            Err(TimeError::Date)
        }
    }

    /// The day `days` later, or `None` at the end of the supported range.
    pub fn plus_days(&self, days: u32) -> Option<Self> {
        self.0
            .checked_add_days(Days::new(u64::from(days)))
            .map(Self)
    }

    /// The day `days` earlier, or `None` at the start of the supported range.
    pub fn minus_days(&self, days: u32) -> Option<Self> {
        self.0
            .checked_sub_days(Days::new(u64::from(days)))
            .map(Self)
    }

    /// Whole days from `earlier` to `self`; negative when `earlier` is later.
    pub fn days_since(&self, earlier: LocalDate) -> i64 {
        self.0.signed_duration_since(earlier.0).num_days()
    }
}

impl fmt::Display for LocalDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.format("%Y-%m-%d"))
    }
}

impl FromStr for LocalDate {
    type Err = TimeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Serialize for LocalDate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LocalDate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(text: &str) -> Timestamp {
        Timestamp::parse(text).expect("valid timestamp")
    }

    #[test]
    fn timestamp_text_is_fixed_width_utc_with_milliseconds() {
        let cases = [
            ("2026-10-04T08:15:30.123Z", "2026-10-04T08:15:30.123Z"),
            ("2026-10-04T08:15:30Z", "2026-10-04T08:15:30.000Z"),
            ("2026-10-04T15:15:30+07:00", "2026-10-04T08:15:30.000Z"),
            ("2026-10-04T08:15:30.123456Z", "2026-10-04T08:15:30.123Z"),
        ];
        for (input, expected) in cases {
            assert_eq!(stamp(input).to_string(), expected, "{input}");
        }
    }

    #[test]
    fn timestamp_text_order_is_time_order() {
        let early = stamp("2026-01-09T23:59:59.999Z");
        let late = stamp("2026-01-10T00:00:00.000Z");
        assert!(early < late);
        assert!(early.to_string() < late.to_string());
    }

    #[test]
    fn timestamp_rejects_text_that_is_not_rfc3339() {
        for input in [
            "",
            "yesterday",
            "2026-10-04",
            "2026-10-04 08:15:30",
            "2026-13-01T00:00:00Z",
        ] {
            assert_eq!(
                Timestamp::parse(input),
                Err(TimeError::Timestamp),
                "{input}"
            );
        }
    }

    #[test]
    fn minus_days_crosses_month_ends() {
        let earlier = stamp("2026-03-01T12:00:00Z")
            .minus_days(1)
            .expect("in range");
        assert_eq!(earlier.to_string(), "2026-02-28T12:00:00.000Z");
    }

    #[test]
    fn unix_seconds_reads_the_same_instant_whatever_the_written_offset() {
        // The estimator compares these numbers with a window in seconds, so the
        // value must be the instant, not the text's local form.
        let cases = [
            ("1970-01-01T00:00:00Z", 0),
            ("2026-10-04T08:15:30Z", 1_791_101_730),
            ("2026-10-04T15:15:30+07:00", 1_791_101_730),
            // Milliseconds are truncated, not rounded.
            ("2026-10-04T08:15:30.999Z", 1_791_101_730),
        ];
        for (input, expected) in cases {
            assert_eq!(stamp(input).unix_seconds(), expected, "{input}");
        }
    }

    #[test]
    fn local_date_accepts_only_zero_padded_real_days() {
        assert!(LocalDate::parse("2026-02-28").is_ok());
        assert!(LocalDate::parse("2028-02-29").is_ok());
        for input in [
            "2026-02-29",
            "2026-2-9",
            "2026-02-30",
            "26-02-01",
            "2026-02-01T00:00",
            "",
        ] {
            assert_eq!(LocalDate::parse(input), Err(TimeError::Date), "{input}");
        }
    }

    #[test]
    fn local_date_arithmetic_is_calendar_based() {
        let day = LocalDate::from_ymd(2026, 12, 31).expect("real day");
        let next = day.plus_days(1).expect("in range");
        assert_eq!(next.to_string(), "2027-01-01");
        assert_eq!(next.days_since(day), 1);
        assert_eq!(day.days_since(next), -1);
    }

    #[test]
    fn both_types_round_trip_through_json_as_plain_strings() {
        let pair = (
            stamp("2026-10-04T08:15:30.123Z"),
            LocalDate::parse("2026-10-04").expect("date"),
        );
        let json = serde_json::to_string(&pair).expect("serialise");
        assert_eq!(json, r#"["2026-10-04T08:15:30.123Z","2026-10-04"]"#);
        let back: (Timestamp, LocalDate) = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, pair);
    }
}
