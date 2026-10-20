#!/usr/bin/env node
// Writes the NOTICE, and the About page's data, from what the build ships.
//
// The release zip carries a NOTICE, and the About page lists the same set.
// Both are derived here, at run time, from the sources that decide what the
// artifact contains: the crates the release binary links, the JavaScript
// packages the static export is built from, the fonts the export serves, the
// models the manifest names and the files the release zip holds.
//
// A hand-written list goes stale the moment a dependency changes, and a stale
// NOTICE is worse than none: it names components the artifact does not
// contain, and misses the ones it does. So the list is generated, and the
// generated list is what the repository holds.
//
// Every fact below is read from its source. A source that cannot be read, or
// a derived list that comes out empty, throws: this script never writes an
// empty list, because a NOTICE that lists nothing is a false claim.
//
// The notices the build cannot prove by itself are read from the licence
// register: the register is the project's record, and the one place where
// wording is allowed. The build says which licences the shipped components
// carry; it cannot say what the MPL-2.0 and CC-BY-4.0 components ask the
// reader to be told. So the register carries those words and this script
// renders them under the component's own line.
//
// Exit codes:
//   0  the files were written, or --check found them already correct
//   1  --check found a committed file that differs from what it would write
//   2  internal error
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// The repository root, found from this script's own path, so the script works
// from any working directory.
const ROOT = fileURLToPath(new URL("../", import.meta.url));

// The files this script owns.
const NOTICE_PATH = "NOTICE";
const DATA_PATH = "apps/web/src/about/notices.json";

// The licence register, and the section of it that holds the shipped set.
const REGISTER_PATH = "docs/LICENSE_REGISTER.md";
const REGISTER_SECTION = 10;

// The width the NOTICE's prose is wrapped at. A notice under a component's
// own line is indented by two spaces, so its text is wrapped two columns
// narrower and the whole line stays inside the same measure.
const NOTICE_WIDTH = 68;

// The target the release artifact is built for.
//
// The target is pinned, and it has to be: the release artifact is a Windows
// x64 binary, so the dependency set is the one cargo resolves for that
// target, on this machine and on the Windows runner alike. Without the pin
// the same command gives a different set per host (on Linux the openssl
// stack replaces schannel and the windows-* crates appear).
const CARGO_TARGET = "x86_64-pc-windows-msvc";

// The kinds, in the order the register and the NOTICE list them.
const KINDS = ["crate", "package", "font", "model", "release"];

/** A file's text, read from the repository root. */
function read(path) {
  return readFileSync(join(ROOT, path), "utf8");
}

/**
 * A command's output, run from the repository root.
 *
 * The commands are run from the root on purpose: `cargo tree` reads the
 * workspace, and `pnpm --dir apps/web` reads the lockfile of the web app.
 *
 * Whether the command goes through a shell is the caller's decision, because
 * the two commands need different things. `cargo` is a real executable and
 * must not go through a shell: the format string holds a space, and a shell
 * would split it into two arguments on Windows. `pnpm` is a `.cmd` file on
 * Windows and cannot be spawned without a shell, so it takes one; none of
 * its arguments holds a space, so the shell has nothing to split.
 */
function run(command, args, what, options = {}) {
  try {
    return execFileSync(command, args, {
      cwd: ROOT,
      encoding: "utf8",
      shell: options.shell === true,
      stdio: ["ignore", "pipe", "inherit"],
    });
  } catch (error) {
    throw new Error(`cannot run ${what} (${command} ${args.join(" ")}): ${error.message}`);
  }
}

/**
 * One line of the cargo tree, read as a crate.
 *
 * The line is `<name> v<version> [<marker>] <licence>`, where the marker is
 * either the path of a workspace crate or the word `proc-macro`, and where a
 * trailing ` (*)` marks a crate that appears more than once.
 *
 * The marker is read before anything is stripped: a marker that is a path
 * points inside the repository, so the crate is Lumingo's own code, and it
 * keeps the project's licence instead of a third-party one. The name is the
 * text before the first ` v`; the rest of the line is the licence, verbatim.
 */
