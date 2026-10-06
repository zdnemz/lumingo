//! How many times the audio of an activity may be played.
//!
//! A `listening_set` allows its first play and `replays_allowed` more. Other
//! audio items (a listening `mcq`, a dictation, a listening minimal pair) carry
//! no limit in the content format, so their audio may be played freely.

use curriculum::Activity;

use super::error::ActivityError;

/// Counts the plays of one activity's audio. The count is a `u8` that saturates
/// at the limit; with no limit it saturates at 255 and nothing is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayCounter {
    /// Total plays allowed, the first included. `None` means no limit.
    allowed: Option<u8>,
    played: u8,
}

impl PlayCounter {
    /// The counter for `activity`.
    pub fn for_activity(activity: &Activity) -> Self {
        let allowed = match activity {
            Activity::ListeningSet(a) => Some(a.replays_allowed.saturating_add(1)),
            _ => None,
        };
        Self { allowed, played: 0 }
    }

    /// Records one play and returns how many have happened, or refuses it when
    /// the limit is used. A refused play changes nothing.
    pub fn play(&mut self) -> Result<u8, ActivityError> {
        if let Some(allowed) = self.allowed
            && self.played >= allowed
        {
            return Err(ActivityError::NoReplaysLeft {
                allowed: allowed - 1,
            });
        }
        self.played = self.played.saturating_add(1);
        Ok(self.played)
    }

    pub fn played(&self) -> u8 {
        self.played
    }

    /// Replays left after the plays so far, or `None` when there is no limit.
    pub fn replays_left(&self) -> Option<u8> {
        self.allowed
            .map(|allowed| allowed.saturating_sub(self.played.max(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter(replays_allowed: u8) -> PlayCounter {
        PlayCounter {
            allowed: Some(replays_allowed.saturating_add(1)),
            played: 0,
        }
    }

    #[test]
    fn the_first_play_and_the_allowed_replays_are_accepted_and_the_next_is_refused() {
        let mut plays = counter(2);
        assert_eq!(plays.replays_left(), Some(2));
        assert_eq!(plays.play(), Ok(1));
        assert_eq!(plays.replays_left(), Some(2));
        assert_eq!(plays.play(), Ok(2));
        assert_eq!(plays.replays_left(), Some(1));
        assert_eq!(plays.play(), Ok(3));
        assert_eq!(plays.replays_left(), Some(0));
        assert_eq!(
            plays.play(),
            Err(ActivityError::NoReplaysLeft { allowed: 2 })
        );
        assert_eq!(plays.played(), 3, "a refused play changes nothing");
    }

    #[test]
    fn zero_replays_means_one_play() {
        let mut plays = counter(0);
        assert_eq!(plays.play(), Ok(1));
        assert_eq!(
            plays.play(),
            Err(ActivityError::NoReplaysLeft { allowed: 0 })
        );
    }

    #[test]
    fn no_limit_never_refuses() {
        let mut plays = PlayCounter {
            allowed: None,
            played: 0,
        };
        for _ in 0..300 {
            assert!(plays.play().is_ok());
        }
        assert_eq!(plays.played(), 255);
        assert_eq!(plays.replays_left(), None);
    }

    #[test]
    fn the_largest_allowance_does_not_overflow() {
        let mut plays = counter(u8::MAX);
        for _ in 0..255 {
            assert!(plays.play().is_ok());
        }
        assert!(plays.play().is_err());
    }
}
