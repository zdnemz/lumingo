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

describe("translate", () => {
  it("fills slots", () => {
    expect(translate("en", "link.seconds", { count: 5 })).toBe("5 s");
    expect(translate("id", "link.seconds", { count: 5 })).toBe("5 dtk");
  });

  it("leaves an unknown slot visible", () => {
    expect(translate("en", "link.seconds", { other: 1 })).toBe("{count} s");
  });
});