function parseCrateLine(line) {
  const body = line.trim();
  if (body === "") return null;
  const separator = body.indexOf(" v");
  if (separator <= 0) throw new Error(`cannot read the crate line: ${body}`);
  const name = body.slice(0, separator);
  const rest = body.slice(separator + 1);
  if (!/^v\d/.test(rest)) throw new Error(`cannot read the version of ${name}: ${body}`);
  const space = rest.indexOf(" ");
  if (space === -1) throw new Error(`the crate ${name} states no licence`);
  const tail = rest.slice(space + 1);

  let marker = null;
  if (tail.startsWith("(")) {
    const close = tail.indexOf(")");
    if (close !== -1) marker = tail.slice(1, close);
  }
  // A path marker means a workspace crate: this project's own code.
  if (marker !== null && marker !== "proc-macro" && (marker.includes("/") || marker.includes("\\"))) {
    return { name, workspace: true, licence: "" };
  }

  let licence = tail;
  if (marker === "proc-macro") licence = tail.slice(tail.indexOf(")") + 1).trim();
  licence = licence.replace(/\s+\(\*\)\s*$/, "").trim();
  if (licence === "") throw new Error(`the crate ${name} states no licence`);
  return { name, workspace: false, licence };
}

/**
 * The third-party crates in the release build.
 *
 * `cargo tree` resolves from the lockfile; it is not a build, so it compiles
 * nothing and needs no network. The workspace crates are dropped: they are
 * Lumingo's own code and they carry the project's licence, which the NOTICE
 * states on its own.
 */
function deriveCrates() {
  // No shell, on any platform: the format string holds a space, and cargo is
  // a real executable that takes it as one argument.
  const output = run(
    "cargo",
    ["tree", "--edges", "normal,build", "--prefix", "none", "--format", "{p} {l}", "--target", CARGO_TARGET],
    "the cargo tree",
    { shell: false },
  );
  const crates = new Map();
  for (const line of output.split(/\r?\n/)) {
    const parsed = parseCrateLine(line);
    if (parsed === null || parsed.workspace) continue;
    crates.set(parsed.name, parsed.licence);
  }
  return crates;
}

/**
 * The JavaScript packages the static export is built from.
 *
 * The `@next/swc-*` family is the Next.js compiler: it compiles the app and
 * never reaches the export, and it is the only part of the closure that
 * differs per platform. The rest is the same on every machine.
 */
function derivePackages() {
  // A shell, on Windows only: pnpm is a `.cmd` file there and cannot be
  // spawned without one. None of these arguments holds a space, so the shell
  // has nothing to split.
  const output = run(
    "pnpm",
    ["--dir", "apps/web", "licenses", "list", "--prod", "--json"],
    "the pnpm licence list",
    { shell: process.platform === "win32" },
  );
  const listing = JSON.parse(output.slice(output.indexOf("{"), output.lastIndexOf("}") + 1));
  const packages = new Map();
  for (const group of Object.values(listing)) {
    for (const pkg of group) {
      if (pkg.name.startsWith("@next/swc-")) continue;
      packages.set(pkg.name, pkg.license);
    }
  }
  return packages;
}

/**
 * The fonts the export serves, with the text of their licence.
 *
 * The stylesheet names the family and the file; the licence text sits next
 * to the fonts in `public/fonts/licenses/`, named after the family with
 * every non-alphanumeric character removed.
 */
function deriveFonts() {
  const css = read("apps/web/src/styles/fonts.css");
  const families = new Map();
  for (const block of css.matchAll(/@font-face\s*\{([\s\S]*?)\}/g)) {
    const family = block[1].match(/font-family:\s*"([^"]+)"/)?.[1];
    const file = block[1].match(/url\("\/fonts\/([^"]+)"\)/)?.[1];
    if (!family || !file) continue;
    if (!families.has(family)) families.set(family, []);
    families.get(family).push(file);
  }
  const fonts = [];
  for (const [family, files] of families) {
    for (const file of files) {
      if (!existsSync(join(ROOT, "apps/web/public/fonts", file))) {
        throw new Error(`the font ${family} names a file that is not bundled: ${file}`);
      }
    }
    const stem = family.replace(/[^a-zA-Z0-9]/g, "");
    const textPath = `apps/web/public/fonts/licenses/${stem}-OFL.txt`;
    if (!existsSync(join(ROOT, textPath))) {
      throw new Error(`the font ${family} has no licence text: ${textPath}`);
    }
    const attribution = read(textPath).split(/\r?\n/)[0].trim();
    if (attribution === "") throw new Error(`the licence text of the font ${family} has no first line`);
    fonts.push({ name: family, licence: "OFL-1.1", attribution });
  }
  return fonts;
}

/** A string field of a manifest block, or an empty string when it has none. */
function manifestField(block, key) {
  const found = block.match(new RegExp(`^\\s*${key}\\s*=\\s*"([^"]*)"`, "m"));
  return found ? found[1] : "";
}

