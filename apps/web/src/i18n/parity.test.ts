import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * The compiler already refuses a key that exists in one dictionary and not the
 * other (`id.ts` is `satisfies Record<MessageKey, string>`, and `en.ts` defines
 * `MessageKey`). This test fails the same way from the source text, so the
 * check also holds when a key is added to one file and the type check is not
 * run, and it catches a key written twice in one file.
 */
function keysOf(file: string): string[] {
  const path = fileURLToPath(new URL(file, import.meta.url));
  const source = readFileSync(path, "utf8");
  return [...source.matchAll(/^ {2}"([^"]+)":/gm)].map((match) => match[1] as string);
}

describe("dictionary files", () => {
  const en = keysOf("./en.ts");
  const id = keysOf("./id.ts");

  it("have keys at all", () => {
    expect(en.length).toBeGreaterThan(400);
  });

  it("define no key twice", () => {
    for (const [name, keys] of [
      ["en", en],
      ["id", id],
    ] as const) {
      const seen = new Set<string>();
      for (const key of keys) {
        expect(seen.has(key), `${name}.ts defines "${key}" twice`).toBe(false);
        seen.add(key);
      }
    }
  });

  it("have every key in both files", () => {
    const inEn = new Set(en);
    const inId = new Set(id);
    expect(en.filter((key) => !inId.has(key)), "keys only in en.ts").toEqual([]);
    expect(id.filter((key) => !inEn.has(key)), "keys only in id.ts").toEqual([]);
  });

  it("keep the keys in the same order, so a review can read the two side by side", () => {
    expect(id).toEqual(en);
  });
});
