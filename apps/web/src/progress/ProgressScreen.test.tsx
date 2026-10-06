import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type ApiClient } from "@/api/client";
import type { AttemptEvidence } from "@/generated/AttemptEvidence";
import type { GameState } from "@/generated/GameState";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import { fakeApi, renderApp, resetBrowserState, setSystem, visibleStrings } from "@/test/utils";
import { EMPTY_PROGRESS, EVIDENCE, FREE_KINDS, FULL_PROGRESS, GAME, UNITS, estimateOf } from "./fixtures";
import { ProgressScreen } from "./ProgressScreen";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

function routes(overrides: Partial<ApiClient> = {}): Partial<ApiClient> {
  return {
    getProgress: () => Promise.resolve(FULL_PROGRESS),
    listUnits: () => Promise.resolve(UNITS),
    getGame: () => Promise.resolve(GAME),
    getAttemptEvidence: () => Promise.resolve(EVIDENCE),
    ...overrides,
  };
}

async function open(overrides: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, routes(overrides));
  const view = renderApp(<ProgressScreen />, api);
  return { api, view };
}

function profileRow(name: string): HTMLElement {
  const list = screen.getByRole("list", { name: "Your four skills" });
  const row = within(list)
    .getAllByRole("listitem")
    .find((item) => within(item).queryByRole("heading", { name }) !== null);
  if (!row) throw new Error(`no profile row for ${name}`);
  return row;
}

/** A CEFR level written as a word on its own: A1, B2, pre-A1. */
const LEVEL = /\b(pre-)?[ABC][12]\b/;

describe("loading", () => {
  it("says it is loading and shows no level while nothing has arrived", async () => {
    const never = new Promise<never>(() => undefined);
    await open({ getProgress: () => never, getGame: () => never });
    expect(screen.getAllByRole("status").map((node) => node.textContent)).toEqual(["Loading...", "Loading..."]);
    expect(screen.queryByRole("list", { name: "Your four skills" })).toBeNull();
    expect(document.body.textContent).not.toMatch(LEVEL);
  });
});

describe("error", () => {
  it("shows the not-answering banner for the assessment and keeps the game on its own", async () => {
    const getProgress = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValueOnce(EMPTY_PROGRESS);
    await open({ getProgress });
    await screen.findByText("The program is not answering");
    expect(screen.getByRole("heading", { name: "The game, just for fun" })).toBeInTheDocument();
    await screen.findByText("Sparks in total");
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByRole("list", { name: "Your four skills" });
    expect(getProgress).toHaveBeenCalledTimes(2);
  });

  it("keeps the assessment when only the game fails", async () => {
    await open({ getGame: () => Promise.reject(new ApiError(500, "internal")) });
    await screen.findByRole("list", { name: "Your four skills" });
    const game = (await screen.findByRole("heading", { name: "The game, just for fun" })).closest("section") as HTMLElement;
    await within(game).findByText("Something went wrong inside the program. Try again. If it repeats, look in the log folder.");
    expect(screen.getByText("Speaking: A2 (estimate, medium confidence), based on 14 scored tasks")).toBeInTheDocument();
  });

  it("explains a refused page", async () => {
    await open({ getProgress: () => Promise.reject(new ApiError(403, "forbidden")) });
    await screen.findByText("The program refused this page");
  });

  it("still shows the units by id when the unit list fails", async () => {
    await open({ listUnits: () => Promise.reject(new TypeError("network")) });
    const units = (await screen.findByRole("heading", { name: "Units" })).closest("section") as HTMLElement;
    expect(within(units).getByText("a1-u01")).toBeInTheDocument();
  });
});

