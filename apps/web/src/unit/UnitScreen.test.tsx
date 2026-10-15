import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type ApiClient } from "@/api/client";
import type { ActivityPresentation } from "@/generated/ActivityPresentation";
import type { ActivityResultView } from "@/generated/ActivityResultView";
import type { ActiveSessionView } from "@/generated/ActiveSessionView";
import type { CheckpointRowView } from "@/generated/CheckpointRowView";
import type { NextActivity } from "@/generated/NextActivity";
import type { SessionEnded } from "@/generated/SessionEnded";
import { fakeApi, nextActivity, presented, progressOf, renderScreen, resetBrowserState, setSystem, unitDetail, unitList, unitSummary } from "@/test/utils";
import { UnitScreen } from "./UnitScreen";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

const SESSION: ActiveSessionView = {
  id: 42,
  kind: "lesson",
  unit_id: "a1-u01",
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

/** A result the walk can put in the frame: the score and the accepted answer are the server's. */
function scored(activity_id: string, score: number | null, expected?: string): ActivityResultView {
  return {
    activity_id,
    activity_type: "mcq",
    score,
    confidence: 1,
    outcome:
      score === null
        ? { kind: "queued" }
        : {
            kind: "deterministic",
            feedback: {
              score,
              correct: score === 1 ? 1 : 0,
              total: 1,
              passed: score >= 0.7,
              items: [
                {
                  index: 0,
                  outcome: score === 1 ? "correct" : "wrong",
                  given: "x",
                  expected: expected ?? "am",
                  explanation: { en: "Because of the verb." },
                },
              ],
              explanation: null,
            },
          },
  };
}

/** The shape the offline server sends for an activity whose scoring is `none`: practice, no score. */
function unscored(activity_id: string, reason: string): ActivityResultView {
  return {
    activity_id,
    activity_type: "roleplay",
    score: null,
    confidence: null,
    outcome: { kind: "unscored", reason },
  };
}

function endedWith(rows: CheckpointRowView[], mean: number, pass_mark: number, answered: number): SessionEnded {
  return {
    id: 42,
    kind: "lesson",
    status: "completed",
    feedback: {
      kind: "unit",
      summary: { unit_id: "a1-u01", rows, mean, pass_mark, passed: mean >= pass_mark, provisional: false, unit_status: "passed", answered },
    },
  };
}

/** A unit with one activity of each body kind the player claims to support. */
const UNIT = unitDetail({
  id: "a1-u01",
  title: { en: "Hello!", id: "Halo!" },
  checkpoint: { pass_score: 0.7, activity_ids: ["a1", "a2"] },
  presentation: [{ id: "p1", kind: "explanation", text: { en: "Greet people by time of day.", id: "Sapa orang sesuai waktu." }, examples: [{ en: "Good morning!", id: "Selamat pagi!" }] }],
  dialogues: [{ id: "d1", title: "New classmates", context: { en: "Two students meet.", id: "Dua siswa bertemu." }, turns: [{ speaker: "Dewi", text: "Good evening!" }] }],
  activities: [
    { type: "mcq", id: "a1", skill: "reading", objective_ids: [], instructions: { en: "Read and choose.", id: "Baca lalu pilih." }, scoring: "deterministic", stem: "Where is Dewi?", options: ["school", "home"], answer_index: 0, explanation: { en: "e" } },
    { type: "gap_fill", id: "a2", skill: "grammar", objective_ids: [], instructions: { en: "Type the missing word.", id: "Ketik kata yang hilang." }, scoring: "deterministic", text: "I ___ Rina.", answers: [["am"]], explanation: { en: "e" } },
    { type: "guided_writing", id: "a3", skill: "writing", objective_ids: [], instructions: { en: "Write two sentences.", id: "Tulis dua kalimat." }, scoring: "rubric", prompt: { en: "Introduce yourself.", id: "Perkenalkan dirimu." }, content_points: ["your name"], rubric_id: "r", model_answers: [], min_words: 8, max_words: 40 },
  ],
});

/** Routes for the player: a lesson session that walks whatever `steps` says. */
function routes(steps: NextActivity[], patch: Partial<ApiClient> = {}): Partial<ApiClient> {
  const queue = [...steps];
  return {
    getUnit: () => Promise.resolve(UNIT),
    startSession: () => Promise.resolve(SESSION),
    nextActivity: () => Promise.resolve(queue.shift() ?? nextActivity({ session_id: 42 })),
    stopSession: () => Promise.resolve(endedWith([{ activity_id: "a1", answered: true, score: 1 }], 1, 0.7, 1)),
    submitActivity: () => Promise.resolve({ result: scored("a1", 1) }),
    ...patch,
  };
}

async function openPlayer(steps: NextActivity[], patch: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, routes(steps, patch));
  renderScreen(<UnitScreen id="a1-u01" />, api);
  return api;
}

