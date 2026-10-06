//! The authored rubrics (`curriculum/catalogs/rubrics/*.json`, schema
//! `curriculum/schema/rubric.schema.json`) as the typed rubric the T3 prompt is
//! built from.
//!
//! The content validators check these files against the schema; this loader
//! reads what they accept and refuses what it cannot use: a dimension without
//! exactly the five bands 0 to 4, or an unknown dimension.

use std::collections::BTreeMap;
use std::path::Path;

use assessment_engine::Level;
use serde::Deserialize;

use super::{Dimension, RubricDimension, WorkshopRubric};

#[derive(Debug, thiserror::Error)]
pub enum RubricCatalogError {
    #[error("cannot read the rubrics folder: {0}")]
    Io(String),
    #[error("{file}: {reason}")]
    Invalid { file: String, reason: String },
    #[error("the rubric id {0} appears in two files")]
    Duplicate(String),
}

/// What a rubric is for (`task_family` of the schema).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskFamily {
    SpokenProduction,
    WrittenProduction,
    SpokenInteraction,
    Mediation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Text {
    en: String,
    #[allow(dead_code)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Band {
    band: u8,
    description: Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[allow(dead_code)]
    schema_version: String,
    id: String,
    version: u32,
    level: String,
    task_family: TaskFamily,
    dimensions: BTreeMap<String, Vec<Band>>,
}

/// One rubric with what the catalog knows about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRubric {
    pub rubric: WorkshopRubric,
    pub level: Level,
    pub family: TaskFamily,
}

/// The rubrics of a folder, by id.
#[derive(Debug, Clone, Default)]
pub struct RubricCatalog {
    rubrics: Vec<CatalogRubric>,
}

fn level_of(text: &str) -> Option<Level> {
    Level::ALL.into_iter().find(|l| l.as_str() == text)
}

fn dimension_of(name: &str) -> Option<Dimension> {
    match name {
        "task_achievement" => Some(Dimension::TaskAchievement),
        "range" => Some(Dimension::Range),
        "accuracy" => Some(Dimension::Accuracy),
        "coherence" => Some(Dimension::Coherence),
        "interaction" => Some(Dimension::Interaction),
        _ => None,
    }
}

fn bands_of(file: &str, name: &str, list: Vec<Band>) -> Result<[String; 5], RubricCatalogError> {
    let invalid = |reason: String| RubricCatalogError::Invalid {
        file: file.to_owned(),
        reason,
    };
    if list.len() != 5 {
        return Err(invalid(format!(
            "{name} has {} band descriptions, not 5",
            list.len()
        )));
    }
    let mut out: [String; 5] = Default::default();
    for (index, band) in list.into_iter().enumerate() {
        if usize::from(band.band) != index {
            return Err(invalid(format!(
                "{name}: the description at position {index} is for band {}",
                band.band
            )));
        }
        out[index] = band.description.en;
    }
    Ok(out)
}

impl RubricCatalog {
    /// Parses one rubric file's text. `file` only labels errors.
    pub fn parse_one(file: &str, text: &str) -> Result<CatalogRubric, RubricCatalogError> {
        let invalid = |reason: String| RubricCatalogError::Invalid {
            file: file.to_owned(),
            reason,
        };
        let parsed: File = serde_json::from_str(text).map_err(|e| invalid(e.to_string()))?;
        let level = level_of(&parsed.level)
            .ok_or_else(|| invalid(format!("{} is not a level", parsed.level)))?;
        // The dimensions in the order of the prompt contract, not the file's.
        let mut dimensions: Vec<RubricDimension> = Vec::new();
        let mut entries: Vec<(Dimension, String, Vec<Band>)> = Vec::new();
        for (name, list) in parsed.dimensions {
            let dimension =
                dimension_of(&name).ok_or_else(|| invalid(format!("unknown dimension {name}")))?;
            entries.push((dimension, name, list));
        }
        let order = |d: Dimension| match d {
            Dimension::TaskAchievement => 0,
            Dimension::Range => 1,
            Dimension::Accuracy => 2,
            Dimension::Coherence => 3,
            Dimension::Interaction => 4,
        };
        entries.sort_by_key(|(d, _, _)| order(*d));
        for (dimension, name, list) in entries {
            dimensions.push(RubricDimension {
                dimension,
                bands: bands_of(file, &name, list)?,
            });
        }
        if dimensions.is_empty() {
            return Err(invalid("the rubric has no dimension".to_owned()));
        }
        Ok(CatalogRubric {
            rubric: WorkshopRubric {
                id: parsed.id,
                version: parsed.version,
                dimensions,
            },
            level,
            family: parsed.task_family,
        })
    }

    /// Builds a catalog from `(file label, text)` pairs.
    pub fn from_texts<'a>(
        files: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, RubricCatalogError> {
        let mut rubrics: Vec<CatalogRubric> = Vec::new();
        for (file, text) in files {
            let one = Self::parse_one(file, text)?;
            if rubrics.iter().any(|r| r.rubric.id == one.rubric.id) {
                return Err(RubricCatalogError::Duplicate(one.rubric.id));
            }
            rubrics.push(one);
        }
        Ok(Self { rubrics })
    }

