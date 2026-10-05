//! The placement staircase (assessment spec, section 10).
//!
//! The learner answers blocks of four authored items at one level, starting at
//! A2. Three or four right moves up one level, zero or one moves down, and two
//! repeats the level once. It stops after six blocks or as soon as the learner
//! is bracketed between a level they passed and the one above it, which they
//! did not. The result is a *suggested* starting level, never a verdict.

use crate::types::Level;

/// Where the staircase starts.
pub const START_LEVEL: Level = Level::A2;
/// Items in one block.
pub const BLOCK_SIZE: usize = 4;
/// The most blocks a placement test uses.
pub const MAX_BLOCKS: usize = 6;

/// The staircase so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    level: Level,
    blocks_done: usize,
    repeated_here: bool,
    highest_passed: Option<Level>,
    lowest_failed: Option<Level>,
}

/// What to do after a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Give the learner a block at this level.
    Block(Level),
    /// Stop. The suggested starting level is the result.
    Done(Level),
}

impl Default for Placement {
    fn default() -> Self {
        Self::new()
    }
}

impl Placement {
    pub fn new() -> Self {
        Self {
            level: START_LEVEL,
            blocks_done: 0,
            repeated_here: false,
            highest_passed: None,
            lowest_failed: None,
        }
    }

    /// The level of the block to give next.
    pub fn level(&self) -> Level {
        self.level
    }

    pub fn blocks_done(&self) -> usize {
        self.blocks_done
    }

    fn up(level: Level) -> Option<Level> {
        Level::ALL
            .iter()
            .copied()
            .find(|candidate| *candidate > level)
    }

    fn down(level: Level) -> Option<Level> {
        Level::ALL
            .iter()
            .rev()
            .copied()
            .find(|candidate| *candidate < level)
    }

    /// What the learner can be suggested right now, from what has been passed
    /// and failed so far.
    fn suggestion(&self) -> Level {
        self.highest_passed.unwrap_or(Level::A1)
    }

