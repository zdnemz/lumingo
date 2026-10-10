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

## Status: linked (owner decision 2026-10-10, ADR-057)

**Update 2026-10-10.** The owner accepted the harper re-link. The tree now links
`harper-core` **0.68.0** (default features off, plus harper's `concurrent`
feature so the checker is `Send + Sync`):

| Where | What | Spelling |
|---|---|---|
| `assessment-engine` | default-on `grammar` feature; `GrammarChecker` (the concrete checker, `&self` + internal lock) | per call |
| `tutor-engine` | default-on `grammar` feature; `HarperCheck` implements the seam for `ScorerEnv`/`WorkshopEnv` | on |
| `content-cli` | its own `HarperCheck` for `UnitOptions.grammar_check` (W03) | **off** on purpose |
| `app-core` | one `HarperCheck` built in `Catalogs::load` (one dictionary load per process), shared by the unit runs and the workshop | on |
| `tutor-cli` | builds one for `unit run` and `unit score-pending` | on |

**Version.** The pre-merge line linked 2.11.0, but 2.11.0 (and 0.69+) cannot
resolve in this tree: `harper-pos-utils` → `burn` → `tracel-llvm-bundler` →
`liblzma-sys` links the native `lzma`, which conflicts with `sherpa-onnx-sys` →
`zip` → `xz2` → `lzma-sys`. The pre-merge tree had no `speech/sherpa`, so the
conflict never appeared there. 0.68.0 is the newest version that resolves next
to sherpa (checked empirically 2026-10-10: 0.55–0.68 resolve; 0.69, 0.70 and
0.72 fail). It still pulls `burn` 0.18.0 (272 new lock entries) but no second
native lzma link.

**Licenses.** harper-core, harper-brill and harper-pos-utils say `Apache-2.0`;
ammonia says `MIT OR Apache-2.0`; `cssparser`, `dtoa-short` and `colored` say
`MPL-2.0` (file-level copyleft, allowed for unmodified crates.io dependencies by
per-crate exceptions in `deny.toml` and the register row). `cargo deny check
licenses` passes.

**The split, and why it stays split.** W03 runs with spelling **off**: units are
full of names the dictionary does not know, and W03 is about grammar. The
workshop keeps spelling **on**: a typed draft is the learner's own spelling. The
rubric's X5 drops spelling findings for a **voice** response (the transcript's
spelling is the recogniser's) and keeps them for typed text. Do not "unify" the
three.

## What the linked checker actually finds (measured 2026-10-10, 0.68.0)

Run against the same sentences the 0.54.0 five-sentence check used:

| Sentence | Findings |
|---|---|
| `I has a book.` | **none** |
| `She go to school every day.` | **none** |
| `This is an test.` | `Miscellaneous: Incorrect indefinite article.` |
| `Last weekend I has a picnic with my family near the lake.` | **none** |
| `teh cat sat on teh mat.` | spelling, capitalisation, typo findings |
| `I recieve a letter.` | `Spelling: Did you mean to spell 'recieve' this way?` |

**The agreement gap persists in 0.68.0.** Subject-verb agreement is the most
common error of the target learners, and the checker does not catch it, so X5
built on this version is a weak alarm and the workshop's first layer is a
partial one. This is recorded, not hidden: the tests assert what the checker
does find (the article mistake, spelling) and never assert that a clean result
means correct grammar. Whether to layer a rule-based agreement check of the
project's own on top is an open question for the owner.

## History

* 2026-10-06: read the licenses, built the adapter against 0.54.0 outside the
  workspace, measured the five sentences.
* 2026-10-07: the owner approved `harper-core`; the pre-merge line linked 2.11.0
  with spelling off for W03.
* 2026-10-08: the merge kept origin's tree, dropping the linking (ADR-055).
* 2026-10-10: the owner accepted the re-link (ADR-057). Linked 0.68.0, the
  newest version that resolves next to sherpa; all four call sites wired; the
  agreement gap re-measured and recorded.
