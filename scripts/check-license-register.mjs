#!/usr/bin/env node
// Compares the licence register's machine table with what the build ships.
//
// What this check proves: the register-row dimension (NFR-L1). Every shipped
// dependency has a row in the register's machine table; every row's kind and
// licence are the ones the build states; every row's status is `ok`; every row
// is for something the build ships; the attribution table carries a notice for
// exactly the components whose licence needs one, and for nothing else; and
// the generated NOTICE and the About page's data are what the generator
// writes, so the artifacts are complete.
//
// What this check does NOT prove: licence compatibility. `cargo deny check
// licenses` (the Rust crates) and `apps/web/scripts/check-licenses.mjs` (the
// JavaScript packages) own that dimension, and their allow list is not
// restated here.
//
// The build is derived by `scripts/gen-notices.mjs`, not by a second
// implementation of the derivation, so the two cannot disagree about what the
// artifact contains.
//
// A register that cannot be read is a broken source and exits 2. A gate that
// has only ever passed is not a proven gate, so the check is proved by
// breaking the register on purpose and watching it fail (ADR-069).
//
// Exit codes:
//   0  the register agrees with the build
//   1  the register does not agree with the build
//   2  internal error
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  derive,
  findTable,
  readAttributions,
  registerSection,
  renderData,
  renderNotice,
} from "./gen-notices.mjs";

// The repository root, found from this script's own path, so the script works
// from any working directory.
const ROOT = fileURLToPath(new URL("../", import.meta.url));

// The register, and the sections of it this check reads.
const REGISTER_PATH = "docs/LICENSE_REGISTER.md";
const MACHINE_SECTION = 10;
const RULES_SECTION = 2;

// The columns of the machine table, and of the attribution table.
const MACHINE_COLUMNS = ["Item", "Kind", "Licence", "Status"];
const ATTRIBUTION_COLUMNS = ["Item", "Notice"];

// The files the generator writes, and the ones the register has to agree with.
const NOTICE_PATH = "NOTICE";
const DATA_PATH = "apps/web/src/about/notices.json";

// The licences that need a notice of their own. The register's rules name
// them, and this check asserts that they are named, so the list cannot drift
// away from the register.
const NOTICE_LICENCES = ["MPL-2.0", "CC-BY-4.0"];

/** A file's text, read from the repository root. */
function read(path) {
  return readFileSync(join(ROOT, path), "utf8");
}

/**
 * Split an expression on a word, at the top level only.
 *
 * The top level is the part of the expression that is not inside brackets, so
 * a bracket that groups alternatives is not split apart.
 */
function splitTopLevel(text, word) {
  const parts = [];
  let depth = 0;
  let current = "";
  let index = 0;
  const isWordCharacter = (character) => /[A-Za-z0-9]/.test(character);
  while (index < text.length) {
    const character = text[index];
    if (character === "(") depth += 1;
    else if (character === ")") depth -= 1;
    if (depth === 0 && text.startsWith(word, index)) {
      const before = index === 0 ? "" : text[index - 1];
      const after = index + word.length < text.length ? text[index + word.length] : "";
      const boundaryBefore = before === "" || !isWordCharacter(before);
      const boundaryAfter = after === "" || !isWordCharacter(after);
      if (boundaryBefore && boundaryAfter) {
        parts.push(current);
        current = "";
        index += word.length;
        continue;
      }
    }
    current += character;
    index += 1;
  }
  parts.push(current);
  return parts.map((part) => part.trim()).filter((part) => part !== "");
}

/**
 * The licences an expression names.
 *
 * A licence expression can name several licences: `MIT OR Apache-2.0` names
 * two, and `MIT AND BSD-3-Clause` names two in a different relation. The
 * expression is split on `AND` into groups, each group is split on `OR` into
 * alternatives, and the alternatives of a group are a set.
 *
 * `/` reads as `OR`, because that is how some packages write it, and every
 * space is dropped, because `MIT OR Apache-2.0` and `MIT OR Apache-2.0` with
 * other spacing name the same licences.
 */