/** The text of an array field of a manifest block, brackets included. */
function manifestArray(block, key) {
  const at = block.search(new RegExp(`^\\s*${key}\\s*=\\s*\\[`, "m"));
  if (at === -1) return "";
  const open = block.indexOf("[", at);
  let depth = 0;
  for (let i = open; i < block.length; i += 1) {
    if (block[i] === "[") depth += 1;
    else if (block[i] === "]") {
      depth -= 1;
      if (depth === 0) return block.slice(open, i + 1);
    }
  }
  return "";
}

/**
 * The candidate models, and whether each one can be downloaded.
 *
 * The manifest is read as text, the way the model manager reads it: an entry
 * is a `[[model]]` block, and the downloader refuses an entry whose file
 * list is empty, before it makes a request. An entry that passes is part of
 * a release; an entry that does not is a candidate, and the About page says
 * so.
 */
function deriveModels() {
  const manifest = read("models/manifest.toml");
  const models = [];
  for (const block of manifest.split(/^\[\[model\]\]$/m).slice(1)) {
    const id = manifestField(block, "id");
    if (id === "") continue;
    const files = manifestArray(block, "files");
    models.push({
      id,
      licence: manifestField(block, "license"),
      licenceUrl: manifestField(block, "license_url"),
      downloadable: files.replace(/[\s\[\]]/g, "") !== "",
    });
  }
  return models;
}

/**
 * The files the release zip holds.
 *
 * The list is the one the packaging script builds the archive from, read the
 * way the document check reads it.
 */
function deriveReleaseMembers() {
  const packaging = read("scripts/package-release.mjs");
  const block = packaging.match(/const members = \[([\s\S]*?)\];/);
  if (!block) throw new Error("cannot read the release members from scripts/package-release.mjs");
  return [...block[1].matchAll(/name:\s*['"]([^'"]+)['"]/g)].map((match) => match[1]);
}

/**
 * The project's own two licences.
 *
 * The code licence is the one the workspace claims, and the LICENSE file has
 * to be that licence's text. The content licence is the first line of
 * LICENSE-CONTENT, which names it.
 */
function deriveProjectLicences() {
  const workspace = read("Cargo.toml");
  const code = workspace.match(/^license\s*=\s*"([^"]+)"/m)?.[1];
  if (!code) throw new Error("cannot read the project licence from Cargo.toml");
  const licence = read("LICENSE");
  if (!/apache license/i.test(licence)) throw new Error("the LICENSE file does not hold the Apache licence");
  const content = read("LICENSE-CONTENT").split(/\r?\n/)[0].trim();
  if (content === "") throw new Error("the LICENSE-CONTENT file has no first line");
  return { code, content };
}

/**
 * Every shipped component, sorted by kind and then by name.
 *
 * The sort is the register's order: the crates, then the packages, then the
 * fonts, then the models, then the files of the release.
 */
function deriveComponents(crates, packages, fonts, models, members, project) {
  const components = [];
  for (const [name, licence] of crates) components.push({ name, kind: "crate", licence });
  for (const [name, licence] of packages) components.push({ name, kind: "package", licence });
  for (const font of fonts) {
    components.push({ name: font.name, kind: "font", licence: font.licence, attribution: font.attribution });
  }
  for (const model of models) {
    if (model.downloadable) components.push({ name: model.id, kind: "model", licence: model.licence });
  }
  for (const member of members) components.push({ name: member, kind: "release", licence: project.code });
  return components.sort(
    (a, b) => KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind) || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0),
  );
}

/** The components of one kind. */
function ofKind(components, kind) {
  return components.filter((component) => component.kind === kind);
}

/**
 * A section of the register, from its heading to the next one.
 *
 * The section is found by its number, because the numbers are what the rest
 * of the repository refers to. A register without that section returns null.
 */
export function registerSection(register, number) {
  const lines = register.split(/\r?\n/);
  const start = lines.findIndex((line) => new RegExp(`^## ${number}\\.\\s`).test(line));
  if (start === -1) return null;
  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    if (/^##\s/.test(lines[index])) {
      end = index;
      break;
    }
  }
  return lines.slice(start + 1, end).join("\n");
}

