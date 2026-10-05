import { describe, expect, it } from "vitest";
import { SPRITES } from "./data";
import { runsOf } from "./Sprite";

const KNOWN = new Set([".", "k", "g", "G", "w", "e", "b", "m", "x", "a"]);

describe("sprite data", () => {
  it("draws every sprite as a rectangle of known pixels", () => {
    for (const [name, def] of Object.entries(SPRITES)) {
      const width = def.rows[0]?.length ?? 0;
      expect(width, `${name} is empty`).toBeGreaterThan(0);
      for (const [index, row] of def.rows.entries()) {
        expect(row.length, `${name} row ${index}`).toBe(width);
        for (const pixel of row) {
          expect(KNOWN.has(pixel), `${name} row ${index} has unknown pixel "${pixel}"`).toBe(true);
        }
      }
    }
  });

  it("keeps every Lumi mood the same size", () => {
    const moods = Object.entries(SPRITES).filter(([name]) => name.startsWith("lumi-"));
    expect(moods.length).toBeGreaterThanOrEqual(6);
    for (const [name, def] of moods) {
      expect(def.rows.length, name).toBe(16);
      expect(def.rows[0]?.length, name).toBe(16);
    }
  });

  it("gives every icon a name that starts with its group", () => {
    for (const name of Object.keys(SPRITES)) {
      expect(/^(lumi|skill|icon)-[a-z]+$/.test(name), name).toBe(true);
    }
  });
});

describe("runsOf", () => {
  it("merges neighbours of one colour and skips empty pixels", () => {
    expect(runsOf(["..aab", "x..."])).toEqual([
      { x: 2, y: 0, length: 2, colour: "a" },
      { x: 4, y: 0, length: 1, colour: "b" },
      { x: 0, y: 1, length: 1, colour: "x" },
    ]);
  });
});
