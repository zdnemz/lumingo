// Fails when two tracked paths collide on a case-insensitive file system.
//
// CI's web job runs on Windows, where module resolution cannot tell
// `KeyGuide.tsx` from `keyGuide.ts`: TypeScript resolved `@/providers/KeyGuide`
// to `keyGuide.ts` and the build failed with TS2724 and TS1149. The checks on
// the machine that commits run on a case-sensitive file system, so only a check
// like this one can see the mistake before CI does. scripts/verify.sh (the
// pre-commit hook) and .github/workflows/ci.yml both run it.
//
// Two rules, the two ways a case-insensitive build is confused:
//   1. two entries in one directory whose names differ only in case. On Windows
//      they are one entry, so a checkout keeps one and the build reads the
//      other, or the checkout itself fails;
//   2. two module names in one directory that differ only in case, whatever
//      their extensions: `foo.ts` next to `foo.d.ts` is fine (same name, same
//      case), `Foo.tsx` next to `foo.ts` is not. Both files can exist on
//      Windows, because the extensions differ, but the resolver gives one
//      module two spellings and the type checker fails — the TS2724/TS1149
//      pair this script exists for.
import { execFileSync } from "node:child_process";

// The extensions a resolver tries for an import that carries none. `.d.ts`
// comes before `.ts` so that `foo.d.ts` is the module `foo`, not `foo.d`.
const IMPORTABLE = [".d.ts", ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"];

/** The module name of a file, or null when no import resolves the file by name. */
function moduleName(name) {
  const lower = name.toLowerCase();
  for (const extension of IMPORTABLE) {
    if (lower.endsWith(extension)) return name.slice(0, -extension.length);
  }
  return null;
}

/** The directory of a path, or "" for a path in the repository root. */
function parentOf(path) {
  const cut = path.lastIndexOf("/");
  return cut === -1 ? "" : path.slice(0, cut);
}

/** The file name of a path. */
function nameOf(path) {
  const cut = path.lastIndexOf("/");
  return cut === -1 ? path : path.slice(cut + 1);
}

function collisions(paths) {
  // One collision is identified by the set of lowercased names it involves, so
  // the same collision found by both rules is reported once. A collision that
  // is part of one already found (a pair inside a three-way collision) is
  // dropped, and a later, larger one replaces an earlier, smaller one.
  const problems = new Map();
  const report = (display, involved) => {
    const parts = new Set(involved.map((path) => path.toLowerCase()));
    for (const [key, problem] of problems) {
      if ([...parts].every((part) => problem.parts.has(part))) return;
      if ([...problem.parts].every((part) => parts.has(part))) problems.delete(key);
    }
    problems.set([...parts].sort().join("\u0000"), {
      text: display.join("  <->  "),
      parts,
    });
  };

  // Rule 1: every file and directory name, keyed by its parent and itself,
  // ignoring case. Two spellings under one key cannot both exist on Windows.
  const siblings = new Map();
  for (const path of paths) {
    const parts = path.split("/");
    for (let depth = 1; depth <= parts.length; depth += 1) {
      const parent = parts.slice(0, depth - 1).join("/");
      const name = parts[depth - 1];
      const component = parent === "" ? name : `${parent}/${name}`;
      // A trailing slash marks a directory, so the report reads clearly.
      const spelling = depth < parts.length ? `${component}/` : component;
      const key = `${parent.toLowerCase()}\u0000${name.toLowerCase()}`;
      const found = siblings.get(key) ?? new Set();
      found.add(spelling);
      siblings.set(key, found);
    }
  }
  for (const found of siblings.values()) {
    if (found.size > 1) {
      const spellings = [...found];
      report(spellings, spellings.map((spelling) => spelling.replace(/\/$/, "")));
    }
  }

  // Rule 2: one entry per module name in a directory, so `foo.ts` and `foo.d.ts`
  // (the same name, the same case) count as one and never collide with each
  // other. The files whose modules share a name are the collision.
  const modules = new Map();
  for (const path of paths) {
    const base = moduleName(nameOf(path));
    if (base === null) continue;
    const module = `${parentOf(path)}/${base}`;
    const key = module.toLowerCase();
    const found = modules.get(key) ?? new Map();
    if (!found.has(module)) found.set(module, path);
    modules.set(key, found);
  }
  for (const found of modules.values()) {
    if (found.size > 1) report([...found.values()], [...found.values()]);
  }

  return [...problems.values()].map(({ text }) => text);
}

/** Every tracked path. `git ls-files` prints what is committed, not what exists. */
function trackedFiles() {
  const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
  const listing = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" });
  return listing.split("\u0000").filter((path) => path !== "");
}

// Paths on the command line make the rules checkable by hand, for example
// `node scripts/check-case-collisions.mjs a/Foo.ts b/foo.ts`.
const paths = process.argv.length > 2 ? process.argv.slice(2) : trackedFiles();

const problems = collisions(paths);
if (problems.length > 0) {
  console.error("Case is ignored on Windows, where CI runs, so these paths cannot coexist:");
  for (const problem of problems) console.error(`  ${problem}`);
  console.error("Rename one of each pair so the names differ by more than case.");
  process.exit(1);
}
console.log(`Path case check passed (${paths.length} files).`);
