import { describe, expect, it } from "vitest";
import {
  BOOT_SCRIPT,
  PREFS_KEY,
  defaultPreferences,
  guessLanguage,
  parsePreferences,
  resolveDisplay,
  resolveMotion,
  resolveTheme,
} from "./preferences";

describe("language guess", () => {
  it("picks Indonesian for Indonesian browsers and English otherwise", () => {
    expect(guessLanguage("id")).toBe("id");
    expect(guessLanguage("id-ID")).toBe("id");
    expect(guessLanguage("en-US")).toBe("en");
    expect(guessLanguage("")).toBe("en");
  });
});

describe("parsePreferences", () => {
  it("returns defaults when nothing is stored", () => {
    expect(parsePreferences(null, "id-ID")).toEqual({
      motion: "system",
      theme: "system",
      language: "id",
      crt: false,
    });
  });

  it("falls back to defaults for broken JSON and for non-objects", () => {
    expect(parsePreferences("{oops")).toEqual(defaultPreferences());
    expect(parsePreferences("42")).toEqual(defaultPreferences());
    expect(parsePreferences("null")).toEqual(defaultPreferences());
  });

  it("keeps valid fields and replaces invalid ones one by one", () => {
    const stored = JSON.stringify({ motion: "off", theme: "sparkles", language: "id", crt: "yes" });
    expect(parsePreferences(stored)).toEqual({
      motion: "off",
      theme: "system",
      language: "id",
      crt: false,
    });
  });
});

describe("resolving settings", () => {
  it("follows the system when the setting is system", () => {
    expect(resolveMotion("system", true)).toBe("off");
    expect(resolveMotion("system", false)).toBe("on");
    expect(resolveTheme("system", true)).toBe("night");
    expect(resolveTheme("system", false)).toBe("day");
  });

  it("lets an explicit choice win over the system, in both directions", () => {
    expect(resolveMotion("on", true)).toBe("on");
    expect(resolveMotion("off", false)).toBe("off");
    expect(resolveTheme("day", true)).toBe("day");
  });

  it("builds the document values", () => {
    const prefs = { ...defaultPreferences("en"), motion: "off" as const, crt: true };
    expect(
      resolveDisplay(prefs, { prefersReducedMotion: false, prefersDark: true, browserLanguage: "en" }),
    ).toEqual({ motion: "off", theme: "night", crt: "on", language: "en" });
  });
});

/** Runs the boot script against a fake document and returns what it set. */
function runBootScript(stored: string | null, hints: { reduced: boolean; dark: boolean; lang: string }) {
  const dataset: Record<string, string> = {};
  const root = { dataset, lang: "" };
  const fakeDocument = { documentElement: root };
  const fakeStorage = { getItem: (key: string) => (key === PREFS_KEY ? stored : null) };
  const matchMedia = (query: string) => ({
    matches: query.includes("reduced-motion") ? hints.reduced : hints.dark,
  });
  const run = new Function("document", "localStorage", "matchMedia", "navigator", BOOT_SCRIPT);
  run(fakeDocument, fakeStorage, matchMedia, { language: hints.lang });
  return { ...dataset, lang: root.lang };
}

describe("boot script", () => {
  it("agrees with resolveDisplay for every stored combination", () => {
    const motions = ["system", "on", "off"] as const;
    const themes = ["system", "night", "day"] as const;
    for (const motion of motions) {
      for (const theme of themes) {
        for (const crt of [true, false]) {
          for (const reduced of [true, false]) {
            for (const dark of [true, false]) {
              for (const language of ["id", "en"] as const) {
                const prefs = { motion, theme, language, crt };
                const expected = resolveDisplay(prefs, {
                  prefersReducedMotion: reduced,
                  prefersDark: dark,
                  browserLanguage: "en",
                });
                const got = runBootScript(JSON.stringify(prefs), { reduced, dark, lang: "en" });
                expect(got).toEqual({
                  motion: expected.motion,
                  theme: expected.theme,
                  crt: expected.crt,
                  lang: expected.language,
                });
              }
            }
          }
        }
      }
    }
  });

  it("guesses the language the same way as the app when nothing is stored", () => {
    expect(runBootScript(null, { reduced: false, dark: true, lang: "id-ID" }).lang).toBe("id");
    expect(runBootScript(null, { reduced: false, dark: true, lang: "en-GB" }).lang).toBe("en");
  });

  it("chooses the still option when storage holds garbage", () => {
    const got = runBootScript("{garbage", { reduced: true, dark: false, lang: "en" });
    expect(got.motion).toBe("off");
  });

  it("turns motion off if reading the page state throws", () => {
    const dataset: Record<string, string> = {};
    const root = { dataset, lang: "" };
    const run = new Function("document", "localStorage", "matchMedia", "navigator", BOOT_SCRIPT);
    run(
      { documentElement: root },
      { getItem: () => null },
      () => {
        throw new Error("blocked");
      },
      { language: "en" },
    );
    expect(dataset.motion).toBe("off");
  });
});
