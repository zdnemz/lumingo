import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type ApiClient } from "@/api/client";
import type { ActiveSessionView } from "@/generated/ActiveSessionView";
import type { ConversationSummaryView } from "@/generated/ConversationSummaryView";
import type { SessionEnded } from "@/generated/SessionEnded";
import type { TurnLine } from "@/generated/TurnLine";
import { SNAPSHOT, connect, fakeApi, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { TalkScreen } from "./TalkScreen";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

const SESSION: ActiveSessionView = {
  id: 7,
  kind: "text_chat",
  unit_id: null,
  channel: "text",
  life: "active",
  turn_state: "waiting",
  fault: null,
  turns_completed: 1,
  turn: 1,
  recent: [],
  pending_reply: null,
  speaking: false,
  activity_id: null,
};

const SUMMARY: ConversationSummaryView = {
  learner_turns: 3,
  top_errors: [
    { category: "articles", count: 2, quote: "I saw a elephant.", correction: "I saw an elephant." },
    { category: "tense", count: 1, quote: "Yesterday I go home.", correction: "Yesterday I went home." },
  ],
  unanalysed_turns: [4],
  analysis_unreliable: true,
};

const ENDED: SessionEnded = {
  id: 7,
  kind: "text_chat",
  status: "completed",
  feedback: { kind: "conversation", summary: SUMMARY },
};

/** The topic field is the only text box on the setup form. */
async function topicBox(): Promise<HTMLElement> {
  return screen.findByLabelText("What shall we talk about?");
}

async function startChat(overrides: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, { startSession: () => Promise.resolve(SESSION), ...overrides });
  renderScreen(<TalkScreen />, api);
  await connect(api);
  const user = userEvent.setup();
  await user.type(await topicBox(), "Food");
  await user.click(screen.getByRole("button", { name: "Start talking" }));
  return { api, user };
}

