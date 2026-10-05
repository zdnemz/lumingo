import { describe, expect, it } from "vitest";
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

  it("ignores anything else", () => {
    expect(parseServerEvent("not json")).toBeNull();
    expect(parseServerEvent("42")).toBeNull();
    expect(parseServerEvent("null")).toBeNull();
    expect(parseServerEvent('{"type":"Mystery","seq":1}')).toBeNull();
    expect(parseServerEvent('{"type":"Heartbeat"}')).toBeNull();
    expect(parseServerEvent('{"type":"Heartbeat","seq":"1"}')).toBeNull();
  });
});

describe("ApiError", () => {
  it("tells a refusal from other failures", () => {
    expect(new ApiError(403, "origin_not_allowed").refused).toBe(true);
    expect(new ApiError(401, "session_required").refused).toBe(true);
    expect(new ApiError(500, "boom").refused).toBe(false);
  });
});