describe("empty", () => {
  it("shows every skill as not enough evidence, with no level, and an honest empty state everywhere", async () => {
    await open({ getProgress: () => Promise.resolve(EMPTY_PROGRESS), getGame: () => Promise.resolve({ ...GAME, xp_total: 0, xp_by_source: [], cosmetics: [] }) });
    await screen.findByText("Nothing is recorded yet");
    for (const skill of ["listening", "speaking", "reading", "writing"]) {
      const row = profileRow(skill[0]?.toUpperCase() + skill.slice(1));
      expect(within(row).getByText(`Not enough evidence yet for ${skill}. More scored ${skill} tasks will help.`)).toBeInTheDocument();
      expect(within(row).getByText("No scored tasks yet.")).toBeInTheDocument();
      expect(row.textContent).not.toMatch(LEVEL);
    }
    expect(screen.getByText("No mistakes are recorded yet.")).toBeInTheDocument();
    expect(screen.getByText("Nothing is due for review right now.")).toBeInTheDocument();
    expect(screen.getByText("No unit progress is recorded yet.")).toBeInTheDocument();
    expect(screen.getByText("No practice sessions yet.")).toBeInTheDocument();
    expect(screen.getByText(/does not list the attempts|keeps only the newest estimate/)).toBeInTheDocument();
    expect(screen.getByText("No sparks yet. Practising earns the first ones.")).toBeInTheDocument();
    expect(screen.getByText("No unlockables are listed.")).toBeInTheDocument();
  });
});

describe("success: the four-skill profile", () => {
  it("writes each estimate in the allowed form, with confidence and evidence count", async () => {
    await open();
    await screen.findByText("Speaking: A2 (estimate, medium confidence), based on 14 scored tasks");
    expect(within(profileRow("Reading")).getByText("Reading: B1 (estimate, high confidence), based on 20 scored tasks")).toBeInTheDocument();
  });

  it("calls a placement-only level a suggested starting level, not a result", async () => {
    await open();
    const row = await waitFor(() => profileRow("Listening"));
    expect(within(row).getByText("Suggested starting level: A1 (estimate from the placement test only, low confidence), based on 4 scored tasks.")).toBeInTheDocument();
    expect(row.textContent).not.toMatch(/your level is/i);
  });

  it("shows no level for a skill whose status is insufficient evidence, even when the server sent one", async () => {
    await open();
    const row = await waitFor(() => profileRow("Writing"));
    expect(within(row).getByText("Not enough evidence yet for writing. More scored writing tasks will help.")).toBeInTheDocument();
    expect(within(row).getByText("2 scored tasks so far.")).toBeInTheDocument();
    expect(row.textContent).not.toMatch(LEVEL);
    expect(row.querySelector(".ladder")).toBeNull();
  });

  it("shows low confidence in words", async () => {
    await open({ getProgress: () => Promise.resolve({ ...EMPTY_PROGRESS, estimates: [estimateOf({ skill: "speaking", level: "B1", confidence: 0.45, evidence_count: 9 })] }) });
    await screen.findByText("Speaking: B1 (estimate, low confidence), based on 9 scored tasks");
  });

  it("says one scored task in the singular", async () => {
    await open({ getProgress: () => Promise.resolve({ ...EMPTY_PROGRESS, estimates: [estimateOf({ skill: "reading", level: "A1", confidence: 0.61, evidence_count: 1 })] }) });
    await screen.findByText("Reading: A1 (estimate, medium confidence), based on 1 scored task");
  });

  it("reads pre-A1 as working towards A1", async () => {
    await open({ getProgress: () => Promise.resolve({ ...EMPTY_PROGRESS, estimates: [estimateOf({ skill: "listening", level: "pre-A1", confidence: 0.4, evidence_count: 9 })] }) });
    await screen.findByText("Working towards A1 (estimate, low confidence), based on 9 scored tasks.");
  });

  it("withholds a level that came without its confidence", async () => {
    await open({ getProgress: () => Promise.resolve({ ...EMPTY_PROGRESS, estimates: [estimateOf({ skill: "speaking", level: "B1", confidence: null })] }) });
    const row = await waitFor(() => profileRow("Speaking"));
    expect(within(row).getByText(/A level for speaking is not shown/)).toBeInTheDocument();
    expect(row.textContent).not.toMatch(LEVEL);
  });

  it("never shows a level outside a sentence that calls it an estimate, and no percentage of a level", async () => {
    await open();
    await screen.findByText("Speaking: A2 (estimate, medium confidence), based on 14 scored tasks");
    const list = screen.getByRole("list", { name: "Your four skills" });
    for (const row of within(list).getAllByRole("listitem")) {
      const sentence = row.querySelector(".profile__sentence")?.textContent ?? "";
      if (LEVEL.test(sentence)) expect(sentence).toMatch(/\(estimate|Suggested starting level/);
      expect(row.querySelector('[role="progressbar"]')).toBeNull();
    }
    // Each row says "Estimate" and there is no single overall level.
    expect(within(list).getAllByText("Estimate")).toHaveLength(4);
    expect(screen.queryByText(/overall|working level/i)).toBeNull();
  });

  it("says nobody has checked the estimates, and uses no banned wording", async () => {
    await open();
    await screen.findByText("Estimated level, based on your recorded work.");
    expect(screen.getByText(/No person has checked these estimates/)).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/certif|official|equivalen|\bexam|ielts|toefl|your level is/i);
  });

  it("speaks Indonesian", async () => {
    setSystem({ reduced: true, language: "id-ID" });
    await open();
    await screen.findByText("Berbicara: A2 (perkiraan, keyakinan sedang), berdasarkan 14 tugas yang dinilai");
    expect(screen.getByText("Perkiraan level, berdasarkan hasil kerja yang tercatat.")).toBeInTheDocument();
    expect(screen.getByText(/Level awal yang disarankan: A1/)).toBeInTheDocument();
    expect(screen.getByText("Belum cukup bukti untuk menulis. Tugas menulis yang dinilai akan membantu.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Permainan, hanya untuk seru-seruan" })).toBeInTheDocument();
    // The unit title follows the language too.
    expect(screen.getAllByText("Halo, aku Sari").length).toBeGreaterThan(0);
  });
});

