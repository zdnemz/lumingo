import { beforeEach, describe, expect, it } from "vitest";
import { ONBOARDING_KEY, STEPS, loadProgress, parseProgress, saveProgress, startStep } from "./progress";

beforeEach(() => window.localStorage.clear());

describe("parseProgress", () => {
  it("reads a stored step", () => {
    expect(parseProgress('{"step":"test","done":false}')).toEqual({ step: "test", done: false });
    expect(parseProgress('{"step":"ready","done":true}')).toEqual({ step: "ready", done: true });
  });

  it("treats anything damaged as never started", () => {
    for (const raw of [null, "", "nope", "42", "null", '{"step":"nowhere"}', '{"done":true}', "[]"]) {
      expect(parseProgress(raw), String(raw)).toBeNull();
    }
  });
});

describe("startStep", () => {
  it("starts at the welcome when nothing is stored", () => {
    expect(startStep(null, { hasProvider: false })).toBe("welcome");
  });

  it("returns to the stored step", () => {
    for (const step of STEPS) {
      expect(startStep({ step, done: false }, { hasProvider: true })).toBe(step);
    }
  });

  it("will not stand past the provider step without a provider profile", () => {
    expect(startStep({ step: "test", done: false }, { hasProvider: false })).toBe("provider");
    expect(startStep({ step: "ready", done: false }, { hasProvider: false })).toBe("provider");
    expect(startStep({ step: "privacy", done: false }, { hasProvider: false })).toBe("privacy");
    expect(startStep({ step: "provider", done: false }, { hasProvider: false })).toBe("provider");
  });

  it("starts again from the welcome after setup was finished", () => {
    expect(startStep({ step: "ready", done: true }, { hasProvider: true })).toBe("welcome");
  });
});

describe("storage", () => {
  it("round-trips", () => {
    saveProgress(window, { step: "hardware", done: false });
    expect(loadProgress(window)).toEqual({ step: "hardware", done: false });
  });

  it("works when storage is blocked", () => {
    const get = Storage.prototype.getItem;
    const set = Storage.prototype.setItem;
    Storage.prototype.getItem = () => {
      throw new Error("blocked");
    };
    Storage.prototype.setItem = () => {
      throw new Error("blocked");
    };
    try {
      expect(loadProgress(window)).toBeNull();
      expect(() => saveProgress(window, { step: "test", done: false })).not.toThrow();
    } finally {
      Storage.prototype.getItem = get;
      Storage.prototype.setItem = set;
    }
    expect(window.localStorage.getItem(ONBOARDING_KEY)).toBeNull();
  });
});