function stepOf(activity: ActivityPresentation | null, patch: Partial<NextActivity> = {}): NextActivity {
  return nextActivity({ session_id: 42, activity, total: 3, ...patch });
}

describe("the picker", () => {
  const UNITS = unitList([
    unitSummary({ id: "a1-u01", sequence: 1, title: { en: "Hello", id: "Halo" } }),
    unitSummary({ id: "a1-u02", sequence: 2, title: { en: "Family", id: "Keluarga" }, prerequisites: ["a1-u01"] }),
  ]);

  it("lists the units and links each one with unitHref", async () => {
    const api = fakeApi(undefined, { listUnits: () => Promise.resolve(UNITS), getProgress: () => Promise.resolve(progressOf([])) });
    renderScreen(<UnitScreen id={null} />, api);
    const link = await screen.findByRole("link", { name: "Unit 1: Hello" });
    // next/link adds or drops the trailing slash from the build setting, so accept both.
    expect(link.getAttribute("href")).toMatch(/^\/unit\/?\?id=a1-u01$/);
    expect(screen.getByLabelText("Unit 2: Family")).toHaveAttribute("aria-disabled", "true");
  });

  it("says it is loading and draws no node until both answers arrived", async () => {
    const never = new Promise<never>(() => undefined);
    const api = fakeApi(undefined, { listUnits: () => never, getProgress: () => Promise.resolve(progressOf([])) });
    renderScreen(<UnitScreen id={null} />, api);
    expect(screen.getByRole("status")).toHaveTextContent("Loading...");
    expect(screen.queryByRole("region", { name: "Quest map" })).toBeNull();
  });
});