describe("pronunciation, history, errors, reviews, units", () => {
  it("shows pronunciation as experimental, with no level, apart from the four skills", async () => {
    await open();
    const panel = (await screen.findByRole("heading", { name: "Pronunciation" })).closest("section") as HTMLElement;
    expect(within(panel).getByText("Experimental")).toBeInTheDocument();
    expect(within(panel).getByText("Pronunciation feedback is experimental")).toBeInTheDocument();
    expect(within(panel).getByText("Pronunciation items in the review queue: 1")).toBeInTheDocument();
    expect(panel.textContent).not.toMatch(LEVEL);
    expect(screen.getByRole("list", { name: "Your four skills" }).textContent).not.toMatch(/pronunciation/i);
  });

  it("says the history is not available and lists what is real", async () => {
    await open();
    const panel = (await screen.findByRole("heading", { name: "Estimate history" })).closest("section") as HTMLElement;
    expect(within(panel).getByText(/keeps only the newest estimate of each skill/)).toBeInTheDocument();
    expect(within(panel).getByText(/Speaking: last worked out .*, rule est\/1/)).toBeInTheDocument();
  });

  it("lists error patterns by count, then name", async () => {
    await open();
    const panel = (await screen.findByRole("heading", { name: "Error patterns" })).closest("section") as HTMLElement;
    const names = within(panel)
      .getAllByRole("rowheader")
      .map((cell) => cell.textContent);
    expect(names).toEqual(["grammar.tense", "grammar.article", "spelling"]);
    expect(within(panel).getByText(/not part of any estimate/)).toBeInTheDocument();
  });

  it("shows the review queue with unit titles, an experimental pronunciation kind, and the cut-off note", async () => {
    await open();
    const panel = (await screen.findByRole("heading", { name: "Review queue" })).closest("section") as HTMLElement;
    expect(within(panel).getByText("Hello, I am Sari")).toBeInTheDocument();
    expect(within(panel).getByText("a1-u01/v3")).toBeInTheDocument();
    expect(within(panel).getByText("Vocabulary")).toBeInTheDocument();
    expect(within(panel).getByText("Pronunciation (experimental)")).toBeInTheDocument();
    expect(within(panel).getByText("2.5")).toBeInTheDocument();
    expect(within(panel).getByText("More than 50 items are due. The 50 most overdue are shown.")).toBeInTheDocument();
  });

  it("shows units with their status and best checkpoint result", async () => {
    await open();
    const panel = (await screen.findByRole("heading", { name: "Units" })).closest("section") as HTMLElement;
    expect(within(panel).getByText("Best checkpoint result: 82 percent")).toBeInTheDocument();
    expect(within(panel).getByText("No checkpoint taken yet")).toBeInTheDocument();
    expect(within(panel).getByText("Passed")).toBeInTheDocument();
    expect(within(panel).getByText("In progress")).toBeInTheDocument();
  });
});

