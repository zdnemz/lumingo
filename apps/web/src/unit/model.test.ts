import { describe, expect, it } from "vitest";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { UnitSummary } from "@/generated/UnitSummary";
import { unitSummary } from "@/test/utils";
import { answerOf, emptyDraft, helpStartsOn, questRegions, skillProgress, skillsPractised, type Draft } from "./model";
import type { ActivityPresentation } from "@/generated/ActivityPresentation";
import { presented, unitDetail } from "@/test/utils";

function progress(units: ProgressOverview["units"]): ProgressOverview {
  return { units, objectives: [], errors: [], reviews_due: [], reviews_due_truncated: false, estimates: [], recent_sessions: [] };
}

function row(unit_id: string, status: ProgressOverview["units"][number]["status"]): ProgressOverview["units"][number] {
  return { unit_id, status, best_checkpoint: null, updated_at: "2026-01-01T00:00:00Z" };
}

const UNITS: UnitSummary[] = [
  unitSummary({ id: "a1-u01", sequence: 1 }),
  unitSummary({ id: "a1-u02", sequence: 2, prerequisites: ["a1-u01"] }),
  unitSummary({ id: "a1-u03", sequence: 3, prerequisites: ["a1-u02"] }),
  unitSummary({ id: "a2-u01", level: "A2", sequence: 1, prerequisites: ["a1-u03"] }),
];

describe("questRegions", () => {
  it("puts each unit in its own level and numbers it by its sequence", () => {
    const regions = questRegions(UNITS, progress([]), "en");
    expect(regions.map((region) => region.level)).toEqual(["A1", "A2"]);
    expect(regions[0]?.nodes.map((node) => node.number)).toEqual([1, 2, 3]);
  });

  it("calls a unit done only when the progress row says so", () => {
    const regions = questRegions(UNITS, progress([row("a1-u01", "passed")]), "en");
    expect(regions[0]?.nodes[0]?.state).toBe("done");
    expect(regions[0]?.nodes[1]?.state).toBe("current");
  });

  it("locks a unit whose prerequisite is not done, and leaves the first unit open", () => {
    const regions = questRegions(UNITS, progress([]), "en");
    expect(regions[0]?.nodes[0]?.state).toBe("current");
    expect(regions[0]?.nodes[1]?.state).toBe("locked");
    expect(regions[1]?.nodes[0]?.state).toBe("locked");
  });

  it("takes an unfinished run as the current unit, from the row that says so", () => {
    const regions = questRegions(UNITS, progress([row("a1-u01", "passed"), row("a1-u02", "in_progress")]), "en");
    expect(regions[0]?.nodes[1]?.state).toBe("current");
    // The unit after it still waits for the run in progress to be done.
    expect(regions[0]?.nodes[2]?.state).toBe("locked");
  });

  it("opens the unit after a done one, with no row of its own", () => {
    const regions = questRegions(UNITS, progress([row("a1-u01", "passed")]), "en");
    expect(regions[0]?.nodes[1]?.state).toBe("current");
    expect(regions[0]?.nodes[2]?.state).toBe("locked");
  });

  it("reads the title in the language the learner reads", () => {
    const units = [unitSummary({ id: "a1-u01", title: { en: "Hello", id: "Halo" } })];
    expect(questRegions(units, progress([]), "id")[0]?.nodes[0]?.title).toBe("Halo");
    expect(questRegions(units, progress([]), "en")[0]?.nodes[0]?.title).toBe("Hello");
  });

  it("counts the four skills the server counted", () => {
    expect(skillsPractised({ listening: 1, speaking: 2, reading: 0, writing: 3, mediation: 1, grammar: 9, vocabulary: 0, pronunciation: 4 })).toBe(3);
  });
});

describe("skillProgress", () => {
  const unit = unitDetail({
    id: "a1-u01",
    activities: [
      { type: "mcq", id: "a1", skill: "listening", objective_ids: [], instructions: { en: "Listen." }, scoring: "deterministic", stem: "s", options: ["a"], answer_index: 0, explanation: { en: "e" } },
      { type: "mcq", id: "a2", skill: "speaking_production", objective_ids: [], instructions: { en: "Speak." }, scoring: "deterministic", stem: "s", options: ["a"], answer_index: 0, explanation: { en: "e" } },
      { type: "mcq", id: "a3", skill: "grammar", objective_ids: [], instructions: { en: "Grammar." }, scoring: "deterministic", stem: "s", options: ["a"], answer_index: 0, explanation: { en: "e" } },
    ],
  }).unit;

  it("counts only the activities the run answered", () => {
    const progress = skillProgress(unit, new Set(["a2"]));
    expect(progress).toEqual([
      { skill: "listening", answered: 0, total: 1 },
      { skill: "speaking", answered: 1, total: 1 },
      { skill: "reading", answered: 0, total: 0 },
      { skill: "writing", answered: 0, total: 0 },
    ]);
  });

  it("counts nothing before the run answered anything", () => {
    expect(skillProgress(unit, new Set()).every((entry) => entry.answered === 0)).toBe(true);
  });
});