describe("starting a chat", () => {
  it("shows the coded refusal and keeps the Start button, because the refusal is the offline state", async () => {
    const startSession = vi.fn(() =>
      Promise.reject(new ApiError(409, "provider_not_configured", "no provider is configured")),
    );
    const api = fakeApi(undefined, { startSession });
    renderScreen(<TalkScreen />, api);
    await connect(api);
    const user = userEvent.setup();
    await user.type(await topicBox(), "Food");
    await user.click(screen.getByRole("button", { name: "Start talking" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("No AI provider is set up yet. Add one first.");
    expect(screen.getByText("Details from the program: no provider is configured")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start talking" })).toBeInTheDocument();
    expect(startSession).toHaveBeenCalledTimes(1);
  });

  it("sends the request the server documents, with the mode, the typed topic and the level", async () => {
    const startSession = vi.fn(() => Promise.resolve(SESSION));
    const api = fakeApi(undefined, { startSession });
    renderScreen(<TalkScreen />, api);
    await connect(api);
    const user = userEvent.setup();
    await user.click(screen.getByRole("radio", { name: "Accuracy: correct me as we go" }));
    await user.click(screen.getByRole("radio", { name: "B1" }));
    await user.type(await topicBox(), "  Food in my city  ");
    await user.click(screen.getByRole("button", { name: "Start talking" }));

    expect(startSession).toHaveBeenCalledWith({
      kind: "text_chat",
      unit_id: null,
      activity_id: null,
      mode: "accuracy",
      topic: { kind: "typed", text: "Food in my city" },
      level: "B1",
      speak: null,
    });
  });

  it("sends what the learner types and never starts a chat without a topic", async () => {
    const sendText = vi.fn(() => Promise.resolve({ session_id: 7, turn: 2 }));
    const startSession = vi.fn(() => Promise.resolve(SESSION));
    const api = fakeApi(undefined, { startSession, sendText });
    renderScreen(<TalkScreen />, api);
    await connect(api);
    const user = userEvent.setup();

    expect(screen.getByRole("button", { name: "Start talking" })).toBeDisabled();
    await user.type(await topicBox(), "Food");
    await user.click(screen.getByRole("button", { name: "Start talking" }));
    await user.type(await screen.findByLabelText("Your message"), "I like rice{Enter}");
    expect(sendText).toHaveBeenCalledWith(7, "I like rice");
  });
});

describe("a conversation", () => {
  it("draws the learner line, the streaming reply and then the summary the server sent", async () => {
    const stopSession = vi.fn(() => Promise.resolve(ENDED));
    const { api } = await startChat({ stopSession });
    const log = await screen.findByRole("log", { name: "Conversation" });
    expect(screen.getByText("Write the first message to begin.")).toBeInTheDocument();

    act(() => {
      api.emit({ type: "TranscriptFinal", seq: 1, session_id: 7, turn: 2, text: "I like rice.", source: "text", turn_seq: 11, edited: false });
    });
    expect(within(log).getByText("I like rice.")).toBeInTheDocument();

    act(() => {
      api.emit({ type: "TurnState", seq: 2, session_id: 7, turn: 2, state: "thinking", tutor_turn_seq: null });
    });
    expect(await screen.findByText("Lumi is thinking")).toBeInTheDocument();

    act(() => {
      api.emit({ type: "TurnState", seq: 3, session_id: 7, turn: 2, state: "replying", tutor_turn_seq: null });
    });
    act(() => {
      api.emit({ type: "TutorTextDelta", seq: 4, session_id: 7, turn: 2, delta: "Rice is " });
      api.emit({ type: "TutorTextDelta", seq: 5, session_id: 7, turn: 2, delta: "nice." });
    });
    expect(within(log).getByText("Rice is nice.")).toBeInTheDocument();
    expect(log).toHaveAttribute("aria-busy", "true");

    // The end of the session: the reply stops streaming and the summary arrives.
    act(() => {
      api.emit({ type: "TurnState", seq: 6, session_id: 7, turn: 2, state: "waiting", tutor_turn_seq: 12 });
      api.emit({ type: "FeedbackReady", seq: 7, session_id: 7, feedback: { kind: "conversation", summary: SUMMARY } });
      api.emit({ type: "SessionState", seq: 8, session_id: 7, kind: "text_chat", life: "ended", fault: null, message: null });
    });
    expect(log).not.toHaveAttribute("aria-busy");
    expect(within(log).getByText("I like rice.")).toBeInTheDocument();

    const panel = (await screen.findByRole("heading", { name: "What to practise next" })).closest("section") as HTMLElement;
    expect(panel).toHaveTextContent("Your turns: 3");
    expect(panel).toHaveTextContent("articles");
    expect(panel).toHaveTextContent("Seen 2 times");
    expect(panel).toHaveTextContent("You wrote: I saw a elephant.");
    expect(panel).toHaveTextContent("Better: I saw an elephant.");
    expect(panel).toHaveTextContent("Turns with no analysis: 4");
    expect(panel).toHaveTextContent("Much of the analysis was filtered out");
    expect(stopSession).not.toHaveBeenCalled();
  });

  it("finishes the session with the server and shows the summary from the answer", async () => {
    const stopSession = vi.fn(() => Promise.resolve(ENDED));
    const { user } = await startChat({ stopSession });
    await user.click(screen.getByRole("button", { name: "Finish the chat" }));
    expect(stopSession).toHaveBeenCalledWith(7);
    const panel = (await screen.findByRole("heading", { name: "What to practise next" })).closest("section") as HTMLElement;
    expect(panel).toHaveTextContent("Your turns: 3");
    expect(screen.queryByLabelText("Your message")).toBeNull();
  });

  it("says a session ended without a summary instead of inventing one", async () => {
    const { api } = await startChat();
    act(() => {
      api.emit({ type: "SessionState", seq: 1, session_id: 7, kind: "text_chat", life: "ended", fault: null, message: null });
    });
    expect(await screen.findByText("This conversation has ended.")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "What to practise next" })).toBeNull();
    expect(screen.queryByLabelText("Your message")).toBeNull();
  });
});

describe("the voice half", () => {
  it("shows the note the snapshot carries and offers no way to start it", async () => {
    const api = fakeApi();
    renderScreen(<TalkScreen />, api);
    await connect(api, { ...SNAPSHOT, unavailable: ["speech"] });
    const user = userEvent.setup();
    await user.click(await screen.findByRole("radio", { name: "By speaking" }));

    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent("Speech: microphone, speakers and voices: not part of this version");
    expect(screen.queryByRole("button", { name: "Start talking" })).toBeNull();
    expect(screen.queryByLabelText("Your message")).toBeNull();
    // No push-to-talk control either: the server would answer 409, not the missing speech.
    expect(screen.queryByRole("button", { name: "Hold to talk" })).toBeNull();
  });
});

describe("a page that was reloaded", () => {
  it("rebuilds the conversation from the snapshot before any event arrives", async () => {
    const recent: TurnLine[] = [
      { turn: 1, role: "tutor", text: "Hello there. What did you eat?", seq: 3, edited: false },
      { turn: 2, role: "learner", text: "I ate rice.", seq: 4, edited: true },
    ];
    const api = fakeApi();
    renderScreen(<TalkScreen />, api);
    await connect(api, { ...SNAPSHOT, active_session: { ...SESSION, recent } });

    const log = await screen.findByRole("log", { name: "Conversation" });
    expect(within(log).getByText("Hello there. What did you eat?")).toBeInTheDocument();
    expect(within(log).getByText("I ate rice.")).toBeInTheDocument();
    expect(within(log).getByText("(edited)")).toBeInTheDocument();
    expect(screen.getByLabelText("Your message")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Finish the chat" })).toBeInTheDocument();
  });
});