describe("free-mode and generated work is never evidence of level", () => {
  it("marks every free mode session as never counting, and only authored kinds as able to count", async () => {
    await open();
    const panel = (await screen.findByRole("heading", { name: "Recent practice" })).closest("section") as HTMLElement;
    const rows = within(panel).getAllByRole("listitem");
    expect(rows).toHaveLength(FULL_PROGRESS.recent_sessions.length);
    const byKind = (text: RegExp) => rows.find((row) => text.test(row.textContent ?? ""));
    for (const label of [/Conversation/, /Text chat/, /Writing workshop/, /Graded reading/]) {
      const row = byKind(label);
      expect(row, String(label)).toBeDefined();
      expect(row?.textContent).toContain("Free practice: never counts toward a level");
      expect(row?.textContent).not.toContain("can add evidence");
    }
    expect(FREE_KINDS).toHaveLength(4);
    expect(byKind(/Drill/)?.textContent).toContain("Practice: not used for a level");
    expect(byKind(/Lesson/)?.textContent).toContain("Authored work: can add evidence to an estimate");
    expect(byKind(/Checkpoint/)?.textContent).toContain("Authored work: can add evidence to an estimate");
  });

  it("puts no session, chat or generated text inside the profile", async () => {
    await open();
    await screen.findByText("Speaking: A2 (estimate, medium confidence), based on 14 scored tasks");
    const list = screen.getByRole("list", { name: "Your four skills" });
    expect(list.textContent).not.toMatch(/chat|conversation|workshop|graded|generated|drill/i);
  });

  it("shows an attempt's evidence without any level, score or percentage, whatever session it came from", async () => {
    const user = userEvent.setup();
    await open();
    await user.click(await screen.findByRole("button", { name: /See the work behind this estimate \(Writing\)/ }));
    await user.type(screen.getByLabelText("Attempt number"), "12");
    await user.click(screen.getByRole("button", { name: "Show the evidence" }));
    const evidence = (await screen.findByRole("heading", { name: "Evidence for attempt 12" })).closest("div") as HTMLElement;
    await within(evidence).findByText("The sentence is complete and correct.");
    expect(evidence.textContent).not.toMatch(LEVEL);
    expect(evidence.textContent).not.toMatch(/\d\s?%|percent|score of|\bscored\b/i);
    expect(within(evidence).getByText(/does not show a level/)).toBeInTheDocument();
  });
});

