#!/usr/bin/env node
// Fails the build the moment the web export needs a Node server.
//
// The web app is a static export: `apps/web/next.config.ts` sets
// `output: "export"`, and the Rust server embeds the exported `apps/web/out`.
// The export is served by that server, so nothing in the web app may need a
// Node server at run time: no route handlers, no middleware, no proxy, no
// server actions, no `.server` modules, no rewrites, no redirects and no
// image optimiser.
//
// What this script IS: a static source scan for those shapes. It reads the
// files and reports the ones that match.
//
// What this script is NOT: a proof that the export never needs a Node server.
// It does not build the app, it does not run it, and it cannot know what a
// dependency does at build time or what a file reaches at run time. It is a
// tripwire for the obvious shapes, not a guarantee. A clean run means this
// scan found nothing — read it that way.
//
// Exit codes:
//   0  clean
//   1  violations found (one line per offender on stderr, `<path>: <what>`)
//   2  internal error
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

// The repository root, found from this script's own path, so the guard works
// from any working directory.
const ROOT = fileURLToPath(new URL("../", import.meta.url));

const WEB = join(ROOT, "apps", "web");
const WEB_SRC = join(WEB, "src");

// A route handler, a middleware and a proxy are all things Next.js serves at
// run time. A static export cannot serve any of them.
const FORBIDDEN_NAMES = new Set([
  "route.ts",
  "route.tsx",
  "route.js",
  "route.jsx",
  "middleware.ts",
  "middleware.tsx",
  "middleware.js",
  "middleware.jsx",
  "proxy.ts",
  "proxy.tsx",
  "proxy.js",
  "proxy.jsx",
]);

// A `.server` module is excluded from the client bundle by Next.js, so it only
// ever runs on a Node server.
const FORBIDDEN_SUFFIXES = [".server.ts", ".server.tsx", ".server.js", ".server.jsx"];

// Dependency and build output directories: not source, and a next.config or a
// node_modules copy of one would only produce false alarms.
const SKIPPED_DIRECTORIES = new Set(["node_modules", ".next", "out", ".git", ".turbo"]);

/** The path as it is printed: relative to the repository root, with `/`. */
function displayPath(path) {
  return relative(ROOT, path).split(sep).join("/");
}

/** Every file under `directory`, depth first, skipping dependency directories. */
function filesUnder(directory) {
  let entries;
  try {
    entries = readdirSync(directory, { withFileTypes: true });
  } catch (error) {
    throw new Error(`cannot read ${displayPath(directory)}: ${error.message}`);
  }
  const found = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (SKIPPED_DIRECTORIES.has(entry.name)) continue;
      found.push(...filesUnder(path));
    } else if (entry.isFile()) {
      found.push(path);
    }
  }
  return found;
}

/** True when the file holds bytes that are not text (an image, say). */
function isBinary(bytes) {
  const limit = Math.min(bytes.length, 8000);
  for (let index = 0; index < limit; index += 1) {
    if (bytes[index] === 0) return true;
  }
  return false;
}

/** True when a line of the file starts the `"use server"` directive. */
function startsServerAction(text) {
  return text.split(/\r?\n/).some((line) => {
    const trimmed = line.trim();
    return trimmed.startsWith('"use server"') || trimmed.startsWith("'use server'");
  });
}

/** What a forbidden file name is, for the report. */
function kindOf(name) {
  const stem = name.replace(/\.(ts|tsx|js|jsx)$/, "");
  if (stem === "route") return "a route handler";
  if (stem === "middleware") return "middleware";
  return "a proxy";
}

/** The violations in `apps/web/src`. */
function scanSource() {
  const violations = [];
  for (const path of filesUnder(WEB_SRC)) {
    const name = basename(path);
    const shown = displayPath(path);

    if (FORBIDDEN_NAMES.has(name)) {
      violations.push(
        `${shown}: forbidden file name — ${kindOf(name)} needs a Node server, and the web build is a static export`,
      );
    }
    for (const suffix of FORBIDDEN_SUFFIXES) {
      if (name.endsWith(suffix)) {
        violations.push(
          `${shown}: forbidden file name suffix — a .server module needs a Node server, and the web build is a static export`,
        );
        break;
      }
    }

    const bytes = readFileSync(path);
    if (isBinary(bytes)) continue;
    if (startsServerAction(bytes.toString("utf8"))) {
      violations.push(
        `${shown}: forbidden directive — "use server" marks a server action, and a server action needs a Node server`,
      );
    }
  }
  return violations;
}

/** The violations in every `next.config.*` file under `apps/web`. */
function scanNextConfig() {
  const violations = [];
  for (const path of filesUnder(WEB)) {
    const name = basename(path);
    if (!/^next\.config\.[^./]+$/.test(name)) continue;
    const shown = displayPath(path);
    const text = readFileSync(path, "utf8");

    if (!/output\s*:\s*["']export["']/.test(text)) {
      violations.push(`${shown}: next.config must set output: "export" — the web build is a static export`);
    }
    if (/\b(rewrites|redirects)\s*:/.test(text)) {
      violations.push(
        `${shown}: next.config must not define rewrites or redirects — a static export has no server to serve them`,
      );
    }
    if (/\bimages\s*:/.test(text) && !/unoptimized\s*:\s*true/.test(text)) {
      violations.push(
        `${shown}: next.config sets images but not unoptimized: true — the image optimiser needs a Node server`,
      );
    }
  }
  return violations;
}

function main() {
  if (!existsSync(WEB_SRC)) {
    throw new Error(`missing directory: ${displayPath(WEB_SRC)}`);
  }
  const violations = [...scanSource(), ...scanNextConfig()];
  if (violations.length > 0) {
    console.error("The web export must not need a Node server. These need one:");
    for (const violation of violations) console.error(`  ${violation}`);
    console.error("Static export is the only build mode. Remove the server feature, or do not use one.");
    process.exitCode = 1;
    return;
  }
  console.log("Export guard passed: no server feature in apps/web/src, and the next.config is a static export.");
}

try {
  main();
} catch (error) {
  console.error(`export guard: internal error: ${error.message}`);
  process.exitCode = 2;
}