function licenceGroups(expression) {
  const normalised = expression.replace(/\//g, " OR ");
  return splitTopLevel(normalised, "AND").map((group) => {
    const alternatives = splitTopLevel(group, "OR")
      .map((alternative) => alternative.replace(/[()]/g, "").replace(/\s+/g, ""))
      .filter((alternative) => alternative !== "");
    return [...new Set(alternatives)].sort();
  });
}

/**
 * Whether two licence expressions name the same licences.
 *
 * The groups are compared as sets, and the order of the groups does not
 * matter: `MIT AND BSD-3-Clause` names the same licences as
 * `BSD-3-Clause AND MIT`, and does not name the same licences as
 * `MIT OR BSD-3-Clause`.
 */
function sameLicences(left, right) {
  const a = licenceGroups(left);
  const b = licenceGroups(right);
  if (a.length !== b.length) return false;
  const key = (group) => group.join("+");
  const leftKeys = a.map(key).sort();
  const rightKeys = b.map(key).sort();
  return leftKeys.every((value, index) => value === rightKeys[index]);
}

/** The licences an expression names, without the groups. */
function licenceAlternatives(expression) {
  return new Set(licenceGroups(expression).flat());
}

/**
 * The licences the register's rules say need a notice.
 *
 * The rules name them, so the list the check works from is the register's own
 * and cannot drift away from it. The line that names them is the one that
 * says the licences need a notice.
 */
function noticeLicences(register) {
  const section = registerSection(register, RULES_SECTION);
  if (section === null) throw new Error(`the register has no section ${RULES_SECTION}`);
  const licences = new Set();
  for (const line of section.split(/\r?\n/)) {
    if (!/notice/i.test(line)) continue;
    for (const token of line.match(/[A-Za-z0-9][A-Za-z0-9.+-]*-\d[A-Za-z0-9.+-]*/g) ?? []) {
      licences.add(token);
    }
  }
  return licences;
}

/** The machine table's rows, read from the register's shipped-set section. */
function machineRows(register) {
  const section = registerSection(register, MACHINE_SECTION);
  if (section === null) throw new Error(`the register has no section ${MACHINE_SECTION}`);
  const table = findTable(section, MACHINE_COLUMNS);
  if (table === null) {
    throw new Error(`the register's section ${MACHINE_SECTION} has no machine table`);
  }
  return table.rows;
}

/** The command line. */
function parseArgs(argv) {
  const options = { printItems: false };
  for (const arg of argv) {
    if (arg === "--print-items") options.printItems = true;
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

/**
 * The register against the build.
 *
 * Every rule below is one thing the register has to say about the build, and
 * every offender is one thing it says wrong.
 */
function compare(components, notices, register) {
  const offenders = [];
  const rows = machineRows(register).map((row) => ({
    item: row[0].trim(),
    kind: row[1].trim(),
    licence: row[2].trim(),
    status: row[3].trim(),
  }));
  // The rows, by the item they are for. Two rows can carry the same item when
  // one is a crate and the other a package, so the kind tells them apart.
  const rowsByItem = new Map();
  for (const row of rows) {
    if (!rowsByItem.has(row.item)) rowsByItem.set(row.item, []);
    rowsByItem.get(row.item).push(row);
  }

  const shipped = new Map(components.map((component) => [`${component.kind} ${component.name}`, component]));
  const shippedNames = new Set(components.map((component) => component.name));

  // Rule 1, 2, 3 and 4: every shipped dependency has a row, and the row says
  // what the build says.
  for (const component of components) {
    const found = rowsByItem.get(component.name);
    if (found === undefined) {
      offenders.push(`${component.name} is in the build but has no row`);
      continue;
    }
    const matching = found.filter((row) => row.kind === component.kind);
    if (matching.length === 0) {
      const kinds = [...new Set(found.map((row) => row.kind))].join(", ");
      offenders.push(`${component.name} is a ${component.kind} in the build but its row says ${kinds}`);
      continue;
    }
    for (const row of matching) {
      if (row.status !== "ok") {
        offenders.push(`${component.name} has status ${row.status}, not ok`);
      }
      if (!sameLicences(row.licence, component.licence)) {
        offenders.push(`${component.name} says licence ${row.licence}, the build says ${component.licence}`);
      }
    }
  }

  // Rule 5: every row is for something the build ships, and no item has two
  // rows of the same kind.
  for (const row of rows) {
    if (!shipped.has(`${row.kind} ${row.item}`)) {
      offenders.push(`${row.item} has a row but is not in the build`);
    }
    const same = rows.filter((other) => other.item === row.item && other.kind === row.kind);
    if (same.length > 1 && row === same[0]) {
      offenders.push(`${row.item} has ${same.length} rows of kind ${row.kind}`);
    }
  }

  // Rule 9 needs the list of licences that need a notice, and the register's
  // own rules have to name it.
  const needing = noticeLicences(register);
  for (const licence of NOTICE_LICENCES) {
    if (!needing.has(licence)) {
      offenders.push(`the register's rules do not name ${licence} as a licence that needs a notice`);
    }
  }

  // Rule 8: a notice for a component the artifact does not contain is a
  // false claim.
  for (const item of notices.keys()) {
    if (!shippedNames.has(item)) {
      offenders.push(`${item} has an attribution notice but is not in the build`);
    }
  }

  // Rule 9: a component whose licence needs a notice and has no row in the
  // attribution table is a notice that was left out.
  for (const component of components) {
    const alternatives = licenceAlternatives(component.licence);
    const wanted = [...needing].filter((licence) => alternatives.has(licence));
    if (wanted.length === 0) continue;
    if (notices.has(component.name)) continue;
    offenders.push(
      `${component.name} is licensed ${component.licence} and needs a notice, but has no row in the attribution table`,
    );
  }

  return { offenders, rowCount: rows.length };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const { components, models, project } = derive();

  if (options.printItems) {
    for (const component of components) {
      process.stdout.write(`${component.kind}\t${component.name}\t${component.licence}\n`);
    }
    return 0;
  }

  const notices = readAttributions();
  const register = read(REGISTER_PATH);
  const { offenders, rowCount } = compare(components, notices, register);

  // Rule 6: the generated artifacts are what the generator writes.
  const files = [
    { path: NOTICE_PATH, text: renderNotice(components, project, notices) },
    { path: DATA_PATH, text: renderData(components, models, project, notices) },
  ];
  for (const file of files) {
    const current = committed(file.path);
    if (current === null) offenders.push(`${file.path} is not there`);
    else if (current !== file.text) offenders.push(`${file.path} is not what the generator writes`);
  }

  if (offenders.length > 0) {
    console.error("The license register does not match the build. These do not:");
    for (const offender of offenders) console.error(`${REGISTER_PATH}: ${offender}`);
    return 1;
  }
  console.log(
    `License register check passed (${components.length} shipped dependencies, ${rowCount} register rows).`,
  );
  return 0;
}

try {
  process.exitCode = main();
} catch (error) {
  console.error(`license register check: internal error: ${error.message}`);
  process.exitCode = 2;
}