describe("the player", () => {
  it("starts a lesson session for the unit and asks the server for the first activity", async () => {
    const startSession = vi.fn(() => Promise.resolve(SESSION));
    await openPlayer([stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "Where is Dewi?", options: ["school", "home"] }))], { startSession });
    await screen.findByText("Where is Dewi?");
    expect(startSession).toHaveBeenCalledWith({
      kind: "lesson",
      unit_id: "a1-u01",
      activity_id: null,
      mode: null,
      topic: null,
      level: null,
      speak: null,
    });
  });

  it("shows the unit's presentation blocks and dialogue as text", async () => {
    await openPlayer([stepOf(null)]);
    await screen.findByText("Greet people by time of day.");
    expect(screen.getByText("New classmates")).toBeInTheDocument();
    expect(screen.getByText("Good evening!")).toBeInTheDocument();
  });

  it("renders a choice body and sends the index of the option that was picked", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", 1) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "Where is Dewi?", options: ["school", "home"] }))], { submitActivity });
    await user.click(await screen.findByRole("radio", { name: "home" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "choice", index: 1 } });
  });

  it("renders a gap fill body and sends one answer per gap", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a2", 1) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a2", { kind: "gap_fill", text: "I ___ Rina.", gaps: 1 }))], { submitActivity });
    await user.type(await screen.findByRole("textbox", { name: "Gap 1" }), "am");
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a2", answer: { kind: "gaps", answers: ["am"] } });
  });

  it("renders a reorder body and sends the tokens in the order they were placed", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", 1) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "reorder", tokens: ["name", "My", "is", "Dewi"] }))], { submitActivity });
    await user.click(await screen.findByRole("button", { name: "Add: My" }));
    await user.click(screen.getByRole("button", { name: "Add: name" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "order", tokens: ["My", "name"] } });
  });

  it("renders a match body and sends a pick per item", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", 1) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "match", left: ["hello"], right: ["selamat tinggal", "halo"] }))], { submitActivity });
    await user.selectOptions(await screen.findByLabelText("Match for: hello"), "halo");
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "picks", picks: [1] } });
  });

  it("renders a dictation body and sends the typed text", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", 1) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "dictation", audio: [{ speaker: null, text: "I am from Bandung." }] }))], { submitActivity });
    await user.type(await screen.findByLabelText("Your answer"), "I am from Bandung.");
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "text", text: "I am from Bandung." } });
  });

  it("renders an error correction body and sends the corrected sentence", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", 1) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "error_correction", sentence: "I Rina." }))], { submitActivity });
    await user.type(await screen.findByLabelText("Your answer"), "I am Rina.");
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "text", text: "I am Rina." } });
  });

  it("renders a writing body with the word range and sends the text", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", null) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "production", prompt: { en: "Introduce yourself." }, content_points: ["your name"], min_words: 8, max_words: 40 }, { activity_type: "guided_writing" }))], { submitActivity });
    expect(await screen.findByText("Introduce yourself.")).toBeInTheDocument();
    expect(screen.getByText("your name")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Your answer"), "My name is Dewi");
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "text", text: "My name is Dewi" } });
  });

  it("renders a reading set as one choice per question and sends the picks", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", 1) }));
    const user = userEvent.setup();
    await openPlayer([
      stepOf(presented("a1", { kind: "reading_set", passage: "Dewi is at school.", questions: [{ stem: "Where?", options: ["school", "home"] }, { stem: "Who?", options: ["Dewi", "Arif"] }] })),
    ], { submitActivity });
    expect(await screen.findByText("Dewi is at school.")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "school" }));
    await user.click(screen.getByRole("radio", { name: "Arif" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "picks", picks: [0, 1] } });
  });

  it("renders a listening set's questions and sends the picks", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", 1) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "listening_set", audio: [{ speaker: null, text: "I am Putu." }], replays_allowed: 2, questions: [{ stem: "Who speaks?", options: ["Putu", "Dewi"] }] }))], { submitActivity });
    await user.click(await screen.findByRole("radio", { name: "Putu" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "picks", picks: [0] } });
  });

  it("shows the pending line when the server queued the answer", async () => {
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "Q", options: ["a", "b"] }))], {
      submitActivity: () => Promise.resolve({ result: scored("a1", null) }),
    });
    await user.click(await screen.findByRole("radio", { name: "a" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(await screen.findByText("Saved. Lumi will mark this when the AI provider is reachable.")).toBeInTheDocument();
  });

  it("shows the verdict and the accepted answer when the server scored the answer", async () => {
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "Q", options: ["a", "b"] }))], {
      submitActivity: () => Promise.resolve({ result: scored("a1", 0, "am") }),
    });
    await user.click(await screen.findByRole("radio", { name: "a" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(await screen.findByText("Not quite")).toBeInTheDocument();
    expect(screen.getByText("Answer: am")).toBeInTheDocument();
  });

  it("renders a read-aloud body as presentation and offers no Check button", async () => {
    await openPlayer([stepOf(presented("a7", { kind: "read_aloud", text: "Thank you very much.", focus_phonemes: ["TH"] }))]);
    expect(await screen.findByText("Thank you very much.")).toBeInTheDocument();
    expect(screen.getByText("Sounds to practise: TH")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });

  it("renders a shadowing body as presentation and offers no Check button", async () => {
    await openPlayer([stepOf(presented("a9", { kind: "shadowing", title: "Repeat after Dewi", lines: [{ speaker: "Dewi", text: "Nice to meet you." }] }))]);
    expect(await screen.findByText("Repeat after Dewi")).toBeInTheDocument();
    expect(screen.getByText("Nice to meet you.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });

  it("renders a spoken minimal pair body as presentation and offers no Check button", async () => {
    await openPlayer([stepOf(presented("a8", { kind: "minimal_pairs_say", pairs: [{ a: "think", b: "sink", focus: "TH" }] }))]);
    expect(await screen.findByText("think")).toBeInTheDocument();
    expect(screen.getByText("sink")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });

  it("renders a spoken production body as presentation and offers no Check button", async () => {
    await openPlayer([
      stepOf(presented("a10", { kind: "production", prompt: { en: "Say where you are from." }, content_points: ["your city"], min_words: 5, max_words: 20 }, { activity_type: "guided_speaking" })),
    ]);
    expect(await screen.findByText("Say where you are from.")).toBeInTheDocument();
    expect(screen.getByText("your city")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });

  it("renders a mediation body and sends the relayed text", async () => {
    const submitActivity = vi.fn(() => Promise.resolve({ result: scored("a1", null) }));
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "mediation", source_text: "Kelas mulai pukul tujuh.", task: { en: "Tell Arif in English." } }, { activity_type: "mediation" }))], { submitActivity });
    expect(await screen.findByText("Kelas mulai pukul tujuh.")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Your answer"), "Class starts at seven.");
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "text", text: "Class starts at seven." } });
  });

  it("shows the server's own reason when it stored an answer without scoring it", async () => {
    const user = userEvent.setup();
    const unscored: ActivityResultView = {
      activity_id: "a1",
      activity_type: "guided_writing",
      score: null,
      confidence: null,
      outcome: { kind: "unscored", reason: "This activity is practice and has no score." },
    };
    await openPlayer([stepOf(presented("a1", { kind: "production", prompt: { en: "Introduce yourself." }, content_points: [], min_words: 8, max_words: 40 }, { activity_type: "guided_writing" }))], {
      submitActivity: () => Promise.resolve({ result: unscored }),
    });
    await user.type(await screen.findByLabelText("Your answer"), "My name is Dewi");
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(await screen.findByText("This activity is practice and has no score.")).toBeInTheDocument();
    expect(screen.queryByText("Saved. Lumi will mark this when the AI provider is reachable.")).toBeNull();
  });

  it("walks on to the next activity the server offers", async () => {
    const user = userEvent.setup();
    await openPlayer([
      stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "First", options: ["a", "b"] })),
      stepOf(presented("a2", { kind: "mcq", passage: null, audio: null, stem: "Second", options: ["c", "d"] })),
    ]);
    await user.click(await screen.findByRole("radio", { name: "a" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    await user.click(await screen.findByRole("button", { name: "Next" }));
    expect(await screen.findByText("Second")).toBeInTheDocument();
  });

  it("shows the server's reason and no Check button for an activity it cannot run", async () => {
    await openPlayer([
      stepOf(null, {
        unavailable: [{ activity_id: "a3", reason: "this activity is played aloud and needs speech output, which is not available" }],
      }),
    ]);
    const reason = "this activity is played aloud and needs speech output, which is not available";
    await screen.findByText(reason);
    const note = screen.getByText(reason).closest("li") as HTMLElement;
    expect(within(note).getByText("Write two sentences.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });

  it("stops the session when no activity is left and shows the checkpoint the server settled", async () => {
    const stopSession = vi.fn(() =>
      Promise.resolve(
        endedWith(
          [
            { activity_id: "a1", answered: true, score: 1 },
            { activity_id: "a2", answered: true, score: 0.5 },
            { activity_id: "a3", answered: false, score: null },
          ],
          0.75,
          0.7,
          2,
        ),
      ),
    );
    await openPlayer([stepOf(null)], { stopSession });
    expect(await screen.findByText("Mean: 75 percent")).toBeInTheDocument();
    expect(screen.getByText("Pass mark: 70 percent")).toBeInTheDocument();
    expect(screen.getByText("2 of 3 checkpoint activities answered.")).toBeInTheDocument();
    expect(screen.getByText("The checkpoint is passed.")).toBeInTheDocument();
    expect(stopSession).toHaveBeenCalledWith(42);
    const rows = within(screen.getByRole("table")).getAllByRole("row");
    expect(rows).toHaveLength(4);
    expect(within(rows[1] as HTMLElement).getByText("Read and choose.")).toBeInTheDocument();
    expect(within(rows[3] as HTMLElement).getByText("Not answered")).toBeInTheDocument();
  });

  it("counts only what the run answered in the four skill sections", async () => {
    const user = userEvent.setup();
    await openPlayer([stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "Q", options: ["a", "b"] }), { total: 3 })]);
    await screen.findByText("Q");
    const reading = () => screen.getByText("Reading").closest("li") as HTMLElement;
    expect(within(reading()).getByText("0 of 1 activities answered in this run")).toBeInTheDocument();
    expect(within(screen.getByText("Writing").closest("li") as HTMLElement).getByText("0 of 1 activities answered in this run")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "a" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(await within(reading()).findByText("1 of 1 activities answered in this run")).toBeInTheDocument();
  });

  it("reveals the unit's Indonesian instruction when the help is switched on", async () => {
    const user = userEvent.setup();
    await openPlayer([stepOf(null)]);
    expect(screen.queryByText("Baca lalu pilih.")).toBeNull();
    await user.click(await screen.findByRole("switch", { name: /Help in Indonesian/ }));
    expect(await screen.findByText("Baca lalu pilih.")).toBeInTheDocument();
  });
});

describe("the roleplay", () => {
  const ROLEPLAY = presented(
    "a11",
    {
      kind: "roleplay",
      scenario: { en: "Two new students meet.", id: "Dua siswa baru bertemu." },
      tutor_role: "a classmate",
      learner_role: "a new student",
      goals: ["Say hello", "Say your name"],
      max_turns: 4,
    },
    { activity_type: "roleplay", skill: "speaking_interaction" },
  );

  it("starts the conversation, shows the server's sentence, and finishes to walk on", async () => {
    const user = userEvent.setup();
    const submitActivity = vi
      .fn()
      .mockResolvedValueOnce({ result: null })
      .mockResolvedValueOnce({ result: unscored("a11", "This activity is practice and has no score.") });
    const api = await openPlayer([stepOf(ROLEPLAY), stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "After the roleplay", options: ["a", "b"] }))], {
      submitActivity,
    });

    await user.click(await screen.findByRole("button", { name: "Start the conversation" }));
    expect(submitActivity).toHaveBeenCalledWith({ session_id: 42, activity_id: "a11", answer: { kind: "roleplay_start" } });

    // The engine reports the missing provider on the event stream.
    api.emit({
      type: "SessionState",
      seq: 3,
      session_id: 42,
      kind: "lesson",
      life: "provider_unavailable",
      fault: null,
      message: "The language model provider could not be reached or kept failing. Check the connection and the provider settings, then resume.",
    });
    expect(
      await screen.findByText("The language model provider could not be reached or kept failing. Check the connection and the provider settings, then resume."),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Finish the conversation" }));
    expect(submitActivity).toHaveBeenLastCalledWith({ session_id: 42, activity_id: "a11", answer: { kind: "roleplay_finish" } });

    await user.click(await screen.findByRole("button", { name: "Next" }));
    expect(await screen.findByText("After the roleplay")).toBeInTheDocument();
  });

  it("shows the server's unscored reason, never a waiting-for-provider claim, and walks on", async () => {
    const user = userEvent.setup();
    const submitActivity = vi
      .fn()
      .mockResolvedValueOnce({ result: null })
      .mockResolvedValueOnce({ result: unscored("a11", "This activity is practice and has no score.") });
    await openPlayer([stepOf(ROLEPLAY), stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "After the roleplay", options: ["a", "b"] }))], {
      submitActivity,
    });

    await user.click(await screen.findByRole("button", { name: "Start the conversation" }));
    await user.click(await screen.findByRole("button", { name: "Finish the conversation" }));

    // The server's own sentence, and not the frame's pending verdict.
    expect(await screen.findByText("This activity is practice and has no score.")).toBeInTheDocument();
    expect(screen.queryByText("Saved. Lumi will mark this when the AI provider is reachable.")).toBeNull();
    expect(screen.queryByText("Correct!")).toBeNull();
    expect(screen.queryByText("Not quite")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Next" }));
    expect(await screen.findByText("After the roleplay")).toBeInTheDocument();
  });

  it("shows a practice line from the dictionaries when the server gave no reason", async () => {
    const user = userEvent.setup();
    const submitActivity = vi
      .fn()
      .mockResolvedValueOnce({ result: null })
      .mockResolvedValueOnce({ result: unscored("a11", "  ") });
    await openPlayer([stepOf(ROLEPLAY)], { submitActivity });

    await user.click(await screen.findByRole("button", { name: "Start the conversation" }));
    await user.click(await screen.findByRole("button", { name: "Finish the conversation" }));

    expect(await screen.findByText("Practice: this activity has no score.")).toBeInTheDocument();
    expect(screen.queryByText("Saved. Lumi will mark this when the AI provider is reachable.")).toBeNull();
  });

  it("still shows the verdict when the server scored the roleplay", async () => {
    const user = userEvent.setup();
    const submitActivity = vi
      .fn()
      .mockResolvedValueOnce({ result: null })
      .mockResolvedValueOnce({
        result: {
          activity_id: "a11",
          activity_type: "roleplay",
          score: 0.8,
          confidence: 0.9,
          outcome: {
            kind: "rubric",
            feedback: {
              dimensions: [{ dimension: "interaction", band: 3, reason: "Kept the exchange going.", evidence_quotes: [], status: "scored", capped: false }],
              content_points: [{ point: "Say hello", covered: true, quote: "Hello!" }],
              on_task: true,
              feedback_en: "You kept the conversation going.",
              feedback_l1: "",
              confidence: 0.9,
              runs: 1,
            },
          },
        } satisfies ActivityResultView,
      });
    await openPlayer([stepOf(ROLEPLAY)], { submitActivity });

    await user.click(await screen.findByRole("button", { name: "Start the conversation" }));
    await user.click(await screen.findByRole("button", { name: "Finish the conversation" }));

    expect(await screen.findByText("Correct!")).toBeInTheDocument();
    expect(screen.queryByText("Saved. Lumi will mark this when the AI provider is reachable.")).toBeNull();
  });
});

