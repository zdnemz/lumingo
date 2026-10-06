//! The catalogs the sessions read: rubrics, the topic bank and the word list.
//! They ship with the program next to the units. A catalog that is not there is
//! not an error: what needs it says so (a productive activity is stored unscored,
//! a topic-bank pick is refused) and everything else works.

use std::path::Path;
use std::sync::Arc;

use curriculum::validate::WordLevels;
use tutor_engine::{RubricCatalog, TopicBank};

use crate::config::CoreConfig;

/// What was loaded.
#[derive(Clone, Default)]
pub(crate) struct Catalogs {
    pub rubrics: Arc<RubricCatalog>,
    /// `None` when `topics.json` is not installed.
    pub topics: Option<Arc<TopicBank>>,
    /// `None` when no word list was configured or it could not be read.
    pub word_levels: Option<Arc<WordLevels>>,
}

impl Catalogs {
    /// Reads the catalogs from disk. Blocking: run it on a blocking thread.
    pub(crate) fn load(config: &CoreConfig) -> Self {
        let rubrics = match RubricCatalog::load_dir(&config.catalogs_dir.join("rubrics")) {
            Ok(catalog) => catalog,
            Err(error) => {
                tracing::info!(%error, "no rubrics were loaded; productive activities will be stored unscored");
                RubricCatalog::default()
            }
        };
        let topics =
            read(&config.catalogs_dir.join("topics.json")).and_then(|text| match TopicBank::parse(
                &text,
            ) {
                Ok(bank) => Some(Arc::new(bank)),
                Err(error) => {
                    tracing::warn!(%error, "topics.json is not valid; the topic bank is not used");
                    None
                }
            });
        let word_levels = config
            .word_list
            .as_deref()
            .and_then(read)
            .and_then(|text| match WordLevels::parse(&text) {
                Ok(levels) => Some(Arc::new(levels)),
                Err(error) => {
                    tracing::warn!(%error, "the word list is not valid; vocabulary checks are skipped");
                    None
                }
            });
        Self {
            rubrics: Arc::new(rubrics),
            topics,
            word_levels,
        }
    }
}

fn read(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) => {
            tracing::debug!(kind = ?error.kind(), "a catalog file was not read");
            None
        }
    }
}
