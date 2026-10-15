import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiClient } from "@/api/client";
import type { ActiveSessionView } from "@/generated/ActiveSessionView";
import type { AuthoredSetView } from "@/generated/AuthoredSetView";
import type { ReadingAnswered } from "@/generated/ReadingAnswered";
import type { ReadingOutcomeView } from "@/generated/ReadingOutcomeView";
import { PREFS_KEY } from "@/state/preferences";
import { SNAPSHOT, connect, fakeApi, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { ReadScreen } from "./ReadScreen";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true, language: "en-US" });
});

const SESSION: ActiveSessionView = {
  id: 42,
  kind: "reading",
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

/** The reading set of the shipped example unit, as the fallback carries it. */
const SET: AuthoredSetView = {
  unit_id: "a1-u01",
  activity_id: "a14-read-set-class-chat",
  text: {
    title: "Class chat",
    passage: "Class chat\n\nSari: Hello! I am from Surabaya.\nRudi: Hi, Sari! I am from Medan.",
    glossary: [],
    questions: [
      { stem: 'Rudi: "I am from ___."', options: ["Surabaya", "Medan", "Jakarta"] },
      { stem: "The teacher:", options: ["Sari", "Rudi", "Mrs. Lina"] },
      { stem: 'Sari: "I am ___, thank you!"', options: ["fine", "from", "nice"] },
    ],
  },
};

const FALLBACK: ReadingOutcomeView = { kind: "fallback", reason: "provider_unavailable", sets: [SET] };

/** What the engine answers for the picks of the fallback test: two of three right. */
const ANSWERED: ReadingAnswered = {
  score: {
    correct: 2,
    total: 3,
    score: 2 / 3,
    questions: [
      { chosen: 1, correct_index: 1, correct: true, explanation_en: "Rudi: I am from Medan.", explanation_l1: "Rudi menulis: I am from Medan." },
      { chosen: 0, correct_index: 2, correct: false, explanation_en: "Mrs. Lina: I am your teacher.", explanation_l1: "Mrs. Lina menulis: I am your teacher." },
      { chosen: 0, correct_index: 0, correct: true, explanation_en: "Sari: I am fine, thank you!", explanation_l1: "" },
    ],
  },
};

const GENERATED: ReadingOutcomeView = {
  kind: "generated",
  content_id: 7,
  regenerated: true,
  vocabulary_checked: false,
  text: {
    title: "A Day at the Market",
    passage: "Dewi goes to the market every Sunday. The market is busy.",
    glossary: [
      { word: "market", gloss_l1: "pasar", example: "The market opens early." },
      { word: "busy", gloss_l1: "ramai", example: "The street is busy." },
    ],
    questions: [
      { stem: "Where does Dewi go?", options: ["school", "the market"] },
      { stem: "How is the market?", options: ["quiet", "busy"] },
    ],
  },
};

interface Routes {
  startSession?: ReturnType<typeof vi.fn>;
  generateReading?: ReturnType<typeof vi.fn>;
  answerReading?: ReturnType<typeof vi.fn>;
}

/** A screen whose server answers the routes it is given, with a snapshot that lacks speech. */
async function open(outcome: ReadingOutcomeView, routes: Routes = {}) {
  const startSession = routes.startSession ?? vi.fn(() => Promise.resolve(SESSION));
  const generateReading = routes.generateReading ?? vi.fn(() => Promise.resolve(outcome));
  const answerReading = routes.answerReading ?? vi.fn(() => Promise.resolve(ANSWERED));
  const api = fakeApi(undefined, {
    startSession,
    generateReading,
    answerReading,
  } as Partial<ApiClient>);
  const view = renderScreen(<ReadScreen />, api);
  await connect(api, { ...SNAPSHOT, unavailable: ["speech"] });
  return { api, view, startSession, generateReading, answerReading };
}

/** Types a topic and asks for a text, which is the whole start of the screen. */
async function askForText(topic = "a day at the market", label = "What to read about", button = "Get a text") {
  const user = userEvent.setup();
  await user.type(await screen.findByLabelText(label), topic);
  await user.click(screen.getByRole("button", { name: button }));
}

describe("getting a text", () => {
  it("starts a reading session at the picked level and asks for a text about the topic", async () => {
    const { startSession, generateReading } = await open(FALLBACK);
    const user = userEvent.setup();
    await askForText();
    await screen.findByText("Class chat");

    expect(startSession).toHaveBeenCalledWith({
      kind: "reading",
      unit_id: null,
      activity_id: null,
      mode: null,
      topic: null,
      level: "A1",
      speak: null,
    });
    expect(generateReading).toHaveBeenCalledWith({ session_id: 42, topic: { kind: "typed", text: "a day at the market" } });

    // The server reads the level when the session starts, so the picker is gone and the fact stays.
    expect(screen.queryByRole("radio", { name: "B2" })).toBeNull();
    expect(screen.getByText("This text is at level A1. To read at another level, open the reading page again.")).toBeInTheDocument();

    // A second text is the same session, asked again: the level cannot change mid-session.
    await user.click(screen.getByRole("button", { name: "Another text" }));
    await vi.waitFor(() => expect(generateReading).toHaveBeenCalledTimes(2));
    expect(startSession).toHaveBeenCalledTimes(1);
  });

  it("sends the level the learner picked", async () => {
    const { startSession } = await open(FALLBACK);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("radio", { name: "B2" }));
    await askForText();
    await vi.waitFor(() => expect(startSession).toHaveBeenCalled());
    expect(startSession.mock.calls[0]?.[0]).toMatchObject({ kind: "reading", level: "B2" });
  });

  it("will not ask for a text without a topic, which the server refuses", async () => {
    await open(FALLBACK);
    expect(await screen.findByRole("button", { name: "Get a text" })).toBeDisabled();
  });

  it("shows the server's refusal and runs the same request again", async () => {
    const generateReading = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValueOnce(FALLBACK);
    await open(FALLBACK, { generateReading });
    await askForText();
    expect(await screen.findByRole("alert")).toHaveTextContent("The program is not answering");
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Class chat")).toBeInTheDocument();
  });
});

