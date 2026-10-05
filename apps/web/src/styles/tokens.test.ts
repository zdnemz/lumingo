// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");

/** Every `--name: value;` declaration inside the first block that matches the selector. */
function declarations(selectorPattern: RegExp): Map<string, string> {
  const match = selectorPattern.exec(css);
  if (!match) throw new Error(`no block for ${selectorPattern}`);
  const open = css.indexOf("{", match.index);
  const close = css.indexOf("\n}", open);
  const body = css.slice(open + 1, close);
  const out = new Map<string, string>();
  for (const line of body.split(";")) {
    const found = /(--[\w-]+)\s*:\s*([^;]+)$/m.exec(line.trim());
    if (found?.[1] && found[2]) out.set(found[1], found[2].trim());
  }
  return out;
}

const primitives = declarations(/^:root\s*\{/m);

function resolve(theme: Map<string, string>, name: string): string | undefined {
  let value = theme.get(name);
  const seen = new Set<string>();
  while (value?.startsWith("var(")) {
    const inner = /var\((--[\w-]+)\)/.exec(value)?.[1];
    if (!inner || seen.has(inner)) return undefined;
    seen.add(inner);
    value = theme.get(inner) ?? primitives.get(inner);
  }
  return value;
}

function luminance(hex: string): number {
  const channels = [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r = 0, g = 0, b = 0] = channels.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [hi = 0, lo = 0] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const THEMES: Record<string, RegExp> = {
  night: /^:root,\s*\n:root\[data-theme="night"\]\s*\{/m,
  day: /^:root\[data-theme="day"\]\s*\{/m,
  forest: /^:root\[data-theme="forest"\]\s*\{/m,
  ember: /^:root\[data-theme="ember"\]\s*\{/m,
};

const REQUIRED = [
  "--color-bg", "--color-bg-deep", "--color-surface", "--color-surface-raised", "--color-border",
  "--color-shadow", "--color-text", "--color-text-muted", "--color-link", "--color-primary",
  "--color-primary-hover", "--color-primary-pressed", "--color-on-primary", "--color-success",
  "--color-danger", "--color-warning", "--color-info", "--color-skill-listening",
  "--color-skill-speaking", "--color-skill-reading", "--color-skill-writing", "--color-focus",
  "--color-selection", "--color-on-selection", "--bevel-light", "--bevel-dark", "--color-scanline",
  "--color-star",
];

describe.each(Object.entries(THEMES))("theme %s", (_name, selector) => {
  const theme = declarations(selector);
  const colour = (token: string): string => {
    const value = resolve(theme, token);
    if (!value?.startsWith("#")) throw new Error(`${token} is not a hex colour: ${value}`);
    return value;
  };

  it("defines every semantic token", () => {
    for (const token of REQUIRED) expect(theme.has(token), token).toBe(true);
  });

  it("keeps body text readable on every surface (WCAG AAA for the main pairs)", () => {
    expect(contrast(colour("--color-text"), colour("--color-bg"))).toBeGreaterThanOrEqual(7);
    expect(contrast(colour("--color-text"), colour("--color-surface"))).toBeGreaterThanOrEqual(7);
    expect(contrast(colour("--color-text"), colour("--color-surface-raised"))).toBeGreaterThanOrEqual(4.5);
    for (const surface of ["--color-bg", "--color-surface", "--color-surface-raised"]) {
      expect(contrast(colour("--color-text-muted"), colour(surface)), surface).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("keeps button text readable in every state", () => {
    for (const state of ["--color-primary", "--color-primary-hover", "--color-primary-pressed"]) {
      expect(contrast(colour("--color-on-primary"), colour(state)), state).toBeGreaterThanOrEqual(4.5);
    }
    expect(contrast(colour("--color-on-selection"), colour("--color-selection"))).toBeGreaterThanOrEqual(4.5);
  });

  it("keeps borders and the focus ring visible (3:1 for non-text)", () => {
    for (const surface of ["--color-bg", "--color-surface"]) {
      expect(contrast(colour("--color-border"), colour(surface)), `border on ${surface}`).toBeGreaterThanOrEqual(3);
      expect(contrast(colour("--color-focus"), colour(surface)), `focus on ${surface}`).toBeGreaterThanOrEqual(3);
    }
  });

  it("keeps status and link colours readable as text", () => {
    for (const token of ["--color-success", "--color-danger", "--color-warning", "--color-info", "--color-link"]) {
      for (const surface of ["--color-bg", "--color-surface"]) {
        expect(contrast(colour(token), colour(surface)), `${token} on ${surface}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("keeps the four skill colours visible on every surface", () => {
    for (const skill of ["listening", "speaking", "reading", "writing"]) {
      for (const surface of ["--color-bg", "--color-surface"]) {
        expect(contrast(colour(`--color-skill-${skill}`), colour(surface)), `${skill} on ${surface}`).toBeGreaterThanOrEqual(3);
      }
    }
  });
});

describe("theme list", () => {
  it("has one CSS block for each theme the preferences know", async () => {
    const { THEMES: known } = await import("@/state/preferences");
    expect([...known].sort()).toEqual(Object.keys(THEMES).sort());
  });
});
