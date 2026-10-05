//! Rule codes, findings and per-unit reports.

use serde::Serialize;

/// How a finding counts. Any error fails validation, warnings are only listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

macro_rules! rule_codes {
    ($( $variant:ident => ($code:literal, $severity:ident, $description:literal) ),+ $(,)?) => {
        /// A validator rule. E, W and X codes are defined by `docs/CURRICULUM_SPEC.md` section 6.
        /// K codes are the catalog checks that the roadmap adds to the same command.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        pub enum RuleCode {
            $( $variant ),+
        }

        impl RuleCode {
            /// Every rule, in report order.
            pub const ALL: &'static [RuleCode] = &[ $( RuleCode::$variant ),+ ];

            /// The short code, for example `E07`.
            pub fn as_str(self) -> &'static str {
                match self { $( RuleCode::$variant => $code ),+ }
            }

            pub fn severity(self) -> Severity {
                match self { $( RuleCode::$variant => Severity::$severity ),+ }
            }

            /// One line saying what the rule demands.
            pub fn description(self) -> &'static str {
                match self { $( RuleCode::$variant => $description ),+ }
            }
        }
    };
}

rule_codes! {
    E01 => ("E01", Error, "valid against unit.schema.json"),
    E02 => ("E02", Error, "id equals the lower-case level, -u and the two-digit sequence"),
    E03 => ("E03", Error, "ids are unique within objectives, vocabulary, grammar, pronunciation, dialogues and activities"),
    E04 => ("E04", Error, "every objective_ids entry exists and every objective has at least one activity"),
    E05 => ("E05", Error, "mcq: answer_index is inside options, and passage and audio_text are not both present"),
    E06 => ("E06", Error, "gap_fill: the number of gaps equals the number of answer lists"),
    E07 => ("E07", Error, "reorder: answer uses exactly the tokens, and the tokens are not already in answer order"),
    E08 => ("E08", Error, "shadowing: dialogue_id exists"),
    E09 => ("E09", Error, "productive tasks: one model answer per band, min_words below max_words, the at answer inside the limits"),
    E10 => ("E10", Error, "roleplay: target grammar and vocabulary ids exist in the unit"),
    E11 => ("E11", Error, "checkpoint ids exist and none has scoring none"),
    E12 => ("E12", Error, "generation_policy.allowed_grammar_ids exist in the unit"),
    E13 => ("E13", Error, "review_items point to existing targets"),
    E14 => ("E14", Error, "A1 to B1: every localized text has an id value"),
    E15 => ("E15", Error, "TTS text uses only allowed characters, and dialogue lines respect the length limits"),
    E16 => ("E16", Error, "minimum content of CURRICULUM_SPEC section 4, including one checkpoint activity per skill"),
    E17 => ("E17", Error, "reading_set and listening_set: answer_index inside options, distinct stems, exactly one audio source that exists"),
    E18 => ("E18", Error, "reading_set: passage length inside the range for the level"),
    E19 => ("E19", Error, "error_correction: error_category is a category of the turn_analysis contract"),
    E20 => ("E20", Error, "error_correction: the sentence differs from every accepted answer after normalisation"),
    W01 => ("W01", Warning, "vocabulary profile: more than 5 percent of running words are above the unit level and not unit targets"),
    W02 => ("W02", Warning, "level_tag differs from the word list"),
    W03 => ("W03", Warning, "an at model answer has a finding from the rule-based grammar checker"),
    W04 => ("W04", Warning, "two activities have nearly the same stem or text"),
    W05 => ("W05", Warning, "Indonesian text is longer than twice or shorter than half of the English text"),
    W06 => ("W06", Warning, "guided_writing: the at model answer is outside the range for the level"),
    W07 => ("W07", Warning, "listening_set: the audio text is outside the range for the level"),
    X01 => ("X01", Error, "each level has sequences 1 to 30 with no gaps"),
    X02 => ("X02", Error, "prerequisites exist and point to earlier units"),
    X03 => ("X03", Error, "the unit matches its syllabus entry: objectives count, grammar ids and theme"),
    X04 => ("X04", Error, "no sentence longer than six words appears in two different units"),
    X05 => ("X05", Error, "every grammar id of the level syllabus is taught in one unit and practised in two"),
    X06 => ("X06", Error, "every rubric_id exists in catalogs/rubrics"),
    K01 => ("K01", Error, "a catalog file is valid against its JSON Schema"),
    K02 => ("K02", Error, "a catalog file keeps its identifiers unique and its references resolvable"),
    K03 => ("K03", Error, "a catalog covers everything the curriculum spec lists for it"),
}

impl std::fmt::Display for RuleCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rule violation or warning, located by the JSON pointer of the value it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: RuleCode,
    pub severity: Severity,
    /// JSON pointer into the file, `""` for the whole document.
    pub path: String,
    pub message: String,
}

impl Finding {
    pub fn new(code: RuleCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: code.severity(),
            path: path.into(),
            message: message.into(),
        }
    }
}

/// A rule that was not evaluated, and why. A skipped rule is never a silent pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub code: RuleCode,
    pub reason: String,
}

/// Everything found for one unit file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UnitReport {
    /// The file the unit came from, as given to the validator.
    pub file: String,
    /// The unit id, when the document had one.
    pub unit_id: Option<String>,
    pub findings: Vec<Finding>,
    pub skipped: Vec<Skipped>,
}

impl UnitReport {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
    }

    pub fn error_count(&self) -> usize {
        self.errors().count()
    }

    pub fn warning_count(&self) -> usize {
        self.warnings().count()
    }

    /// True when the unit has no error. Warnings do not matter.
    pub fn passed(&self) -> bool {
        self.error_count() == 0
    }

    /// The codes of the findings, in report order, without repeats.
    pub fn codes(&self) -> Vec<RuleCode> {
        let mut codes: Vec<RuleCode> = Vec::new();
        for finding in &self.findings {
            if !codes.contains(&finding.code) {
                codes.push(finding.code);
            }
        }
        codes
    }

    pub fn push(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    pub fn skip(&mut self, code: RuleCode, reason: impl Into<String>) {
        if !self.skipped.iter().any(|s| s.code == code) {
            self.skipped.push(Skipped {
                code,
                reason: reason.into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_unique_short_name_and_the_right_severity() {
        let mut seen = std::collections::HashSet::new();
        for code in RuleCode::ALL {
            assert!(seen.insert(code.as_str()), "duplicate {code}");
            let expected = if code.as_str().starts_with('W') {
                Severity::Warning
            } else {
                Severity::Error
            };
            assert_eq!(code.severity(), expected, "{code}");
            assert!(!code.description().is_empty());
        }
        // E01 to E20, W01 to W07, X01 to X06 and K01 to K03.
        assert_eq!(RuleCode::ALL.len(), 20 + 7 + 6 + 3);
    }
}