    /// Reads every `.json` file of a folder, in file-name order. Blocking: call
    /// it at start-up, not on an async worker.
    pub fn load_dir(dir: &Path) -> Result<Self, RubricCatalogError> {
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| RubricCatalogError::Io(e.kind().to_string()))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        let mut texts = Vec::with_capacity(paths.len());
        for path in &paths {
            let text = std::fs::read_to_string(path)
                .map_err(|e| RubricCatalogError::Io(e.kind().to_string()))?;
            let label = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            texts.push((label, text));
        }
        Self::from_texts(texts.iter().map(|(l, t)| (l.as_str(), t.as_str())))
    }

    pub fn len(&self) -> usize {
        self.rubrics.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rubrics.is_empty()
    }

    /// The rubric with this id, as units name it in `rubric_id`.
    pub fn get(&self, id: &str) -> Option<&CatalogRubric> {
        self.rubrics.iter().find(|r| r.rubric.id == id)
    }

    /// The rubric of a level and task family, the highest version when several.
    /// A roleplay names no rubric, so its scoring looks for the interaction
    /// rubric of its unit's level.
    pub fn for_family(&self, level: Level, family: TaskFamily) -> Option<&CatalogRubric> {
        self.rubrics
            .iter()
            .filter(|r| r.level == level && r.family == family)
            .max_by_key(|r| r.rubric.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimension_json(label: &str) -> String {
        let bands: Vec<String> = (0..5)
            .map(|b| format!(r#"{{"band": {b}, "description": {{"en": "{label} band {b}"}}}}"#))
            .collect();
        format!("[{}]", bands.join(","))
    }

    fn rubric_text(id: &str, family: &str, extra: &str) -> String {
        format!(
            r#"{{"schema_version": "1.0", "id": "{id}", "version": 1, "level": "A1",
                "task_family": "{family}",
                "dimensions": {{ "coherence": {c}, "task_achievement": {t}, "accuracy": {a}, "range": {r}{extra} }} }}"#,
            c = dimension_json("c"),
            t = dimension_json("t"),
            a = dimension_json("a"),
            r = dimension_json("r"),
        )
    }

    #[test]
    fn a_rubric_file_becomes_a_rubric_with_dimensions_in_prompt_order() {
        let text = rubric_text("rubric-a1-spoken-production", "spoken_production", "");
        let one = RubricCatalog::parse_one("x.json", &text).unwrap();
        assert_eq!(one.rubric.id, "rubric-a1-spoken-production");
        assert_eq!(one.rubric.version, 1);
        assert_eq!(one.level, Level::A1);
        assert_eq!(one.family, TaskFamily::SpokenProduction);
        let order: Vec<Dimension> = one.rubric.dimensions.iter().map(|d| d.dimension).collect();
        assert_eq!(
            order,
            [
                Dimension::TaskAchievement,
                Dimension::Range,
                Dimension::Accuracy,
                Dimension::Coherence
            ]
        );
        assert_eq!(one.rubric.dimensions[1].bands[3], "r band 3");
    }

    #[test]
    fn an_interaction_rubric_has_a_fifth_dimension() {
        let extra = format!(r#", "interaction": {}"#, dimension_json("i"));
        let text = rubric_text("rubric-a1-spoken-interaction", "spoken_interaction", &extra);
        let one = RubricCatalog::parse_one("x.json", &text).unwrap();
        assert_eq!(one.rubric.dimensions.len(), 5);
        assert_eq!(one.rubric.dimensions[4].dimension, Dimension::Interaction);
    }

    #[test]
    fn a_file_the_loader_cannot_use_is_refused_with_its_name() {
        let good = rubric_text("rubric-a1-x", "spoken_production", "");
        // A dimension with four descriptions instead of five.
        let four: Vec<String> = (0..4)
            .map(|b| format!(r#"{{"band": {b}, "description": {{"en": "c band {b}"}}}}"#))
            .collect();
        let short = good.replace(&dimension_json("c"), &format!("[{}]", four.join(",")));
        let error = RubricCatalog::parse_one("short.json", &short).unwrap_err();
        assert!(error.to_string().starts_with("short.json: "), "{error}");
        assert!(error.to_string().contains("4 band descriptions"), "{error}");
        let unknown = rubric_text("rubric-a1-x", "spoken_production", r#", "fluency": []"#);
        assert!(
            RubricCatalog::parse_one("u.json", &unknown)
                .unwrap_err()
                .to_string()
                .contains("unknown dimension fluency")
        );
        let bad_level = good.replace(r#""level": "A1""#, r#""level": "D4""#);
        assert!(RubricCatalog::parse_one("l.json", &bad_level).is_err());
        let wrong_order = good.replace(
            r#""band": 0, "description": {"en": "t band 0"}"#,
            r#""band": 1, "description": {"en": "t band 0"}"#,
        );
        assert!(RubricCatalog::parse_one("o.json", &wrong_order).is_err());
        assert!(RubricCatalog::parse_one("n.json", "not json").is_err());
    }

    #[test]
    fn the_catalog_finds_a_rubric_by_id_or_by_level_and_family_and_refuses_a_repeated_id() {
        let spoken = rubric_text("rubric-a1-spoken-production", "spoken_production", "");
        let written = rubric_text("rubric-a1-written-production", "written_production", "");
        let catalog =
            RubricCatalog::from_texts([("a.json", spoken.as_str()), ("b.json", written.as_str())])
                .unwrap();
        assert_eq!(catalog.len(), 2);
        assert!(catalog.get("rubric-a1-written-production").is_some());
        assert!(catalog.get("rubric-a1-nothing").is_none());
        let found = catalog
            .for_family(Level::A1, TaskFamily::WrittenProduction)
            .unwrap();
        assert_eq!(found.rubric.id, "rubric-a1-written-production");
        assert!(
            catalog
                .for_family(Level::A2, TaskFamily::WrittenProduction)
                .is_none()
        );
        assert!(
            catalog
                .for_family(Level::A1, TaskFamily::SpokenInteraction)
                .is_none()
        );
        assert!(matches!(
            RubricCatalog::from_texts([("a.json", spoken.as_str()), ("c.json", spoken.as_str())]),
            Err(RubricCatalogError::Duplicate(_))
        ));
    }
}