/** The cells of a markdown table row, with the escaped pipes unescaped. */
function tableCells(line) {
  const body = line.trim();
  const inner = body.slice(1, body.endsWith("|") ? -1 : undefined);
  const cells = [];
  let current = "";
  let escaped = false;
  for (const character of inner) {
    if (escaped) {
      current += character;
      escaped = false;
    } else if (character === "\\") {
      current += character;
      escaped = true;
    } else if (character === "|") {
      cells.push(current.trim());
      current = "";
    } else {
      current += character;
    }
  }
  cells.push(current.trim());
  return cells.map((cell) => cell.replace(/\\\|/g, "|"));
}

/** Whether a line is a markdown table row. */
function isTableRow(line) {
  return /^\s*\|.*\|\s*$/.test(line);
}

/** Whether a row of cells is the separator under a header. */
function isSeparator(cells) {
  return cells.length > 0 && cells.every((cell) => /^:?-{2,}:?$/.test(cell));
}

/**
 * A markdown table, found by its header row.
 *
 * The header names the columns, so two tables in the same section are told
 * apart by their columns and not by their position. A row that does not have
 * the header's number of cells is a broken table and throws: a table the
 * reader cannot read is a broken source, not a table with fewer rows.
 */
export function findTable(markdown, header) {
  const lines = markdown.split(/\r?\n/);
  const wanted = header.map((name) => name.trim().toLowerCase());
  for (let index = 0; index < lines.length; index += 1) {
    if (!isTableRow(lines[index])) continue;
    const cells = tableCells(lines[index]);
    if (cells.length !== wanted.length) continue;
    if (!cells.every((cell, column) => cell.toLowerCase() === wanted[column])) continue;
    const rows = [];
    for (let row = index + 1; row < lines.length; row += 1) {
      if (!isTableRow(lines[row])) break;
      const rowCells = tableCells(lines[row]);
      if (isSeparator(rowCells)) continue;
      if (rowCells.length !== wanted.length) {
        throw new Error(`the table with the columns ${header.join(", ")} has a row with ${rowCells.length} cells`);
      }
      rows.push(rowCells);
    }
    return { header, rows };
  }
  return null;
}

/**
 * The key a shipped component is looked up by: its kind and its name.
 *
 * The name alone is not enough: `semver` is in the shipped set twice, once as
 * a crate and once as a package, so a table that is keyed on the name alone
 * cannot say which of the two a row is for.
 */
export function shippedKey(kind, name) {
  return JSON.stringify([kind, name]);
}

/**
 * The register's attribution table: the notices the build cannot prove.
 *
 * The table is the one place where wording is allowed, so it is read from the
 * register and not invented here. A table that is missing, that cannot be
 * read, or that is empty throws: the shipped set holds MPL-2.0 components,
 * so an empty table is a broken source, not a pass.
 *
 * A row is keyed on the item's name and its kind, the way the machine table
 * is: one row per shipped item of one kind.
 */
export function readAttributions() {
  const register = read(REGISTER_PATH);
  const section = registerSection(register, REGISTER_SECTION);
  if (section === null) throw new Error(`the register has no section ${REGISTER_SECTION}`);
  const table = findTable(section, ["Item", "Kind", "Notice"]);
  if (table === null) {
    throw new Error(`the register's section ${REGISTER_SECTION} has no attribution table`);
  }
  const rows = [];
  for (const row of table.rows) {
    const item = row[0].trim();
    const kind = row[1].trim();
    const notice = row[2].trim();
    if (item === "") throw new Error("the attribution table has a row with no item");
    if (kind === "") throw new Error(`the attribution row for ${item} has no kind`);
    if (notice === "") throw new Error(`the attribution row for ${item} has no notice`);
    rows.push({ name: item, kind, notice });
  }
  if (rows.length === 0) throw new Error("the register's attribution table is empty");
  return rows;
}

/**
 * The notices of the attribution table, and what is wrong with the rest.
 *
 * A row is matched to a shipped component by its name and its kind. A row
 * that matches no component is a notice that goes nowhere, and two rows for
 * the same component are two notices for one component: both are reported
 * here, and the caller decides what to do about them. The generator refuses
 * to write a NOTICE from a table in that state; the check names the rows.
 */
export function resolveNotices(components, rows) {
  const shipped = new Set(components.map((component) => shippedKey(component.kind, component.name)));
  const notices = new Map();
  const unmatched = [];
  const duplicates = [];
  for (const row of rows) {
    const key = shippedKey(row.kind, row.name);
    if (!shipped.has(key)) {
      unmatched.push(row);
      continue;
    }
    if (notices.has(key)) {
      duplicates.push(row);
      continue;
    }
    notices.set(key, row.notice);
  }
  return { notices, unmatched, duplicates };
}

