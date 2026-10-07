//! The topic banks of the free modes (`curriculum/catalogs/topics.json`):
//! per level, conversation scenarios, writing prompts and reading topics
//! (CURRICULUM_SPEC section 9). The catalog itself is C-05's to write; this
//! reader needs only the fields the free modes use and ignores everything
//! else, so the file can grow without breaking a reader.
//!
//! A conversation scenario is shaped like a `roleplay` activity: the tutor
//! role, the learner role, the scenario and the goals (CURRICULUM_SPEC
//! section 9). A level with no entries is empty, not an error.

use std::collections::BTreeMap;

use curriculum::{Level, Localized};
use serde::Deserialize;

/// A conversation scenario from the bank: like a `roleplay`, outside a unit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ConversationTopic {
    pub id: String,
    pub title: Localized,
    pub scenario: Localized,
    pub tutor_role: String,
    pub learner_role: String,
    pub goals: Vec<String>,
}

/// A writing prompt: what to write, for whom, and what it should cover.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WritingPrompt {
    pub id: String,
    pub prompt: Localized,
    pub reader: String,
    pub purpose: String,
    pub content_points: Vec<String>,
}

/// A topic for graded reading: a title and the kind of text it suggests.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReadingTopic {
    pub id: String,
    pub title: Localized,
    #[serde(default)]
    pub kind: Option<String>,
}

/// One level's entries. Every section is optional in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct LevelBank {
    #[serde(default)]
    pub conversations: Vec<ConversationTopic>,
    #[serde(default)]
    pub writing_prompts: Vec<WritingPrompt>,
    #[serde(default)]
    pub reading_topics: Vec<ReadingTopic>,
}

/// The whole bank, keyed by the level text of the file (`A1` to `C2`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TopicBank {
    #[serde(default)]
    levels: BTreeMap<String, LevelBank>,
}

impl TopicBank {
    /// Reads the catalog text.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    fn level(&self, level: Level) -> Option<&LevelBank> {
        self.levels.get(level_text(level))
    }

    /// The conversation scenarios of one level, in file order.
    pub fn conversations(&self, level: Level) -> &[ConversationTopic] {
        self.level(level)
            .map_or(&[], |bank| bank.conversations.as_slice())
    }

    /// The writing prompts of one level, in file order.
    pub fn writing_prompts(&self, level: Level) -> &[WritingPrompt] {
        self.level(level)
            .map_or(&[], |bank| bank.writing_prompts.as_slice())
    }

    /// The reading topics of one level, in file order.
    pub fn reading_topics(&self, level: Level) -> &[ReadingTopic] {
        self.level(level)
            .map_or(&[], |bank| bank.reading_topics.as_slice())
    }

    /// One conversation scenario by id.
    pub fn conversation(&self, level: Level, id: &str) -> Option<&ConversationTopic> {
        self.conversations(level)
            .iter()
            .find(|topic| topic.id == id)
    }
}

/// The level text the catalog keys use, the same as the JSON form.
fn level_text(level: Level) -> &'static str {
    match level {
        Level::A1 => "A1",
        Level::A2 => "A2",
        Level::B1 => "B1",
        Level::B2 => "B2",
        Level::C1 => "C1",
        Level::C2 => "C2",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A three-entry bank, the size the roadmap names for S4-11 until C-05
    /// writes the real catalog.
    const SAMPLE: &str = r#"{
        "schema_version": "1.0",
        "version": 1,
        "levels": {
            "A1": {
                "conversations": [
                    {
                        "id": "cafe", "title": {"en": "At a cafe", "id": "Di kafe"},
                        "scenario": {"en": "You order a drink and pay.", "id": "Anda memesan minuman."},
                        "tutor_role": "a friendly waiter", "learner_role": "a customer",
                        "goals": ["Order a drink", "Say the price"]
                    },
                    {
                        "id": "classmate", "title": {"en": "A new classmate"},
                        "scenario": {"en": "You meet a new classmate."},
                        "tutor_role": "a new classmate", "learner_role": "Yourself",
                        "goals": ["Say your name"]
                    },
                    {
                        "id": "family", "title": {"en": "Your family"},
                        "scenario": {"en": "You show a photo of your family."},
                        "tutor_role": "a friend", "learner_role": "Yourself",
                        "goals": ["Name two family members"]
                    }
                ],
                "writing_prompts": [{
                    "id": "note", "prompt": {"en": "Write a note to a friend."},
                    "reader": "a friend", "purpose": "invite",
                    "content_points": ["when", "where"]
                }],
                "reading_topics": [{"id": "pets", "title": {"en": "Pets"}, "kind": "story"}]
            }
        }
    }"#;

    #[allow(clippy::unwrap_used)] // test helper
    fn bank() -> TopicBank {
        TopicBank::parse(SAMPLE).unwrap()
    }

    #[test]
    fn the_bank_is_read_by_level_and_id() {
        let bank = bank();
        assert_eq!(bank.conversations(Level::A1).len(), 3);
        let topic = bank.conversation(Level::A1, "cafe").unwrap();
        assert_eq!(topic.title.en, "At a cafe");
        assert_eq!(topic.title.id.as_deref(), Some("Di kafe"));
        assert_eq!(topic.tutor_role, "a friendly waiter");
        assert_eq!(topic.goals.len(), 2);
        // The writing and reading sections come from the same file.
        assert_eq!(bank.writing_prompts(Level::A1).len(), 1);
        assert_eq!(bank.writing_prompts(Level::A1)[0].reader, "a friend");
        assert_eq!(bank.reading_topics(Level::A1).len(), 1);
        assert_eq!(
            bank.reading_topics(Level::A1)[0].kind.as_deref(),
            Some("story")
        );
    }

    #[test]
    fn a_level_or_section_with_no_entries_is_empty_not_an_error() {
        let bank = bank();
        assert!(bank.conversations(Level::B1).is_empty());
        assert!(bank.writing_prompts(Level::A1).len() == 1);
        assert!(bank.conversation(Level::A1, "nope").is_none());
        assert!(
            TopicBank::parse("{}")
                .unwrap()
                .conversations(Level::A1)
                .is_empty()
        );
        // A level with only conversations has empty other sections.
        let conversations_only = r#"{"levels": {"A2": {"conversations": []}}}"#;
        let bank = TopicBank::parse(conversations_only).unwrap();
        assert!(bank.conversations(Level::A2).is_empty());
        assert!(bank.writing_prompts(Level::A2).is_empty());
    }
}
