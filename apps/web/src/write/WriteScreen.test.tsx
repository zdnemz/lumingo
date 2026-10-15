import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type ApiClient } from "@/api/client";
import type { ActiveSessionView } from "@/generated/ActiveSessionView";
import type { DraftAccepted } from "@/generated/DraftAccepted";
import type { DraftFeedbackView } from "@/generated/DraftFeedbackView";
import type { SessionEnded } from "@/generated/SessionEnded";
import { connect, fakeApi, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { WriteScreen } from "./WriteScreen";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

const PROMPT = "Write about your morning.";

const SESSION: ActiveSessionView = {
  id: 42,
  kind: "writing",
  unit_id: null,
  channel: "text",
  life: "active",
  turn_state: null,
  fault: null,
  turns_completed: 0,
  turn: 0,
  recent: [],
  pending_reply: null,
  speaking: false,
  activity_id: null,
};

const ENDED: SessionEnded = { id: 42, kind: "writing", status: "completed", feedback: null };

function accepted(patch: Partial<DraftAccepted> = {}): DraftAccepted {
  return {
    session_id: 42,
    turn_seq: 1,
    words: 6,
    rule_checked: true,
    rule_findings: [],
    below_minimum: false,
    ...patch,
  };
}

/** Offline: the analysis failed, the draft is queued, and nothing else is in the event. */
function pending(turn_seq: number): DraftFeedbackView {
  return { turn_seq, status: "pending", analysis: null, comparison: null, rubric: null };
}

/** With a provider: the errors, the bands and the comparison against the draft before. */
const ANALYSED: DraftFeedbackView = {
  turn_seq: 1,
  status: "analysed",
  analysis: {
    turn_seq: 1,
    errors: [
      { category: "verb_tense", quote: "I go", correction: "I went", severity: "major", addressed_in_reply: false },
    ],
    objective_evidence: [],
    note: "",
    dropped: 0,
  },
  comparison: {
    earlier: [{ error: { quote: "I go", category: "verb_tense" }, resolution: "fixed" }],
    new: [{ quote: "to the market", category: "preposition" }],
  },
  rubric: {
    dimensions: [
      {
        dimension: "task_achievement",
        band: 3,
        reason: "The draft answers the task.",
        evidence_quotes: ["Yesterday I go to the market."],
        status: "scored",
        capped: false,
      },
      {
        dimension: "range",
        band: null,
        reason: "The evidence did not hold twice.",
        evidence_quotes: [],
        status: "needs_review",
        capped: false,
      },
    ],
    content_points: [],
    on_task: true,
    feedback_en: "You covered the task; watch the past tense.",
    feedback_l1: "Kamu sudah menjawab tugasnya; perhatikan bentuk lampau.",
    confidence: 0.5,
    runs: 1,
  },
};

function routes(overrides: Partial<ApiClient> = {}): Partial<ApiClient> {
  return {
    startSession: () => Promise.resolve(SESSION),
    submitDraft: () => Promise.resolve(accepted()),
    stopSession: () => Promise.resolve(ENDED),
    ...overrides,
  };
}

async function open(overrides: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, routes(overrides));
  const view = renderScreen(<WriteScreen />, api);
  await connect(api);
  return { api, view };
}

/** Sets the task and starts the session, leaving the draft editor on screen. */
async function begin(user: ReturnType<typeof userEvent.setup>, prompt = PROMPT) {
  await user.type(await screen.findByLabelText("Your task"), prompt);
  await user.click(screen.getByRole("button", { name: "Start writing" }));
  return screen.findByLabelText("Your draft");
}

describe("starting the session", () => {
  it("sends a typed prompt with the level the learner picked and no unit, activity or mode", async () => {
    const user = userEvent.setup();
    const startSession = vi.fn(() => Promise.resolve(SESSION));
    await open({ startSession });

    // Nothing to write about yet: the button is blocked, so the server's 400 never happens.
    const start = screen.getByRole("button", { name: "Start writing" });
    expect(start).toBeDisabled();
    await user.click(start);
    expect(startSession).not.toHaveBeenCalled();

    await user.type(await screen.findByLabelText("Your task"), PROMPT);
    await user.click(screen.getByRole("radio", { name: "B1" }));
    await user.click(screen.getByRole("button", { name: "Start writing" }));

    expect(startSession).toHaveBeenCalledWith({
      kind: "writing",
      unit_id: null,
      activity_id: null,
      mode: null,
      topic: { kind: "typed", text: PROMPT },
      level: "B1",
      speak: null,
    });
    expect(await screen.findByLabelText("Your draft")).toBeInTheDocument();
  });
});

