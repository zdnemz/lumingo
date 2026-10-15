import { describe, expect, it } from "vitest";
import type { ServerEvent } from "@/generated/ServerEvent";
import { ApiError, parseServerEvent } from "./client";

describe("parseServerEvent", () => {
  it("accepts known events", () => {
    expect(parseServerEvent('{"type":"Heartbeat","seq":3,"uptime_ms":1000}')).toEqual({
      type: "Heartbeat",
      seq: 3,
      uptime_ms: 1000,
    });
    expect(parseServerEvent('{"type":"Snapshot","seq":0,"state":{}}')?.type).toBe("Snapshot");
  });

  it("accepts every event a free-mode screen reads", () => {
    const events = [
      { type: "SessionState", seq: 1, session_id: 7, kind: "writing", life: "active", fault: null, message: null },
      { type: "TurnState", seq: 2, session_id: 7, turn: 1, state: "waiting", tutor_turn_seq: null },
      {
        type: "TranscriptFinal",
        seq: 3,
        session_id: 7,
        turn: 1,
        text: "selamat pagi",
        source: "text",
        turn_seq: 4,
        edited: false,
      },
      { type: "TutorTextDelta", seq: 4, session_id: 7, turn: 1, delta: "Halo, " },
      {
        type: "FeedbackReady",
        seq: 5,
        session_id: 7,
        feedback: { kind: "reading", score: { correct: 1, total: 2, score: 0.5, questions: [] } },
      },
      { type: "Error", seq: 6, session_id: null, code: "provider_not_configured", message: "no provider" },
    ] satisfies ServerEvent[];

    for (const event of events) expect(parseServerEvent(JSON.stringify(event))).toEqual(event);
  });

  it("ignores anything else", () => {
    expect(parseServerEvent("not json")).toBeNull();
    expect(parseServerEvent("42")).toBeNull();
    expect(parseServerEvent("null")).toBeNull();
    expect(parseServerEvent('{"type":"Mystery","seq":1}')).toBeNull();
    expect(parseServerEvent('{"type":"Heartbeat"}')).toBeNull();
    expect(parseServerEvent('{"type":"Heartbeat","seq":"1"}')).toBeNull();
    expect(parseServerEvent('{"type":"MicLevel","seq":1,"level":0.5}')).toBeNull();
  });
});

describe("ApiError", () => {
  it("tells a refusal from other failures", () => {
    expect(new ApiError(403, "origin_not_allowed").refused).toBe(true);
    expect(new ApiError(401, "session_required").refused).toBe(true);
    expect(new ApiError(500, "boom").refused).toBe(false);
  });
});