/**
 * A paragraph, wrapped to a width.
 *
 * The words are the ones the register carries; only the line breaks are
 * added here.
 */
function wrap(text, width) {
  const words = text.split(/\s+/).filter((word) => word !== "");
  const lines = [];
  let line = "";
  for (const word of words) {
    if (line === "") {
      line = word;
    } else if (`${line} ${word}`.length <= width) {
      line = `${line} ${word}`;
    } else {
      lines.push(line);
      line = word;
    }
  }
  if (line !== "") lines.push(line);
  return lines;
}

/**
 * A notice, indented under the component's own line.
 *
 * The text is the register's, verbatim; the indent and the line breaks are
 * the NOTICE's.
 */
function noticeLines(notice) {
  if (notice === undefined) return [];
  return wrap(notice, NOTICE_WIDTH - 2).map((line) => `  ${line}`);
}

/**
 * The NOTICE.
 *
 * The file states the project's own two licences, then lists what the build
 * ships, one line per component. It names no directory the release zip does
 * not hold, and no component the artifact does not contain.
 *
 * A component the register's attribution table names gets its notice on an
 * indented line directly beneath its own line.
 */
function renderNotice(components, project, notices) {
  const lines = [];
  lines.push("Lumingo");
  lines.push("Copyright 2026 zdnemz and Lumingo contributors");
  lines.push("");
  lines.push("This product includes software and content developed for the Lumingo project.");
  lines.push("");
  lines.push(`- Source code is licensed under ${project.code} (see LICENSE).`);
  lines.push("- Curriculum content written for this project is licensed under the");
  lines.push(`  ${project.content} license`);
  lines.push("  (see LICENSE-CONTENT).");
  lines.push("");
  lines.push("Third-party components keep their own licenses. The list below is");
  lines.push("generated from this build, so it names exactly what this release");
  lines.push("contains.");
  lines.push("");
  lines.push(`Rust crates in this build (${ofKind(components, "crate").length}):`);
  lines.push("");
  for (const crate of ofKind(components, "crate")) {
    lines.push(`- ${crate.name} — ${crate.licence}`);
    lines.push(...noticeLines(notices.get(shippedKey(crate.kind, crate.name))));
  }
  lines.push("");
  lines.push(`JavaScript packages in this build (${ofKind(components, "package").length}):`);
  lines.push("");
  for (const pkg of ofKind(components, "package")) {
    lines.push(`- ${pkg.name} — ${pkg.licence}`);
    lines.push(...noticeLines(notices.get(shippedKey(pkg.kind, pkg.name))));
  }
  lines.push("");
  lines.push(
    "Bundled fonts (SIL Open Font License 1.1; the license texts are served with the fonts from /fonts/licenses/):",
  );
  lines.push("");
  for (const font of ofKind(components, "font")) {
    lines.push(`- ${font.name} — ${font.licence} — ${font.attribution}`);
    lines.push(...noticeLines(notices.get(shippedKey(font.kind, font.name))));
  }
  lines.push("");
  lines.push("Speech models:");
  lines.push("");
  const models = ofKind(components, "model");
  if (models.length === 0) {
    lines.push("No speech model is part of this release. Every model is downloaded");
    lines.push("by the user's own machine, under its own terms, once it has been");
    lines.push("checked at the source; none of them can be downloaded yet.");
  } else {
    for (const model of models) {
      lines.push(`- ${model.name} — ${model.licence}`);
      lines.push(...noticeLines(notices.get(shippedKey(model.kind, model.name))));
    }
  }
  lines.push("");
  lines.push("Files in this release:");
  lines.push("");
  for (const member of ofKind(components, "release")) {
    lines.push(`- ${member.name} — ${member.licence}`);
    lines.push(...noticeLines(notices.get(shippedKey(member.kind, member.name))));
  }
  lines.push("");
  return lines.join("\n");
}

/**
 * The data the About page renders.
 *
 * It is data, not prose: the page imports it at build time, so the file adds
 * no server feature and no file the release zip does not already hold.
 *
 * The `attribution` field carries the notice of every component the
 * register's attribution table names, and the font's own OFL line. It is the
 * notice verbatim: this is JSON, not a letter.
 */
