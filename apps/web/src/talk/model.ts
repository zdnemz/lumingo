// Pure rules for the conversation screen: the lines a snapshot carries and the
// events the server sends, turned into what `ChatLog` draws. Nothing here
// invents a line, a turn or a summary: a turn that never happened has no line.

import type { ConversationSummaryView } from "@/generated/ConversationSummaryView";
import type { FeedbackMode } from "@/generated/FeedbackMode";
import type { Level } from "@/generated/Level";
import type { ServerEvent } from "@/generated/ServerEvent";
import type { TurnLine } from "@/generated/TurnLine";
import type { TurnPhase } from "@/generated/TurnPhase";
import type { ChatMessage } from "./ChatLog";

/**
 * The levels a learner may pick for a chat. They are the generated `Level`
 * union as data, not dictionary keys: a CEFR code is never translated.
 */
export const LEVELS: readonly Level[] = ["A1", "A2", "B1", "B2", "C1", "C2"];

/** The two feedback modes of a conversation (`FeedbackMode`). */
export const MODES: readonly FeedbackMode[] = ["fluency", "accuracy"];

/** What the screen shows of one conversation. */
export interface Transcript {
  /** Oldest first, the order `ChatLog` draws them in. */
  messages: ChatMessage[];
  /** The tutor has the turn and has not written anything yet. */
  thinking: boolean;
}

export const EMPTY: Transcript = { messages: [], thinking: false };

/** One line is keyed by its turn and its side, so an event replaces the line it is about. */
const learnerId = (turn: number) => `l-${turn}`;
const tutorId = (turn: number) => `t-${turn}`;

/** The conversation a snapshot carries: the lines of the turns that ran, oldest first. */
export function seed(lines: readonly TurnLine[]): Transcript {
  return {
    messages: lines.map((line) => ({
      id: line.role === "learner" ? learnerId(line.turn) : tutorId(line.turn),
      role: line.role,
      text: line.text,
      edited: line.edited,
    })),
    thinking: false,
  };
}

/** The session an event belongs to, or `null` for the events that name none. */
export function sessionIdOf(event: ServerEvent): number | null {
  if (!("session_id" in event)) return null;
  return typeof event.session_id === "number" ? event.session_id : null;
}

/** Replaces the line with the same key, or puts it at the end. */
function put(messages: readonly ChatMessage[], message: ChatMessage): ChatMessage[] {
  return messages.some((current) => current.id === message.id)
    ? messages.map((current) => (current.id === message.id ? message : current))
    : [...messages, message];
}

/** Marks the reply of `turn` as complete: the server moved the turn on. */
function settle(messages: readonly ChatMessage[], turn: number): ChatMessage[] {
  const id = tutorId(turn);
  return messages.map((message) => (message.id === id ? { ...message, streaming: false } : message));
}

/** Nothing is being written any more: the turn ended, or the session stopped. */
function stopWriting(transcript: Transcript): Transcript {
  return {
    messages: transcript.messages.map((message) => (message.streaming ? { ...message, streaming: false } : message)),
    thinking: false,
  };
}

function turnState(transcript: Transcript, turn: number, phase: TurnPhase): Transcript {
  switch (phase) {
    case "thinking":
      return { messages: stopWriting(transcript).messages, thinking: true };
    case "replying":
      return { messages: transcript.messages, thinking: false };
    case "waiting":
      return { messages: settle(transcript.messages, turn), thinking: false };
    // The phases only a voice turn uses cannot arrive for a text chat.
    default:
      return transcript;
  }
}

/**
 * One event folded into the conversation. Every event is keyed by its turn, so
 * applying one twice changes nothing, except that a delta continues the reply
 * it belongs to.
 */
export function applyEvent(transcript: Transcript, event: ServerEvent): Transcript {
  switch (event.type) {
    case "TranscriptFinal":
      return {
        ...transcript,
        messages: put(transcript.messages, {
          id: learnerId(event.turn),
          role: "learner",
          text: event.text,
          edited: event.edited,
        }),
      };
    case "TutorTextDelta": {
      const id = tutorId(event.turn);
      const written = transcript.messages.find((message) => message.id === id)?.text ?? "";
      return {
        ...transcript,
        messages: put(transcript.messages, { id, role: "tutor", text: written + event.delta, streaming: true }),
      };
    }
    case "TurnState":
      return turnState(transcript, event.turn, event.state);
    case "SessionState":
      // A paused, stopped or ended session is not writing any more.
      return event.life === "active" ? transcript : stopWriting(transcript);
    default:
      return transcript;
  }
}

/** The summary of the end, when the event carries one for this conversation. */
export function summaryOf(event: ServerEvent): ConversationSummaryView | null {
  return event.type === "FeedbackReady" && event.feedback.kind === "conversation" ? event.feedback.summary : null;
}