describe("helpStartsOn", () => {
  it("follows the setting, and the level for auto", () => {
    expect(helpStartsOn("on", "C1")).toBe(true);
    expect(helpStartsOn("off", "A1")).toBe(false);
    expect(helpStartsOn("auto", "A1")).toBe(true);
    expect(helpStartsOn("auto", "A2")).toBe(true);
    expect(helpStartsOn("auto", "B1")).toBe(false);
    expect(helpStartsOn(null, "A1")).toBe(false);
  });
});

function draft(patch: Partial<Draft>): Draft {
  return { ...emptyDraft(), ...patch };
}

function withBody(body: ActivityPresentation["body"], activity_type: ActivityPresentation["activity_type"] = "mcq"): ActivityPresentation {
  return presented("a1", body, { activity_type });
}

describe("answerOf", () => {
  it("sends the chosen option of a choice", () => {
    expect(answerOf(withBody({ kind: "mcq", passage: null, audio: null, stem: "s", options: ["a", "b"] }), draft({ choice: 1 })).answer).toEqual({
      kind: "choice",
      index: 1,
    });
  });

  it("waits until a choice is made", () => {
    expect(answerOf(withBody({ kind: "mcq", passage: null, audio: null, stem: "s", options: ["a", "b"] }), emptyDraft()).canSubmit).toBe(false);
  });

  it("sends one answer per gap and refuses a partly filled one", () => {
    const body = withBody({ kind: "gap_fill", text: "a ___ b ___", gaps: 2 });
    expect(answerOf(body, draft({ gaps: ["x", "y"] })).answer).toEqual({ kind: "gaps", answers: ["x", "y"] });
    expect(answerOf(body, draft({ gaps: ["x", " "] })).canSubmit).toBe(false);
  });

  it("sends the tokens in the order the learner placed them", () => {
    const body = withBody({ kind: "reorder", tokens: ["My", "name", "is", "Dewi"] });
    expect(answerOf(body, draft({ placed: [0, 1, 2, 3] })).answer).toEqual({ kind: "order", tokens: ["My", "name", "is", "Dewi"] });
    expect(answerOf(body, draft({ placed: [] })).canSubmit).toBe(false);
  });

  it("sends a pick per item, in the position the display uses", () => {
    const body = withBody({ kind: "match", left: ["hello", "bye"], right: ["halo", "selamat tinggal"] });
    expect(answerOf(body, draft({ picks: [1, 0] })).answer).toEqual({ kind: "picks", picks: [1, 0] });
    expect(answerOf(body, draft({ picks: [1, null] })).canSubmit).toBe(false);
  });

  it("sends the typed answer of a dictation, an error correction and a mediation", () => {
    const text = { kind: "text", text: "I am Rina" } as const;
    expect(answerOf(withBody({ kind: "dictation", audio: [{ speaker: null, text: "I am Rina" }] }), draft({ text: "I am Rina" })).answer).toEqual(text);
    expect(answerOf(withBody({ kind: "error_correction", sentence: "I Rina" }), draft({ text: "I am Rina" })).answer).toEqual(text);
    expect(answerOf(withBody({ kind: "mediation", source_text: "s", task: { en: "t" } }), draft({ text: "I am Rina" })).answer).toEqual(text);
  });

  it("sends the written production of a guided writing task", () => {
    const body = withBody({ kind: "production", prompt: { en: "Introduce yourself." }, content_points: ["name"], min_words: 8, max_words: 40 }, "guided_writing");
    expect(answerOf(body, draft({ text: "My name is Dewi" })).answer).toEqual({ kind: "text", text: "My name is Dewi" });
  });

  it("offers no control for a spoken production, a read-aloud, a shadowing run or a spoken pair", () => {
    const spoken = withBody({ kind: "production", prompt: { en: "Speak." }, content_points: [], min_words: 5, max_words: 20 }, "guided_speaking");
    expect(answerOf(spoken, draft({ text: "hello" })).noControl).toBe(true);
    expect(answerOf(withBody({ kind: "read_aloud", text: "Thank you", focus_phonemes: ["TH"] }), emptyDraft()).noControl).toBe(true);
    expect(answerOf(withBody({ kind: "shadowing", title: "d", lines: [] }), emptyDraft()).noControl).toBe(true);
    expect(answerOf(withBody({ kind: "minimal_pairs_say", pairs: [{ a: "think", b: "sink", focus: "TH" }] }), emptyDraft()).noControl).toBe(true);
  });

  it("offers no control for the roleplay: it is started and finished by its own panel", () => {
    const roleplay = withBody({ kind: "roleplay", scenario: { en: "s" }, tutor_role: "t", learner_role: "l", goals: [], max_turns: 4 }, "roleplay");
    expect(answerOf(roleplay, emptyDraft()).noControl).toBe(true);
  });
});