function renderData(components, models, project, notices) {
  return `${JSON.stringify(
    {
      project: { code: project.code, content: project.content },
      components: components.map((component) => {
        const attribution = notices.get(shippedKey(component.kind, component.name)) ?? component.attribution;
        return attribution === undefined
          ? { name: component.name, kind: component.kind, licence: component.licence }
          : {
              name: component.name,
              kind: component.kind,
              licence: component.licence,
              attribution,
            };
      }),
      models: models.map((model) => ({
        id: model.id,
        licence: model.licence,
        licenceUrl: model.licenceUrl,
        downloadable: model.downloadable,
      })),
    },
    null,
    2,
  )}\n`;
}

/** A markdown table cell: a pipe inside the cell would end the cell. */
function cell(text) {
  return text.replace(/\|/g, "\\|");
}

/**
 * The machine table's rows: one row per shipped item.
 *
 * A dependency that enters the build gets one new row; a dependency that
 * leaves gets its row deleted. The licence in a row is the licence the
 * source states, not a paraphrase of it. The status is `ok`: the item's
 * licence is known and compatible, which is what a shipped item is.
 */
function renderRows(components) {
  return components
    .map(
      (component) =>
        `| ${cell(component.name)} | ${component.kind} | ${cell(component.licence)} | ok |`,
    )
    .join("\n");
}

/** Everything the file is built from, read once. */
function derive() {
  const crates = deriveCrates();
  const packages = derivePackages();
  const fonts = deriveFonts();
  const models = deriveModels();
  const members = deriveReleaseMembers();
  const project = deriveProjectLicences();

  if (crates.size === 0) throw new Error("no crate came out of the cargo tree");
  if (packages.size === 0) throw new Error("no JavaScript package came out of the licence list");
  if (fonts.length === 0) throw new Error("no font came out of the stylesheet");
  if (models.length === 0) throw new Error("no model came out of the manifest");
  if (members.length === 0) throw new Error("no release member came out of the packaging script");

  const components = deriveComponents(crates, packages, fonts, models, members, project);
  if (components.length === 0) throw new Error("no shipped component was derived");
  return { components, models, project };
}

/** The command line. */
function parseArgs(argv) {
  const options = { check: false, print: false };
  for (const arg of argv) {
    if (arg === "--check") options.check = true;
    else if (arg === "--print") options.print = true;
    else throw new Error(`unknown argument: ${arg}`);
  }
  return options;
}

/** The committed text of a file, or null when the file is not there. */
function committed(path) {
  const full = join(ROOT, path);
  if (!existsSync(full)) return null;
  return readFileSync(full, "utf8");
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const { components, models, project } = derive();

  if (options.print) {
    process.stdout.write(`${renderRows(components)}\n`);
    return 0;
  }

  const rows = readAttributions();
  const { notices, unmatched, duplicates } = resolveNotices(components, rows);
  if (unmatched.length > 0) {
    const row = unmatched[0];
    throw new Error(
      `the attribution row for ${row.name} as a ${row.kind} names nothing the build ships`,
    );
  }
  if (duplicates.length > 0) {
    const row = duplicates[0];
    throw new Error(`the attribution table has two rows for ${row.name} as a ${row.kind}`);
  }
  const files = [
    { path: NOTICE_PATH, text: renderNotice(components, project, notices) },
    { path: DATA_PATH, text: renderData(components, models, project, notices) },
  ];

  if (options.check) {
    const stale = [];
    for (const file of files) {
      const current = committed(file.path);
      if (current === null) stale.push(`${file.path}: the file is not there`);
      else if (current !== file.text) stale.push(`${file.path}: the committed file differs from what this script writes`);
    }
    if (stale.length > 0) {
      console.error("The notice files must be what the build generates. These are not:");
      for (const line of stale) console.error(`  ${line}`);
      console.error("Run `node scripts/gen-notices.mjs` and commit what it writes.");
      return 1;
    }
    console.log(`Notice check passed (${components.length} components).`);
    return 0;
  }

  for (const file of files) {
    mkdirSync(dirname(join(ROOT, file.path)), { recursive: true });
    writeFileSync(join(ROOT, file.path), file.text, "utf8");
    console.log(file.path);
  }
  return 0;
}

// The derivation and the renderers are exported, so the register check reads
// the build the same way this script does and the two cannot disagree about
// what the artifact contains.
export { derive, renderData, renderNotice, renderRows };

// This script runs only when it is the entry point. The check imports it, and
// an import must not write the files.
const invokedDirectly =
  process.argv[1] !== undefined && pathToFileURL(resolve(process.argv[1])).href === import.meta.url;

if (invokedDirectly) {
  try {
    process.exitCode = main();
  } catch (error) {
    console.error(`gen-notices: internal error: ${error.message}`);
    process.exitCode = 2;
  }
}