describe("drill-down from a skill to its evidence", () => {
  it("opens from the keyboard, names the skill, and says the attempt list is not available", async () => {
    const user = userEvent.setup();
    await open();
    await screen.findByRole("list", { name: "Your four skills" });
    const trigger = screen.getByRole("button", { name: /See the work behind this estimate \(Speaking\)/ });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    trigger.focus();
    await user.keyboard("{Enter}");
    const heading = await screen.findByRole("heading", { name: "The work behind Speaking" });
    expect(heading).toHaveFocus();
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    const panel = heading.closest("section") as HTMLElement;
    expect(within(panel).getByText("Estimation rule version").nextElementSibling).toHaveTextContent("est/1");
    expect(within(panel).getByText("Scored tasks counted").nextElementSibling).toHaveTextContent("14");
    expect(within(panel).getByText("Confidence").nextElementSibling).toHaveTextContent("medium");
    expect(within(panel).getByText(/does not list the attempts of a skill yet/)).toBeInTheDocument();
    expect(within(panel).getByText(/Only authored lesson, checkpoint and placement tasks count toward an estimate/)).toBeInTheDocument();
    await user.click(within(panel).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("heading", { name: "The work behind Speaking" })).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it("describes a skill without a level as not enough evidence and shows no confidence", async () => {
    const user = userEvent.setup();
    await open();
    await user.click(await screen.findByRole("button", { name: /See the work behind this estimate \(Writing\)/ }));
    const panel = (await screen.findByRole("heading", { name: "The work behind Writing" })).closest("section") as HTMLElement;
    expect(within(panel).getByText("Status").nextElementSibling).toHaveTextContent("Not enough evidence yet");
    expect(within(panel).getByText("Confidence").nextElementSibling).toHaveTextContent("None");
    expect(panel.textContent).not.toMatch(LEVEL);
  });

  it("refuses something that is not an attempt number and makes no request", async () => {
    const user = userEvent.setup();
    const getAttemptEvidence = vi.fn(() => Promise.resolve(EVIDENCE));
    await open({ getAttemptEvidence });
    await user.click(await screen.findByRole("button", { name: /See the work behind this estimate \(Reading\)/ }));
    for (const text of ["", "abc", "0", "-3", "1.5", "12 34"]) {
      const field = screen.getByLabelText("Attempt number");
      await user.clear(field);
      if (text) await user.type(field, text);
      await user.click(screen.getByRole("button", { name: "Show the evidence" }));
      expect(screen.getByText("Enter a whole number above zero.")).toBeInTheDocument();
    }
    expect(getAttemptEvidence).not.toHaveBeenCalled();
  });

  it("shows the answer, quotes, reasons and the scorer and engine versions as text", async () => {
    const user = userEvent.setup();
    const getAttemptEvidence = vi.fn(() => Promise.resolve(EVIDENCE));
    await open({ getAttemptEvidence });
    await user.click(await screen.findByRole("button", { name: /See the work behind this estimate \(Writing\)/ }));
    await user.type(screen.getByLabelText("Attempt number"), " 12 ");
    await user.click(screen.getByRole("button", { name: "Show the evidence" }));
    const panel = (await screen.findByRole("heading", { name: "Evidence for attempt 12" })).closest("div") as HTMLElement;
    await within(panel).findByText("The sentence is complete and correct.");
    expect(getAttemptEvidence).toHaveBeenCalledWith(12, expect.any(AbortSignal));
    // The learner's text is a text node: the tag is visible as characters, not an element.
    expect(within(panel).getByText(/I like <b>tea<\/b>\./)).toBeInTheDocument();
    expect(panel.querySelector("b")).toBeNull();
    expect(within(panel).getByText("I am from Jakarta")).toBeInTheDocument();
    expect(within(panel).getByText("Rubric, applied by the AI provider you chose")).toBeInTheDocument();
    expect(within(panel).getByText("writing-a1/1+t3/2+model-x")).toBeInTheDocument();
    expect(within(panel).getByText("norm/1")).toBeInTheDocument();
    expect(within(panel).getByText("rubric/1")).toBeInTheDocument();
    expect(within(panel).getByText("evidence/1")).toBeInTheDocument();
    expect(within(panel).getByText("stt: whisper-small 1.2")).toBeInTheDocument();
    expect(within(panel).getByText("writing-a1/1")).toBeInTheDocument();
    expect(within(panel).getByText("a1-u01/guided-writing-1")).toBeInTheDocument();
    for (const heading of ["What was answered", "Quotes from the answer", "Reasons from the scorer", "How it was scored"]) {
      expect(within(panel).getByRole("heading", { name: heading })).toBeInTheDocument();
    }
  });

  it("marks a pronunciation row experimental and shows its reference text as such", async () => {
    const user = userEvent.setup();
    const pron: AttemptEvidence = {
      attempt_id: 5,
      activity_id: "a1-u02/read-aloud-1",
      skill: "pronunciation",
      dimension: "pronunciation",
      evidence: [
        { id: 1, kind: "metric", content: null, data: { scorer: "pron_engine", scorer_version: "pron/0.1", algorithm_version: "pron-gop/1", engines: [], details: { experimental: true } }, created_at: "2026-10-06T09:00:00Z" },
        { id: 2, kind: "response_text", content: "Three thin things", data: { kind: "reference_text" }, created_at: "2026-10-06T09:00:00Z" },
      ],
    };
    await open({ getAttemptEvidence: () => Promise.resolve(pron) });
    await user.click(await screen.findByRole("button", { name: /See the work behind this estimate \(Speaking\)/ }));
    await user.type(screen.getByLabelText("Attempt number"), "5");
    await user.click(screen.getByRole("button", { name: "Show the evidence" }));
    const panel = (await screen.findByRole("heading", { name: "Evidence for attempt 5" })).closest("div") as HTMLElement;
    await within(panel).findByText("Pronunciation engine (experimental)");
    expect(within(panel).getByText("Pronunciation feedback is experimental")).toBeInTheDocument();
    expect(within(panel).getByText("The text that was to be read aloud")).toBeInTheDocument();
  });

  it("says when a row carries no version, and when nothing is stored", async () => {
    const user = userEvent.setup();
    const bare: AttemptEvidence = {
      attempt_id: 8,
      activity_id: "a1-u01/mcq-1",
      skill: "reading",
      dimension: "accuracy",
      evidence: [{ id: 1, kind: "metric", content: null, data: { run_bands: [2, 3] }, created_at: "2026-10-06T09:00:00Z" }],
    };
    const none: AttemptEvidence = { ...bare, attempt_id: 9, evidence: [] };
    const getAttemptEvidence = vi.fn((id: number) => Promise.resolve(id === 8 ? bare : none));
    await open({ getAttemptEvidence });
    await user.click(await screen.findByRole("button", { name: /See the work behind this estimate \(Reading\)/ }));
    await user.type(screen.getByLabelText("Attempt number"), "8");
    await user.click(screen.getByRole("button", { name: "Show the evidence" }));
    await screen.findByText("No scorer or engine version is stored with this row.");
    await user.clear(screen.getByLabelText("Attempt number"));
    await user.type(screen.getByLabelText("Attempt number"), "9");
    await user.click(screen.getByRole("button", { name: "Show the evidence" }));
    await screen.findByText(/No evidence is stored for this attempt/);
  });

  it("says there is no such attempt on not_found, and offers retry on any other failure", async () => {
    const user = userEvent.setup();
    const getAttemptEvidence = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(404, "not_found"))
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(EVIDENCE);
    await open({ getAttemptEvidence });
    await user.click(await screen.findByRole("button", { name: /See the work behind this estimate \(Writing\)/ }));
    await user.type(screen.getByLabelText("Attempt number"), "99");
    await user.click(screen.getByRole("button", { name: "Show the evidence" }));
    await screen.findByText("There is no attempt with that number.");
    await user.clear(screen.getByLabelText("Attempt number"));
    await user.type(screen.getByLabelText("Attempt number"), "98");
    await user.click(screen.getByRole("button", { name: "Show the evidence" }));
    const panel = (await screen.findByRole("heading", { name: "Evidence for attempt 98" })).closest("div") as HTMLElement;
    await within(panel).findByText("The program is not answering");
    await user.click(within(panel).getByRole("button", { name: "Try again" }));
    await within(panel).findByText("The sentence is complete and correct.");
  });
});

describe("game panel", () => {
  it("shows sparks, rank, streak, rest tokens and unlockables", async () => {
    await open();
    const game = (await screen.findByRole("heading", { name: "The game, just for fun" })).closest("section") as HTMLElement;
    await within(game).findByText("340");
    expect(within(game).getByText("Glow rank 3, Glow")).toBeInTheDocument();
    expect(within(game).getByRole("progressbar", { name: "Progress to the next rank" })).toHaveAttribute("aria-valuenow", "40");
    expect(within(game).getByText("Next rank at 500 sparks")).toBeInTheDocument();
    expect(within(game).getByText("Day streak: 4")).toBeInTheDocument();
    expect(within(game).getByText("Longest streak: 9")).toBeInTheDocument();
    expect(within(game).getByText("Practised today")).toBeInTheDocument();
    expect(within(game).getByText("Rest-day tokens: 0")).toBeInTheDocument();
    expect(within(game).getByText("Lesson")).toBeInTheDocument();
    expect(within(game).getByText("Red cap")).toBeInTheDocument();
    expect(within(game).getByText("In use now")).toBeInTheDocument();
    expect(within(game).getByText("Forest theme")).toBeInTheDocument();
    expect(within(game).getByText("Unlocked")).toBeInTheDocument();
    expect(within(game).getByText(/Unlocks at Glow rank 5/)).toBeInTheDocument();
    // An id the dictionaries do not know yet is shown as it is.
    expect(within(game).getByText("accessory-new-thing")).toBeInTheDocument();
  });

  it("says in words that it is cosmetic and is not a level, and shows no level of its own", async () => {
    await open();
    const game = (await screen.findByRole("heading", { name: "The game, just for fun" })).closest("section") as HTMLElement;
    await within(game).findByText("Sparks and streaks are only for fun. They are not a level estimate.");
    expect(within(game).getByText(/Nothing here is read from your attempts, and nothing here changes your estimates/)).toBeInTheDocument();
    expect(game.textContent).not.toMatch(LEVEL);
    expect(game.textContent).not.toMatch(/\bCEFR\b/);
  });

  it("is a separate region from the assessment, not inside it", async () => {
    await open();
    const game = (await screen.findByRole("heading", { name: "The game, just for fun" })).closest("section") as HTMLElement;
    const assessment = (await screen.findByRole("heading", { name: "What your recorded work shows" })).closest("section") as HTMLElement;
    expect(assessment.contains(game)).toBe(false);
    expect(game.contains(assessment)).toBe(false);
    expect(game.dataset.kind).toBe("game");
    // The assessment has no game vocabulary.
    await within(assessment).findByRole("list", { name: "Your four skills" });
    expect(assessment.textContent).not.toMatch(/sparks|streak|glow|wardrobe|unlockable/i);
  });

  it("shows rest tokens that practice would use, and the top rank", async () => {
    const game: GameState = {
      ...GAME,
      rank: { rank: 6, progress_percent: 100, next_threshold: null },
      streak: { ...GAME.streak, active_today: false, rest_tokens_available: 2, tokens_needed_to_continue: 1 },
    };
    await open({ getGame: () => Promise.resolve(game) });
    await screen.findByText("Top rank reached");
    expect(screen.getByText("Rest-day tokens: 2")).toBeInTheDocument();
    expect(screen.getByText("Rest-day tokens that practice today would use: 1")).toBeInTheDocument();
    expect(screen.getByText("Not practised yet today")).toBeInTheDocument();
  });
});

describe("language", () => {
  it("has no English left on the screen when the language is Indonesian", async () => {
    setSystem({ reduced: true, language: "id-ID" });
    const { view } = await open();
    await screen.findByText(/Berbicara: A2/);
    await screen.findByText("Total percik");
    const english = visibleStrings(view.container).filter((text) =>
      /\b(Not enough evidence|scored tasks|Review queue|Error patterns|Recent practice|Sparks|streak|Unlockables|Estimate history|The work behind)\b/.test(text),
    );
    expect(english).toEqual([]);
  });
});

describe("what the screen asks the server for", () => {
  it("uses only the typed client routes it should, and none of the free-mode data", async () => {
    const api = fakeApi(undefined, routes());
    const getProgress = vi.spyOn(api, "getProgress");
    const getGame = vi.spyOn(api, "getGame");
    const listUnits = vi.spyOn(api, "listUnits");
    renderApp(<ProgressScreen />, api);
    await screen.findByText("Sparks in total");
    expect(getProgress).toHaveBeenCalledTimes(1);
    expect(getGame).toHaveBeenCalledTimes(1);
    expect(listUnits).toHaveBeenCalledTimes(1);
  });
});

describe("shape of the progress fixture", () => {
  it("covers each free mode", () => {
    const kinds = (FULL_PROGRESS as ProgressOverview).recent_sessions.map((s) => s.kind);
    for (const kind of FREE_KINDS) expect(kinds).toContain(kind);
  });
});
