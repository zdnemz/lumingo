use serde::{Deserialize, Serialize};

/// What earned sparks. The names are written into the sparks ledger, so they
/// must stay stable once a database exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SparkSource {
    /// One finished activity inside a unit.
    Activity,
    /// A pronunciation drill round.
    Drill,
    /// One item cleared in the review queue.
    Review,
    /// A finished voice or text conversation.
    Conversation,
    /// A writing-workshop draft.
    FreeWriting,
    /// A graded reading passage with its questions.
    FreeReading,
    /// A unit or level checkpoint that was passed.
    Checkpoint,
    /// The placement test.
    Placement,
}

impl SparkSource {
    pub const ALL: [Self; 8] = [
        Self::Activity,
        Self::Drill,
        Self::Review,
        Self::Conversation,
        Self::FreeWriting,
        Self::FreeReading,
        Self::Checkpoint,
        Self::Placement,
    ];

    /// The stable name stored in the ledger.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Drill => "drill",
            Self::Review => "review",
            Self::Conversation => "conversation",
            Self::FreeWriting => "free_writing",
            Self::FreeReading => "free_reading",
            Self::Checkpoint => "checkpoint",
            Self::Placement => "placement",
        }
    }
}

/// Sparks for one finished piece of practice.
///
/// The amount depends on the kind of practice and nothing else. It does not
/// depend on the score, because sparks reward effort and must never double as
/// a hidden grade, and it has no daily cap, because a cap would turn into
/// pressure to stop or to hurry.
pub fn award(source: SparkSource) -> u32 {
    match source {
        SparkSource::Review => 3,
        SparkSource::Activity | SparkSource::Drill => 5,
        SparkSource::FreeReading => 15,
        SparkSource::Conversation => 15,
        SparkSource::FreeWriting => 20,
        SparkSource::Placement => 20,
        SparkSource::Checkpoint => 50,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_earns_something_and_has_a_distinct_stable_name() {
        let mut names: Vec<&str> = SparkSource::ALL.iter().map(|s| s.as_str()).collect();
        assert!(SparkSource::ALL.iter().all(|s| award(*s) > 0));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SparkSource::ALL.len());
    }

    #[test]
    fn the_serde_name_matches_the_ledger_name() {
        for source in SparkSource::ALL {
            let json = serde_json::to_string(&source).expect("serialise");
            assert_eq!(json, format!("\"{}\"", source.as_str()));
        }
    }

    #[test]
    fn a_checkpoint_is_worth_the_most() {
        let top = SparkSource::ALL.iter().map(|s| award(*s)).max();
        assert_eq!(top, Some(award(SparkSource::Checkpoint)));
    }
}
