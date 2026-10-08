# Rule-based grammar findings

Where the project stands on `harper-core` (assessment spec 5.3, context pack 5.7).

## The seam

`curriculum::validate::GrammarCheck` has one method, `findings(&str) -> Vec<String>`.
Three places use it, and each takes it as an `Option`:

| Place | With no checker |
|---|---|
| Rubric cross-check X5 (`tutor-engine`, `ScorerEnv.grammar`) | X5 is not evaluated. No alarm, no change of confidence. |
| Writing workshop, first layer (`WorkshopEnv.grammar`) | `DraftSubmission.rule_checked` is `false` and the list is empty. The UI must say "not checked", never "no findings". |
| Content warning W03 (`UnitOptions.grammar_check`) | The rule is reported as skipped. |

There is no stand-in that returns an empty list. A missing checker is a missing
checker.

Convention: a finding about spelling starts with `Spelling`. The rubric scorer
leaves those out of the X5 count for a voice response, because a transcript's
spelling is the recogniser's.

## Status: not linked, waiting for the owner

`harper-core` is Apache-2.0, but it needs `ammonia`, which needs `cssparser` and
`dtoa-short`. Both of those are MPL-2.0. `docs/LICENSE_REGISTER.md` rule 2 asks
for a `review` row and the owner's decision before MPL-2.0 enters the build, so
nothing was added to `Cargo.toml` and `cargo deny check licenses` was not
changed. The register row says what was read and what could not be.

Other findings that matter for the decision:

* Newer versions are heavier. From 0.55 `harper-pos-utils` depends on `burn`
  (a machine-learning framework). From 0.73 `burn` brings a second native `lzma`
  link that Cargo refuses next to `sherpa-onnx-sys`. 0.54.0 is the newest that
  resolves in this workspace.
* 0.54.0 builds on the pinned toolchain (checked outside the workspace, 36 s).
* What it finds, run on 2026-10-06 against five sentences: `an apple and a umbrella`
  gives "Incorrect indefinite article."; `teh cat sat on teh mat.` gives spelling
  and capitalisation findings; but **`I has a book.` and `She go to school every
  day.` give no finding at all**, and the name `Dewi` in a correct sentence is
  reported as a spelling mistake. Subject-verb agreement is the most common error
  of the target learners, so X5 built on this version would be a weak alarm, and
  names would raise false "spelling" findings in written work. This is a
  five-sentence check, not an evaluation.

## The adapter that was built and run

This is the whole adapter, compiled and tested against `harper-core =0.54.0` in a
scratch crate with its own `[workspace]` and a path dependency on `curriculum`.
It is not in the repository's build.

```toml
[dependencies]
curriculum = { version = "0.0.0", path = "../curriculum" }
harper-core = { version = "=0.54.0", default-features = false }
```

```rust
use std::sync::{Arc, Mutex};

use curriculum::validate::GrammarCheck;
use harper_core::linting::{LintGroup, LintKind, Linter};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};

pub struct HarperChecker {
    dictionary: Arc<FstDictionary>,
    group: Mutex<LintGroup>,
}

impl HarperChecker {
    pub fn new() -> Self {
        let dictionary = FstDictionary::curated();
        let group = LintGroup::new_curated(dictionary.clone(), Dialect::American);
        Self { dictionary, group: Mutex::new(group) }
    }
}

fn label(kind: LintKind) -> &'static str {
    match kind {
        LintKind::Spelling | LintKind::Typo => "Spelling",
        LintKind::Punctuation => "Punctuation",
        LintKind::Capitalization => "Capitalization",
        LintKind::Agreement => "Agreement",
        _ => "Grammar",
    }
}

impl GrammarCheck for HarperChecker {
    fn findings(&self, text: &str) -> Vec<String> {
        let document = Document::new_plain_english(text, &*self.dictionary);
        let Ok(mut group) = self.group.lock() else { return Vec::new() };
        group
            .lint(&document)
            .into_iter()
            .map(|lint| format!("{}: {}", label(lint.lint_kind), lint.message))
            .collect()
    }
}
```

If the owner accepts the licenses: put this in a small `crates/grammar-check`
crate, add the two `deny.toml` exceptions with their register rows, add
`Arc::new(HarperChecker::new())` to `UnitEnv.grammar`, `ScorerEnv.grammar` and
`WorkshopEnv.grammar`, and consider dropping findings whose text is a capitalised
word inside a sentence before they are counted.
