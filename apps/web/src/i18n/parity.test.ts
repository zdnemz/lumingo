import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { en } from "./en";

/**
 * The compiler already refuses a key that exists in one dictionary and not the
 * other (`id.ts` is `satisfies Record<MessageKey, string>`, and `en.ts` defines
 * `MessageKey`). This test fails the same way from the source text, so the
 * check also holds when a key is added to one file and the type check is not
 * run, and it catches a key written twice in one file.
 *
 * The strings live in `parts/<area>.en.ts` / `parts/<area>.id.ts` pairs. Two
 * screens may be written at the same time, so the checks run per pair, and the
 * cross-pair duplicate check below keeps the composed dictionary honest.
 */
const AREAS = ["shell", "unit", "progress", "settings", "talk", "write", "read"] as const;

function keysOf(file: string): string[] {
  const path = fileURLToPath(new URL(file, import.meta.url));
  const source = readFileSync(path, "utf8");
  return [...source.matchAll(/^ {2}"([^"]+)":/gm)].map((match) => match[1] as string);
}

describe("dictionary files", () => {
  const parts = AREAS.map((area) => ({
    area,
    en: keysOf(`./parts/${area}.en.ts`),
    id: keysOf(`./parts/${area}.id.ts`),
  }));

  it("have keys at all", () => {
    const total = parts.reduce((sum, part) => sum + part.en.length, 0);
    expect(total).toBeGreaterThan(400);
  });

  it("define no key twice", () => {
    for (const part of parts) {
      for (const [name, keys] of [
        [`parts/${part.area}.en.ts`, part.en],
        [`parts/${part.area}.id.ts`, part.id],
      ] as const) {
        const seen = new Set<string>();
        for (const key of keys) {
          expect(seen.has(key), `${name} defines "${key}" twice`).toBe(false);
          seen.add(key);
        }
      }
    }
  });

  it("define no key twice across the parts", () => {
    // A duplicate in two parts is silent in the composed object (the later
    // spread wins), so compare the composed key count with the sum of the
    // parts. `Object.keys(en)` is what the app really looks up.
    const sum = parts.reduce((total, part) => total + part.en.length, 0);
    expect(Object.keys(en).length, "the composed dictionary lost a key to a duplicate").toBe(sum);
  });

  it("have every key in both files", () => {
    for (const part of parts) {
      const inEn = new Set(part.en);
      const inId = new Set(part.id);
      expect(part.en.filter((key) => !inId.has(key)), `keys only in parts/${part.area}.en.ts`).toEqual([]);
      expect(part.id.filter((key) => !inEn.has(key)), `keys only in parts/${part.area}.id.ts`).toEqual([]);
    }
  });

  it("keep the keys in the same order, so a review can read the two side by side", () => {
    for (const part of parts) {
      expect(part.id, `parts/${part.area}.id.ts is out of order`).toEqual(part.en);
    }
  });
});
