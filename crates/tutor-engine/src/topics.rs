//! The topic banks of the free modes (`curriculum/catalogs/topics.json`, schema
//! `curriculum/schema/topics.schema.json`): per level, conversation scenarios,
//! writing prompts and reading topics.
//!
//! The file is validated by `content-cli`; this reader only needs the fields the
//! free modes use and ignores the rest.

use std::collections::BTreeMap;

use assessment_engine::Level;
use serde::{Deserialize, Serialize};

/// A text in English and, usually, Indonesian.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedText {
    pub en: String,
    #[serde(default)]
    pub id: Option<String>,
}

/// A conversation scenario: like a `roleplay` activity, outside a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationTopic {
    pub id: String,
    pub title: LocalizedText,
    pub scenario: LocalizedText,
    pub tutor_role: String,
    pub learner_role: String,
    pub goals: Vec<String>,
}

/// A writing prompt with its reader, purpose and content points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritingPrompt {
    pub id: String,
    pub prompt: LocalizedText,
    pub reader: String,
    pub purpose: String,
    pub content_points: Vec<String>,
}

/// A topic for graded reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingTopic {
    pub id: String,
    pub title: LocalizedText,
    /// The kind of text it suggests: a message, a notice, a story, an article.
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelBank {
    #[serde(default)]
    pub conversations: Vec<ConversationTopic>,
    #[serde(default)]
    pub writing_prompts: Vec<WritingPrompt>,
    #[serde(default)]
    pub reading_topics: Vec<ReadingTopic>,
}

/// The whole bank, by level.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicBank {
    #[serde(default)]
    levels: BTreeMap<String, LevelBank>,
}

impl TopicBank {
    /// Reads `topics.json`.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    fn level(&self, level: Level) -> Option<&LevelBank> {
        self.levels.get(level.as_str())
    }

    pub fn conversations(&self, level: Level) -> &[ConversationTopic] {
        self.level(level)
            .map_or(&[], |l| l.conversations.as_slice())
    }

    pub fn writing_prompts(&self, level: Level) -> &[WritingPrompt] {
        self.level(level)
            .map_or(&[], |l| l.writing_prompts.as_slice())
    }

    pub fn reading_topics(&self, level: Level) -> &[ReadingTopic] {
        self.level(level)
            .map_or(&[], |l| l.reading_topics.as_slice())
    }

    pub fn conversation(&self, level: Level, id: &str) -> Option<&ConversationTopic> {
        self.conversations(level).iter().find(|t| t.id == id)
    }

    pub fn writing_prompt(&self, level: Level, id: &str) -> Option<&WritingPrompt> {
        self.writing_prompts(level).iter().find(|t| t.id == id)
    }

    pub fn reading_topic(&self, level: Level, id: &str) -> Option<&ReadingTopic> {
        self.reading_topics(level).iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "schema_version": "1.0", "version": 1,
        "levels": {
            "A1": {
                "conversations": [{
                    "id": "cafe", "title": {"en": "At a cafe", "id": "Di kafe"},
                    "scenario": {"en": "You order a drink."},
                    "tutor_role": "a waiter", "learner_role": "a customer",
                    "goals": ["Order a drink"]
                }],
                "writing_prompts": [{
                    "id": "note", "prompt": {"en": "Write a note to a friend."},
                    "reader": "a friend", "purpose": "invite", "content_points": ["when", "where"]
                }],
                "reading_topics": [{"id": "pets", "title": {"en": "Pets"}, "kind": "story"}]
            }
        }
    }"#;

    #[test]
    fn the_bank_is_read_by_level_and_id() {
        let bank = TopicBank::parse(SAMPLE).expect("parses");
        assert_eq!(bank.conversations(Level::A1).len(), 1);
        assert_eq!(
            bank.conversation(Level::A1, "cafe")
                .map(|t| t.tutor_role.as_str()),
            Some("a waiter")
        );
        assert_eq!(
            bank.writing_prompt(Level::A1, "note")
                .map(|p| p.content_points.len()),
            Some(2)
        );
        assert_eq!(
            bank.reading_topic(Level::A1, "pets")
                .and_then(|t| t.kind.as_deref()),
            Some("story")
        );
        assert!(bank.conversation(Level::A1, "nope").is_none());
    }

    #[test]
    fn a_level_with_no_entries_is_empty_not_an_error() {
        let bank = TopicBank::parse(SAMPLE).expect("parses");
        assert!(bank.conversations(Level::C2).is_empty());
        assert!(bank.writing_prompts(Level::B1).is_empty());
        assert!(bank.reading_topics(Level::B2).is_empty());
        assert!(
            TopicBank::parse("{}")
                .expect("parses")
                .conversations(Level::A1)
                .is_empty()
        );
    }

    #[test]
    fn bad_json_is_an_error() {
        assert!(TopicBank::parse("[1,2").is_err());
    }
}
