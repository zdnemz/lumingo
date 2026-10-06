import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// Vitest runs from apps/web.
const SRC = join(process.cwd(), "src") + "/";

/** Folders and files written for setup, settings, privacy, diagnostics, progress and the game panel. Every learner-facing word in them comes from the dictionaries. */
const CHECKED = [
  "onboarding",
  "settings",
  "providers",
  "progress",
  "game/GamePanel.tsx",
  "app/progress",
  "app/onboarding",
  "app/settings",
  "components/ConnectionBanner.tsx",
  "components/ErrorBanner.tsx",
  "components/HardwareSummary.tsx",
  "components/PrivacyStatements.tsx",
  "components/UnavailableNote.tsx",
  "ui/StatusBanner.tsx",
  "ui/TextField.tsx",
];

/** Attributes whose text a learner or a screen reader reads. */
const TEXT_ATTRIBUTES = new Set(["aria-label", "aria-description", "aria-placeholder", "aria-roledescription", "placeholder", "title", "alt", "label", "legend", "hint"]);

function files(path: string): string[] {
  const full = join(SRC, path);
  if (statSync(full).isFile()) return [full];
  return readdirSync(full).flatMap((entry) => {
    const child = join(path, entry);
    return statSync(join(SRC, child)).isDirectory() ? files(child) : /\.tsx$/.test(entry) && !/\.test\.tsx$/.test(entry) ? [join(SRC, child)] : [];
  });
}

function hasLetters(text: string): boolean {
  return /\p{L}/u.test(text);
}

/** Finds literal words in JSX: text between tags, `{"text"}`, and text-bearing attributes. */
function scan(name: string, text: string): string[] {
  const source = ts.createSourceFile(name, text, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
  const found: string[] = [];
  const at = (node: ts.Node, word: string) => {
    const { line } = source.getLineAndCharacterOfPosition(node.getStart());
    found.push(`${name}:${line + 1} "${word.trim()}"`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isJsxText(node) && hasLetters(node.text)) at(node, node.text);
    if (ts.isJsxExpression(node) && node.expression && (ts.isStringLiteral(node.expression) || ts.isNoSubstitutionTemplateLiteral(node.expression))) {
      if (hasLetters(node.expression.text) && !ts.isJsxAttribute(node.parent)) at(node, node.expression.text);
    }
    if (ts.isJsxAttribute(node) && ts.isIdentifier(node.name) || (ts.isJsxAttribute(node) && ts.isJsxNamespacedName(node.name))) {
      const name = ts.isIdentifier(node.name) ? node.name.text : node.name.getText();
      const value = node.initializer;
      if (TEXT_ATTRIBUTES.has(name) && value && ts.isStringLiteral(value) && hasLetters(value.text)) at(node, `${name}=${value.text}`);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

describe("hard-coded user-facing text", () => {
  const all = CHECKED.flatMap(files);

  it("looks at the screens it is meant to look at", () => {
    expect(all.length).toBeGreaterThan(15);
  });

  it("is not in any setup, settings, privacy or diagnostics component", () => {
    expect(all.flatMap((file) => scan(file.replace(SRC, ""), readFileSync(file, "utf8")))).toEqual([]);
  });

  it("finds a literal word in text, in an expression and in a text attribute, and ignores symbols and class names", () => {
    const bad = scan("x.tsx", 'const a = <p aria-label="Close" className="px-btn">Hello {"there"}<span>••</span></p>;');
    expect(bad).toHaveLength(3);
    expect(bad.join("\n")).toContain("Close");
    expect(bad.join("\n")).toContain("Hello");
    expect(bad.join("\n")).toContain("there");
    expect(scan("y.tsx", 'const b = <p className="px-btn" aria-label={t("x")}>{t("y")} <span>••</span></p>;')).toEqual([]);
  });
});