    /// Records a finished block with `correct` right answers out of
    /// [`BLOCK_SIZE`] and returns the next step. Answers above the block size
    /// are treated as a full block.
    pub fn record_block(&mut self, correct: usize) -> Step {
        let correct = correct.min(BLOCK_SIZE);
        let here = self.level;
        self.blocks_done += 1;

        if correct >= 3 {
            self.highest_passed = Some(self.highest_passed.map_or(here, |h| h.max(here)));
            self.repeated_here = false;
            match Self::up(here) {
                // Passed the top level: nothing above to try.
                None => return Step::Done(here),
                Some(next) => {
                    if self.lowest_failed.is_some_and(|failed| failed <= next) {
                        // Passed here, failed the level above: bracketed.
                        return Step::Done(self.suggestion());
                    }
                    self.level = next;
                }
            }
        } else if correct <= 1 {
            self.lowest_failed = Some(self.lowest_failed.map_or(here, |f| f.min(here)));
            self.repeated_here = false;
            match Self::down(here) {
                // Failed even A1: the suggestion is A1.
                None => return Step::Done(Level::A1),
                Some(next) => {
                    if self.highest_passed.is_some_and(|passed| passed >= next) {
                        return Step::Done(self.suggestion());
                    }
                    self.level = next;
                }
            }
        } else if self.repeated_here {
            // Two right twice at the same level: this is where the learner stands.
            return Step::Done(here);
        } else {
            self.repeated_here = true;
        }

        if self.blocks_done >= MAX_BLOCKS {
            return Step::Done(self.suggestion());
        }
        Step::Block(self.level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A learner of true level `skill` answers an item at or below it right
    /// with the given chance, and above it rarely. The generator is a seeded
    /// linear congruential one, so every run is the same.
    fn simulate(skill: Level, seed: u64) -> (Level, usize) {
        let mut state = seed;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % 100
        };
        let mut placement = Placement::new();
        let mut level = placement.level();
        loop {
            let chance = if level <= skill { 92 } else { 15 };
            let correct = (0..BLOCK_SIZE).filter(|_| next() < chance).count();
            match placement.record_block(correct) {
                Step::Block(next_level) => level = next_level,
                Step::Done(result) => return (result, placement.blocks_done()),
            }
        }
    }

    fn rank(level: Level) -> i32 {
        Level::ALL
            .iter()
            .position(|l| *l == level)
            .map_or(0, |i| i as i32)
    }

    #[test]
    fn it_starts_at_a2() {
        assert_eq!(Placement::new().level(), Level::A2);
    }

    #[test]
    fn three_or_four_right_moves_up_and_zero_or_one_moves_down() {
        let mut up = Placement::new();
        assert_eq!(up.record_block(3), Step::Block(Level::B1));
        let mut top = Placement::new();
        assert_eq!(top.record_block(4), Step::Block(Level::B1));
        let mut down = Placement::new();
        assert_eq!(down.record_block(1), Step::Block(Level::A1));
        let mut none = Placement::new();
        assert_eq!(none.record_block(0), Step::Block(Level::A1));
    }

    #[test]
    fn two_right_repeats_the_level_once_and_then_settles_there() {
        let mut p = Placement::new();
        assert_eq!(p.record_block(2), Step::Block(Level::A2));
        assert_eq!(p.record_block(2), Step::Done(Level::A2));
    }

    #[test]
    fn passing_one_level_and_failing_the_next_brackets_the_learner() {
        let mut p = Placement::new();
        assert_eq!(p.record_block(3), Step::Block(Level::B1)); // passed A2
        assert_eq!(p.record_block(1), Step::Done(Level::A2)); // failed B1: bracketed
    }

    #[test]
    fn failing_a_level_and_then_passing_the_one_below_brackets_the_learner() {
        let mut p = Placement::new();
        assert_eq!(p.record_block(1), Step::Block(Level::A1)); // failed A2
        assert_eq!(p.record_block(4), Step::Done(Level::A1)); // passed A1
    }

    #[test]
    fn failing_a1_suggests_a1() {
        let mut p = Placement::new();
        assert_eq!(p.record_block(0), Step::Block(Level::A1));
        assert_eq!(p.record_block(0), Step::Done(Level::A1));
    }

    #[test]
    fn passing_c2_ends_at_c2() {
        let mut p = Placement::new();
        for expected in [Level::B1, Level::B2, Level::C1, Level::C2] {
            assert_eq!(p.record_block(4), Step::Block(expected));
        }
        assert_eq!(p.record_block(4), Step::Done(Level::C2));
    }

    #[test]
    fn it_never_runs_past_six_blocks() {
        let mut p = Placement::new();
        // Alternate 2 and a pass so no bracket forms early and the cap is what stops it.
        let mut steps = 0;
        let mut result = None;
        while result.is_none() && steps < 20 {
            steps += 1;
            if let Step::Done(level) = p.record_block(if steps % 2 == 1 { 4 } else { 2 }) {
                result = Some(level);
            }
        }
        assert!(result.is_some());
        assert!(p.blocks_done() <= MAX_BLOCKS);
    }

    #[test]
    fn an_impossible_score_counts_as_a_full_block() {
        let mut p = Placement::new();
        assert_eq!(p.record_block(99), Step::Block(Level::B1));
    }

    #[test]
    fn simulated_learners_are_placed_at_or_next_to_their_true_level() {
        for skill in Level::ALL {
            let mut within_one = 0;
            let trials = 200;
            for seed in 0..trials {
                let (placed, blocks) = simulate(skill, seed);
                assert!(blocks <= MAX_BLOCKS);
                if (rank(placed) - rank(skill)).abs() <= 1 {
                    within_one += 1;
                }
            }
            assert!(
                within_one * 100 >= trials * 90,
                "true level {skill:?}: only {within_one} of {trials} within one level"
            );
        }
    }
}
