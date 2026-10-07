//! The index row for one unit: what the database stores about a unit file, and
//! the checksums that tell whether a file changed.
//!
//! The units themselves stay JSON files; the index maps a unit id to its level,
//! sequence, title, objectives and file checksum, so a session or attempt can
//! point at a unit that is not open. This module knows nothing about the
//! database: it builds plain data, and the caller (the server's wiring) maps it
//! to `storage` types.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::load::{LoadError, UnitLoader};
use crate::model::{Level, Skill, Unit};

/// One objective as the index stores it. The id is `<unit id>/<objective id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitObjective {
    pub id: String,
    pub skill: Skill,
    pub can_do_en: String,
}

/// The row the database keeps for one unit file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitIndexEntry {
    pub id: String,
    pub level: Level,
    pub sequence: u32,
    pub schema_version: String,
    pub title_en: String,
    /// SHA-256 of the unit file's bytes, lower-case hex.
    pub file_sha256: String,
    pub objectives: Vec<UnitObjective>,
}

impl UnitIndexEntry {
    /// The index row for `unit`, whose file hashed to `file_sha256`.
    pub fn from_unit(unit: &Unit, file_sha256: &str) -> Self {
        Self {
            id: unit.id.clone(),
            level: unit.level,
            sequence: unit.sequence,
            schema_version: unit.schema_version.clone(),
            title_en: unit.title.en.clone(),
            file_sha256: file_sha256.to_owned(),
            objectives: unit
                .objectives
                .iter()
                .map(|objective| UnitObjective {
                    id: format!("{}/{}", unit.id, objective.id),
                    skill: objective.skill,
                    can_do_en: objective.can_do.en.clone(),
                })
                .collect(),
        }
    }
}

impl UnitLoader {
    /// Loads one unit file and returns its index row, checksum included.
    pub fn load_index_entry(&self, path: &Path) -> Result<UnitIndexEntry, LoadError> {
        let bytes = fs::read(path).map_err(|_| LoadError::Io(path.to_path_buf()))?;
        let unit = self.load_bytes(&bytes)?;
        Ok(UnitIndexEntry::from_unit(&unit, &sha256_hex(&bytes)))
    }
}

/// SHA-256 of `bytes` as 64 lower-case hex characters.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing to a String cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// One checksum for a set of units: SHA-256 over the sorted
/// `<unit id> <file checksum>` lines. Two sets with the same units and the same
/// file bytes always hash the same, whatever order they were read in.
pub fn manifest_checksum(entries: &[UnitIndexEntry]) -> String {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|entry| format!("{} {}\n", entry.id, entry.file_sha256))
        .collect();
    lines.sort();
    sha256_hex(lines.concat().as_bytes())
}

/// The name of a content version, derived from its manifest checksum. Deriving
/// it means an edited file is always a new version, so the index can never hold
/// one version name for two different sets of files.
pub fn content_version_for(manifest_sha256: &str) -> String {
    let short: String = manifest_sha256.chars().take(12).collect();
    format!("manifest-{short}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const EXAMPLE: &str = "../../curriculum/examples/a1-u01.example.json";

    fn example_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE)
    }

    #[allow(clippy::unwrap_used)] // test helper
    fn example_entry() -> UnitIndexEntry {
        UnitLoader::new().load_index_entry(&example_path()).unwrap()
    }

    #[test]
    fn sha256_matches_the_published_test_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_example_unit_has_an_index_row_with_its_checksum() {
        let entry = example_entry();
        assert_eq!(entry.id, "a1-u01");
        assert_eq!(entry.level, Level::A1);
        assert_eq!(entry.sequence, 1);
        assert_eq!(entry.schema_version, "1.0");
        assert_eq!(entry.title_en, "Hello! Nice to meet you");
        assert_eq!(entry.file_sha256.len(), 64);
        assert!(entry.file_sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(entry.objectives.len(), 6);
        assert_eq!(entry.objectives[0].id, "a1-u01/o1-greet");
        assert_eq!(entry.objectives[0].skill, Skill::SpeakingInteraction);
        assert!(!entry.objectives[0].can_do_en.is_empty());
    }

    #[test]
    fn the_checksum_is_of_the_file_bytes() {
        let path = example_path();
        let bytes = std::fs::read(&path).unwrap();
        let entry = example_entry();
        assert_eq!(entry.file_sha256, sha256_hex(&bytes));
        // A byte-for-byte copy has the same checksum; one edited byte does not.
        let mut edited = bytes.clone();
        edited.push(b'\n');
        assert_ne!(entry.file_sha256, sha256_hex(&edited));
    }

    #[test]
    fn the_manifest_checksum_ignores_the_order_of_the_units() {
        let mut first = example_entry();
        first.id = "a1-u01".to_owned();
        let mut second = example_entry();
        second.id = "a1-u02".to_owned();
        second.file_sha256 = sha256_hex(b"other bytes");
        assert_eq!(
            manifest_checksum(&[first.clone(), second.clone()]),
            manifest_checksum(&[second, first])
        );
    }

    #[test]
    fn the_manifest_checksum_changes_when_one_file_changes() {
        let entry = example_entry();
        let same = entry.clone();
        let mut changed = entry.clone();
        changed.file_sha256 = sha256_hex(b"different");
        let manifest = manifest_checksum(std::slice::from_ref(&entry));
        assert_eq!(
            manifest,
            manifest_checksum(std::slice::from_ref(&same)),
            "the same files hash the same"
        );
        assert_ne!(manifest, manifest_checksum(&[changed]));
    }

    #[test]
    fn the_content_version_names_the_manifest() {
        let manifest = "0123456789abcdef".to_owned() + &"0".repeat(48);
        assert_eq!(content_version_for(&manifest), "manifest-0123456789ab");
    }
}