describe("the authored fallback", () => {
  it("shows the passage and its questions, and scores the answers with every explanation", async () => {
    const { answerReading } = await open(FALLBACK);
    const user = userEvent.setup();
    await askForText();
    await screen.findByText("Class chat");

    expect(screen.getByLabelText("Reading text")).toHaveTextContent("Rudi: Hi, Sari! I am from Medan.");
    expect(screen.getAllByRole("radio")).toHaveLength(9);
    // An authored set carries no glossary, so no word is a button.
    expect(screen.queryByRole("button", { name: /Word with a meaning/ })).toBeNull();

    await user.click(screen.getByRole("radio", { name: "Medan" }));
    await user.click(screen.getByRole("radio", { name: "Sari" }));
    await user.click(screen.getByRole("radio", { name: "fine" }));
    await user.click(screen.getByRole("button", { name: "Check" }));

    expect(await screen.findByText("2 of 3 correct")).toBeInTheDocument();
    expect(screen.getByText("Practice only: this score does not count toward a level estimate.")).toBeInTheDocument();
    expect(screen.getByText("Why: Rudi: I am from Medan.")).toBeInTheDocument();
    expect(screen.getByText("Why: Mrs. Lina: I am your teacher.")).toBeInTheDocument();
    expect(screen.getByText("Why: Sari: I am fine, thank you!")).toBeInTheDocument();

    // The answers are for the authored set the server named, and the choices lock.
    expect(answerReading).toHaveBeenCalledWith(42, {
      target: { kind: "authored", unit_id: "a1-u01", activity_id: "a14-read-set-class-chat" },
      answers: [1, 0, 0],
    });
    expect(screen.getByRole("radio", { name: "Medan" })).toBeDisabled();
  });

  it("sends exactly one answer per question, null for one left open", async () => {
    const { answerReading } = await open(FALLBACK);
    const user = userEvent.setup();
    await askForText();
    await screen.findByText("Class chat");

    await user.click(screen.getByRole("radio", { name: "Medan" }));
    await user.click(screen.getByRole("radio", { name: "fine" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    await screen.findByText("2 of 3 correct");

    const request = answerReading.mock.calls[0]?.[1] as { answers: (number | null)[] };
    expect(request.answers).toHaveLength(3);
    expect(request.answers).toEqual([1, null, 0]);
  });

  it("says plainly when the level has no authored set and no text could be written", async () => {
    await open({ kind: "fallback", reason: "provider_unavailable", sets: [] });
    await askForText();
    expect(await screen.findByText("Nothing to read at this level yet")).toBeInTheDocument();
    expect(screen.getByText(/Nothing was invented to fill the gap/)).toBeInTheDocument();
    expect(screen.queryByRole("radio")).toBeNull();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });

  it("repeats the server's reason for offering the authored sets", async () => {
    await open({ kind: "fallback", reason: "unusable", sets: [SET] });
    await askForText();
    expect(await screen.findByText(/Two texts from the provider did not pass the checks/)).toBeInTheDocument();
  });
});

describe("the learner's language", () => {
  it("shows the explanation in Indonesian first, and English only where that side is empty", async () => {
    window.localStorage.setItem(PREFS_KEY, JSON.stringify({ language: "id" }));
    await open(FALLBACK);
    const user = userEvent.setup();
    await askForText("sehari di pasar", "Mau membaca tentang apa", "Minta teks");
    await screen.findByText("Class chat");

    await user.click(screen.getByRole("radio", { name: "Medan" }));
    await user.click(screen.getByRole("radio", { name: "Sari" }));
    await user.click(screen.getByRole("radio", { name: "fine" }));
    await user.click(screen.getByRole("button", { name: "Periksa" }));

    expect(await screen.findByText("2 dari 3 benar")).toBeInTheDocument();
    expect(screen.getByText("Alasannya: Rudi menulis: I am from Medan.")).toBeInTheDocument();
    // The third explanation has no Indonesian side, so the English one is shown instead of nothing.
    expect(screen.getByText("Alasannya: Sari: I am fine, thank you!")).toBeInTheDocument();
  });
});

describe("a generated text", () => {
  it("labels it generated, says the vocabulary check was skipped, and opens the meanings", async () => {
    const { answerReading } = await open(GENERATED);
    const user = userEvent.setup();
    await askForText();
    await screen.findByText("A Day at the Market");

    expect(screen.getByText("Generated text")).toBeInTheDocument();
    expect(screen.getByText("A model wrote this text, so a detail may be wrong.")).toBeInTheDocument();
    expect(screen.getByText("The first text failed a check, so this is the second one.")).toBeInTheDocument();
    expect(screen.getByText("The vocabulary check was skipped: no word list is installed.")).toBeInTheDocument();

    // The word appears twice in the passage, so it is a button both times.
    const market = screen.getAllByRole("button", { name: "Word with a meaning: market" });
    expect(market).toHaveLength(2);
    await user.click(market[0] as HTMLElement);
    expect(screen.getByText("market means pasar")).toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "the market" }));
    await user.click(screen.getByRole("radio", { name: "quiet" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    await screen.findByText("2 of 3 correct");

    // A generated text is scored by its stored content id, not by a unit.
    expect(answerReading).toHaveBeenCalledWith(42, {
      target: { kind: "generated", content_id: 7 },
      answers: [1, 0],
    });
  });

  it("takes the score from the answer, not from the event that repeats it", async () => {
    const { api } = await open(FALLBACK);
    const user = userEvent.setup();
    await askForText();
    await screen.findByText("Class chat");
    await user.click(screen.getByRole("radio", { name: "Medan" }));
    await user.click(screen.getByRole("radio", { name: "Sari" }));
    await user.click(screen.getByRole("radio", { name: "fine" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    await screen.findByText("2 of 3 correct");

    api.emit({ type: "FeedbackReady", seq: 9, session_id: 42, feedback: { kind: "reading", score: ANSWERED.score } });
    expect(screen.getAllByText("2 of 3 correct")).toHaveLength(1);
  });
});

describe("reading aloud", () => {
  it("has no read-aloud control while the speech engine is missing, and says so", async () => {
    await open(GENERATED);
    await askForText();
    await screen.findByText("A Day at the Market");

    const note = screen.getByRole("note");
    expect(note).toHaveTextContent("Not available in this build");
    expect(note).toHaveTextContent("Speech: microphone, speakers and voices");
    expect(screen.queryByRole("button", { name: /aloud/i })).toBeNull();
    expect(screen.queryByRole("link", { name: /aloud/i })).toBeNull();
    expect(document.body.textContent).not.toMatch(/aloud/i);
  });
});