describe("a step that failed", () => {
  it("retries the next-activity step and not the session, and the walk continues", async () => {
    const user = userEvent.setup();
    const startSession = vi.fn(() => Promise.resolve(SESSION));
    const nextActivity = vi
      .fn()
      .mockResolvedValueOnce(stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "First", options: ["a", "b"] })))
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(stepOf(presented("a2", { kind: "mcq", passage: null, audio: null, stem: "Second", options: ["c", "d"] })));
    await openPlayer([], { startSession, nextActivity });

    // The first step arrived, the answer is checked, and "Next" fails.
    await user.click(await screen.findByRole("radio", { name: "a" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    await user.click(await screen.findByRole("button", { name: "Next" }));

    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    expect(startSession).toHaveBeenCalledTimes(1);
    expect(nextActivity).toHaveBeenCalledTimes(2);

    // The retry runs the step that failed, never a new session.
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Second")).toBeInTheDocument();
    expect(startSession).toHaveBeenCalledTimes(1);
    expect(nextActivity).toHaveBeenCalledTimes(3);
  });

  it("retries a failed submit with the same answer and never starts a session", async () => {
    const user = userEvent.setup();
    const startSession = vi.fn(() => Promise.resolve(SESSION));
    const submitActivity = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(500, "internal", "the answer could not be stored"))
      .mockResolvedValueOnce({ result: scored("a1", 1) });
    await openPlayer([stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "Q", options: ["a", "b"] }))], { startSession, submitActivity });

    await user.click(await screen.findByRole("radio", { name: "a" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Something went wrong inside the program");

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Correct!")).toBeInTheDocument();
    expect(startSession).toHaveBeenCalledTimes(1);
    expect(submitActivity).toHaveBeenCalledTimes(2);
    expect(submitActivity).toHaveBeenLastCalledWith({ session_id: 42, activity_id: "a1", answer: { kind: "choice", index: 0 } });
  });

  it("retries a failed stop and shows the checkpoint the server settled", async () => {
    const user = userEvent.setup();
    const startSession = vi.fn(() => Promise.resolve(SESSION));
    const stopSession = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(endedWith([{ activity_id: "a1", answered: true, score: 1 }], 1, 0.7, 1));
    await openPlayer([stepOf(null)], { startSession, stopSession });

    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    expect(startSession).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Mean: 100 percent")).toBeInTheDocument();
    expect(startSession).toHaveBeenCalledTimes(1);
    expect(stopSession).toHaveBeenCalledTimes(2);
  });

  it("retries the start when starting the session failed, and offers a way back", async () => {
    const startSession = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(SESSION);
    await openPlayer([stepOf(presented("a1", { kind: "mcq", passage: null, audio: null, stem: "First", options: ["a", "b"] }))], { startSession });

    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    // next/link adds or drops the trailing slash from the build setting, so accept both.
    expect(screen.getByRole("link", { name: "Back to the units" }).getAttribute("href")).toMatch(/^\/unit\/?$/);

    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("First")).toBeInTheDocument();
    expect(startSession).toHaveBeenCalledTimes(2);
  });
});