describe("layer one", () => {
  it("renders every finding of the rule checker verbatim", async () => {
    const user = userEvent.setup();
    const findings = [
      "Possible spelling mistake: “recieve” should be “receive”.",
      "“Yesterdey” is not a word in the dictionary.",
    ];
    const submitDraft = vi.fn(() => Promise.resolve(accepted({ words: 7, rule_findings: findings })));
    await open({ submitDraft });
    const editor = await begin(user);

    await user.type(editor, "Yesterday I go to the market.");
    await user.click(screen.getByRole("button", { name: "Check this draft" }));

    expect(submitDraft).toHaveBeenCalledWith(42, "Yesterday I go to the market.");
    for (const finding of findings) {
      expect(await screen.findByText(finding)).toBeInTheDocument();
    }
    // The count is the server's, and the draft it belongs to is shown as it was handed in.
    expect(screen.getByText("Words, as the program counted them: 7")).toBeInTheDocument();
    expect(screen.getByText("Draft 1")).toBeInTheDocument();
  });

  it("says not checked when no rule checker ran, and never says clean", async () => {
    const user = userEvent.setup();
    await open({ submitDraft: () => Promise.resolve(accepted({ rule_checked: false, rule_findings: [] })) });
    const editor = await begin(user);

    await user.type(editor, "It was good.");
    await user.click(screen.getByRole("button", { name: "Check this draft" }));

    expect(
      await screen.findByText(
        "Not checked: this program has no rule checker, so nothing about this draft was checked.",
      ),
    ).toBeInTheDocument();
    expect(document.body.textContent ?? "").not.toMatch(/clean/i);
    expect(screen.queryByText("The rule checker found nothing to mark.")).toBeNull();
  });
});

describe("layers two and three", () => {
  it("shows the waiting line and says the draft is stored when the feedback is pending", async () => {
    const user = userEvent.setup();
    // The event is detached from the HTTP answer: it arrives before the answer does.
    let release: ((value: DraftAccepted) => void) | null = null;
    const submitDraft = vi.fn(
      () =>
        new Promise<DraftAccepted>((resolve) => {
          release = resolve;
        }),
    );
    const { api } = await open({ submitDraft });
    const editor = await begin(user);

    await user.type(editor, "It was good.");
    await user.click(screen.getByRole("button", { name: "Check this draft" }));

    act(() => {
      api.emit({
        type: "FeedbackReady",
        seq: 3,
        session_id: 42,
        feedback: { kind: "draft", feedback: pending(1) },
      });
    });
    await act(async () => {
      release?.(accepted({ words: 3 }));
    });

    expect(await screen.findByText("Waiting for an AI provider")).toBeInTheDocument();
    expect(
      screen.getByText("The draft is stored. Its errors and bands are added when an AI provider is available."),
    ).toBeInTheDocument();
    expect(screen.getByText("Words, as the program counted them: 3")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Bands" })).toBeNull();
  });

  it("renders the errors, the bands with a needs-review dimension, and the comparison", async () => {
    const user = userEvent.setup();
    const { api } = await open();
    const editor = await begin(user);

    await user.type(editor, "Yesterday I go to the market.");
    await user.click(screen.getByRole("button", { name: "Check this draft" }));
    act(() => {
      api.emit({
        type: "FeedbackReady",
        seq: 4,
        session_id: 42,
        feedback: { kind: "draft", feedback: ANALYSED },
      });
    });

    // The error is marked on the learner's own words, with the server's category and correction.
    const marked = await screen.findByLabelText("Marked text");
    expect(marked).toHaveTextContent("Yesterday I go to the market.");
    const mark = screen.getByRole("button", { name: "Mistake 1 of 1: verb_tense: I go" });
    await user.click(mark);
    expect(screen.getByText("Better: I went")).toBeInTheDocument();

    // The bands: one scored, one the program will not score, each with its own reason.
    expect(await screen.findByText("Band 3 of 4")).toBeInTheDocument();
    expect(screen.getByText("Needs review")).toBeInTheDocument();
    expect(screen.getByText("task_achievement")).toBeInTheDocument();
    expect(screen.getByText("range")).toBeInTheDocument();
    expect(screen.getByText("The draft answers the task.")).toBeInTheDocument();
    expect(screen.getByText("The evidence did not hold twice.")).toBeInTheDocument();
    expect(screen.getByText("You covered the task; watch the past tense.")).toBeInTheDocument();
    const quotes = screen.getByRole("list", { name: "Quotes from your draft" });
    expect(within(quotes).getByText("Yesterday I go to the market.")).toBeInTheDocument();

    // The comparison against the earlier draft, only because the server sent one.
    expect(screen.getByText("Fixed: 1")).toBeInTheDocument();
    expect(screen.getByText("Still there: 0")).toBeInTheDocument();
    expect(screen.getByText("New: 1")).toBeInTheDocument();
  });
});

describe("the editor", () => {
  it("blocks an empty draft and keeps the text until the server takes it", async () => {
    const user = userEvent.setup();
    const submitDraft = vi.fn(() => Promise.resolve(accepted({ words: 6 })));
    await open({ submitDraft });
    const editor = await begin(user);

    const check = screen.getByRole("button", { name: "Check this draft" });
    expect(check).toBeDisabled();
    await user.click(check);
    expect(submitDraft).not.toHaveBeenCalled();

    await user.type(editor, "Yesterday I go to the market.");
    await user.click(screen.getByRole("button", { name: "Check this draft" }));
    expect(await screen.findByText("Words, as the program counted them: 6")).toBeInTheDocument();
    expect(editor).toHaveValue("");
  });

  it("says plainly that a finished writing session came with no summary", async () => {
    const user = userEvent.setup();
    const stopSession = vi.fn(() => Promise.resolve(ENDED));
    await open({ stopSession });
    await begin(user);

    await user.click(screen.getByRole("button", { name: "Finish the session" }));

    expect(stopSession).toHaveBeenCalledWith(42);
    expect(await screen.findByRole("heading", { name: "Session finished" })).toBeInTheDocument();
    expect(
      screen.getByText(
        "The program sent no summary with the end of a writing session. What it kept stays in the stored session row, which the progress page lists.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Your draft")).toBeNull();
  });
});
