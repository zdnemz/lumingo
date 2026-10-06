import { describe, expect, it } from "vitest";
import { dictionaries, translate } from "./index";

/** Wording that must never reach a learner (product rules, section "Allowed and banned wording"). */
const BANNED = [
  "fully offline",
  "fully local",
  "no cloud",
  "private",
  "certified",
  "official",
  "ielts",
  "toefl",
  "accurate pronunciation",
  "securely",
  "encrypt",
  "website",
  "online app",
  "hosted",
  // Indonesian counterparts
  "sepenuhnya offline",
  "tanpa cloud",
  "bersertifikat",
  "resmi",
  "terenkripsi",
  "dienkripsi",
  "aman tersimpan",
  "situs web",
  "aplikasi online",
  "di-host",
  "pribadi",
];

describe("dictionaries", () => {
  it("define the same keys in both languages", () => {
    expect(Object.keys(dictionaries.id).sort()).toEqual(Object.keys(dictionaries.en).sort());
  });

  it("have no empty strings", () => {
    for (const [language, table] of Object.entries(dictionaries)) {
      for (const [key, value] of Object.entries(table)) {
        expect(value.trim(), `${language}:${key}`).not.toBe("");
      }
    }
  });

  it("use the same {slots} in both languages", () => {
    const slots = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
    for (const key of Object.keys(dictionaries.en) as (keyof typeof dictionaries.en)[]) {
      expect(slots(dictionaries.id[key]), key).toEqual(slots(dictionaries.en[key]));
    }
  });

  it("never use banned wording", () => {
    for (const [language, table] of Object.entries(dictionaries)) {
      for (const [key, value] of Object.entries(table)) {
        const lower = value.toLowerCase();
        for (const word of BANNED) {
          expect(lower.includes(word), `${language}:${key} contains "${word}"`).toBe(false);
        }
      }
    }
  });
});

/**
 * Strings that are the same in both languages on purpose: a product or
 * language name, a loan word, or a unit. Any other equal pair is a string that
 * was copied and never translated.
 */
const SAME_ON_PURPOSE: readonly string[] = [
  "app.name",
  "link.mode",
  "settings.language.id",
  "settings.language.en",
  "settings.state.off",
  "quest.unit",
  "chat.tutor",
  "provider.protocol.anthropic_messages",
  "data.session.unit",
  "diag.program.title",
  "diag.program.mode",
  "diag.latency.p50",
  "hardware.minimum",
  "hardware.gb",
];

describe("untranslated strings", () => {
  const keys = Object.keys(dictionaries.en) as (keyof typeof dictionaries.en)[];
  const equal = keys.filter((key) => dictionaries.en[key] === dictionaries.id[key] && /\p{L}{3}/u.test(dictionaries.en[key]));

  it("are only the ones listed as the same on purpose", () => {
    expect(equal.filter((key) => !SAME_ON_PURPOSE.includes(key))).toEqual([]);
  });

  it("have no stale entries in that list", () => {
    expect(SAME_ON_PURPOSE.filter((key) => !equal.includes(key as (typeof keys)[number]))).toEqual([]);
  });
});

describe("translate", () => {
  it("fills slots", () => {
    expect(translate("en", "link.seconds", { count: 5 })).toBe("5 s");
    expect(translate("id", "link.seconds", { count: 5 })).toBe("5 dtk");
  });

  it("leaves an unknown slot visible", () => {
    expect(translate("en", "link.seconds", { other: 1 })).toBe("{count} s");
  });
});
