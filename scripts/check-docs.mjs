#!/usr/bin/env node
// Fails when the release documents stop matching the code.
//
// The six documents under check — docs/PRIVACY.md, docs/MODELS.md,
// docs/LIMITATIONS.md, docs/INSTALL.md, docs/RELEASE_NOTES.md and
// CONTRIBUTING.md — state facts about the program: where the data folder is,
// which hosts a download starts from, what the release zip holds, which names
// a provider needs. A document that drifts from the code is a document that
// lies, and a learner reads it before the program ever runs.
//
// What this script IS: a comparison. It reads the documents, reads the code
// the facts come from, and reports where the two disagree. Every fact is
// derived from its source and never copied into this file, so a change in the
// code moves the check with it.
//
// What this script is NOT: a proof that the documents are true. It does not
// run the program, it does not build the web app, and it cannot know what a
// dependency does at run time. It reads files and compares strings. A clean
// run means this scan found nothing — read it that way.
//
// Exit codes:
//   0  clean
//   1  violations found (one line per offender on stderr, `<path>: <what>`)
//   2  internal error
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// The repository root, found from this script's own path, so the guard works
// from any working directory.
const ROOT = fileURLToPath(new URL("../", import.meta.url));

// The documents, in the order they are reported.
const DOCUMENTS = [
  "docs/PRIVACY.md",
  "docs/MODELS.md",
  "docs/LIMITATIONS.md",
  "docs/INSTALL.md",
  "docs/RELEASE_NOTES.md",
  "CONTRIBUTING.md",
];

// The headings each document must carry, in the order they must carry them.
const HEADINGS = {
  "docs/PRIVACY.md": [
    "# Privacy",
    "## What leaves your computer",
    "## What stays on your computer",
    "## Where your data is stored",
    "## Where your key is stored",
    "## Outbound hosts",
    "## What is not yet proven",
  ],
  "docs/MODELS.md": [
    "# Models",
    "## What the models are for",
    "## The candidate models",
    "## Download hosts",
    "## Why nothing can be downloaded yet",
    "## Licences",
  ],
  "docs/LIMITATIONS.md": [
    "# Limitations",
    "## What is not measured",
    "## The engines",
    "## The release build",
    "## The estimates",
    "## What would close the gap",
  ],
  "docs/INSTALL.md": [
    "# Installing Lumingo",
    "## What you need",
    "## Unpack the zip",
    "## Set up your provider",
    "## Start the program",
    "## Check the files",
    "## What is not in this build",
  ],
  "docs/RELEASE_NOTES.md": [
    "# Lumingo 1.0.0",
    "## What is in this release",
    "## Known limits",
    "## What has been checked",
    "## What has not been checked",
    "## Install",
  ],
  "CONTRIBUTING.md": [
    "# Contributing",
    "## What this repository is",
    "## Before you start",
    "## Build and test",
    "## Commit rules",
    "## Do not commit",
    "## Writing documentation",
  ],
};

// The wording the owner banned (PRD section 12.4). Each entry is a pattern
// and the wording it stands for, so the report names the wording and not a
// regex.
const BANNED_WORDS = [
  { pattern: /\bfully offline\b/i, wording: '"fully offline"' },
  { pattern: /\bfully local ai\b/i, wording: '"fully local AI"' },
  { pattern: /\bno cloud\b/i, wording: '"no cloud"' },
  { pattern: /\bprivacy-first\b/i, wording: '"privacy-first"' },
  { pattern: /\bcertif\w*/i, wording: '"certif*"' },
  { pattern: /\bofficial\b/i, wording: 'the word "official"' },
  { pattern: /\bequivalent to (ielts|toefl)\b/i, wording: '"equivalent to IELTS" or "equivalent to TOEFL"' },
  { pattern: /\baccurate pronunciation\b/i, wording: '"accurate pronunciation"' },
  { pattern: /\bencrypt\w*/i, wording: '"encrypt*"' },
  { pattern: /\bsecurely stored\b/i, wording: '"securely stored"' },
  { pattern: /\bweb ?site\b/i, wording: '"website" or "web site"' },
  { pattern: /\bonline app\b/i, wording: '"online app"' },
  { pattern: /\bhost(ed|ing)\b/i, wording: '"hosted" or "hosting"' },
  { pattern: /\bprivate\b/i, wording: 'the word "private"' },
];

// The tokens that mean a document was not finished.
const PLACEHOLDERS = [
  { pattern: /\bTODO\b/i, token: "TODO" },
  { pattern: /\bTBD\b/i, token: "TBD" },
  { pattern: /\[PRICE\]/i, token: "[PRICE]" },
  { pattern: /\blorem\b/i, token: "lorem" },
];

/** A file's text, read from the repository root. */
function read(path) {
  return readFileSync(join(ROOT, path), "utf8");
}

/** A regex that matches `text` literally. */
function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The first match of `pattern` in `text`, or a throw when there is none. */
function matches(text, pattern, what) {
  const found = text.match(pattern);
  if (!found) throw new Error(`cannot read ${what}`);
  return found;
}

/**
 * Every occurrence of a derived name in a text.
 *
 * A plain name (no `.` and no `/`) matches as a whole word, so `Lumingo` is
 * not found in `LumingoX`. A name that carries a `.` or a `/` is a family:
 * it matches the name plus any trailing word characters, so `lumingo.sqlite`
 * is found in `lumingo.sqlite` and `lumingo.sqliteX` is found as a family
 * member that is not the name — which is how a document that states a wrong
 * value is caught.
 */
function occurrences(text, value) {
  const escaped = escapeRegExp(value);
  const pattern = value.includes(".") || value.includes("/")
    ? new RegExp(`(?<!\\w)${escaped}\\w*`, "g")
    : new RegExp(`(?<!\\w)${escaped}(?!\\w)`, "g");
  const found = [];
  for (const match of text.matchAll(pattern)) {
    if (match[0].length > 0) found.push(match[0]);
  }
  return found;
}

/**
 * The facts, derived from the code.
 *
 * Every value here comes from a source file. Nothing is written down here
 * that the code does not already say.
 */
function deriveFacts() {
  const config = read("crates/app-core/src/config.rs");
  const security = read("apps/server/src/security.rs");
  const main = read("apps/server/src/main.rs");
  const inspector = read("crates/llm-client/src/inspector.rs");
  const packaging = read("scripts/package-release.mjs");
  const envTemplate = read(".env.example");
  const manifest = read("models/manifest.toml");
  const ownerRelease = read("docs/OWNER_RELEASE.md");
  const hook = read(".husky/pre-commit");

  // The data folder: the Windows branch of `data_dir_for` joins the folder
  // name to the per-user folder the environment names.
  const windows = matches(
    config,
    /Platform::Windows => \{\s*non_empty\("([A-Z_]+)"\)\.map\(\|dir\| PathBuf::from\(dir\)\.join\("([^"]+)"\)\)/,
    "the Windows data folder from crates/app-core/src/config.rs",
  );
  const dataFolder = windows[2];

  const databaseFile = matches(
    config,
    /pub const DATABASE_FILE: &str = "([^"]+)"/,
    "the database file name from crates/app-core/src/config.rs",
  )[1];
  const providersFile = matches(
    config,
    /pub const PROVIDERS_FILE: &str = "([^"]+)"/,
    "the providers file name from crates/app-core/src/config.rs",
  )[1];

  const cookieName = matches(
    security,
    /pub const COOKIE_NAME: &str = "([^"]+)"/,
    "the session cookie name from apps/server/src/security.rs",
  )[1];
  const loopbackHost = matches(
    security,
    /hosts: \[format!\("([^"]+):\{port\}"/,
    "the loopback host from apps/server/src/security.rs",
  )[1];

  const defaultPort = matches(
    main,
    /default_value_t = (\d+)\)/,
    "the default port from apps/server/src/main.rs",
  )[1];

  const inspectorCapacity = matches(
    inspector,
    /pub const DEFAULT_CAPACITY: usize = (\d+)/,
    "the inspector capacity from crates/llm-client/src/inspector.rs",
  )[1];
  const bodyLimit = matches(
    inspector,
    /pub const MAX_BODY_BYTES: usize = (\d+) \* (\d+)/,
    "the inspector body limit from crates/llm-client/src/inspector.rs",
  );
  const bodyBytes = Number(bodyLimit[1]) * Number(bodyLimit[2]);
  const inspectorBodyLimit = bodyBytes % 1024 === 0 ? `${bodyBytes / 1024} KiB` : `${bodyBytes} bytes`;

  // The release zip: the members, the checksum file, the name shape and the
  // checksum algorithm, all from the packaging script.
  const membersBlock = matches(
    packaging,
    /const members = \[([\s\S]*?)\];/,
    "the release members from scripts/package-release.mjs",
  )[1];
  const zipMembers = [...membersBlock.matchAll(/name:\s*['"]([^'"]+)['"]/g)].map((match) => match[1]);
  const checksumFile = matches(
    packaging,
    /join\(outDir,\s*"([^"]+)"\)/,
    "the checksum file name from scripts/package-release.mjs",
  )[1];
  const zipTemplate = matches(
    packaging,
    /join\(outDir,\s*`([^`]+)`\)/,
    "the zip name template from scripts/package-release.mjs",
  )[1];
  const algorithm = matches(
    packaging,
    /createHash\("([^"]+)"\)/,
    "the checksum algorithm from scripts/package-release.mjs",
  )[1];
  const shaBits = /^sha(\d+)$/.exec(algorithm);
  const checksumAlgorithm = shaBits ? `SHA-${shaBits[1]}` : algorithm.toUpperCase();

  // The platform of the release: the executable in the archive is a Windows
  // one, and the owner's release runbook names the artifact the pipeline
  // writes. The version in that name is a build-time value, so only the
  // platform part is taken.
  const exeMember = zipMembers.find((name) => name.endsWith(".exe"));
  const operatingSystem = exeMember ? "windows" : "linux";
  const templatePattern = zipTemplate
    .split(/\$\{[^}]+\}/)
    .map(escapeRegExp)
    .join("[^\\s`]+");
  const artifactName = matches(
    ownerRelease,
    new RegExp(templatePattern),
    "the release artifact name in docs/OWNER_RELEASE.md",
  )[0];
  const namePrefix = zipTemplate.slice(0, zipTemplate.indexOf("${"));
  const nameSuffix = zipTemplate.slice(zipTemplate.lastIndexOf("}") + 1);
  const artifactMiddle = artifactName.slice(namePrefix.length, artifactName.length - nameSuffix.length);
  const platformAt = artifactMiddle.indexOf(operatingSystem);
  if (platformAt === -1) {
    throw new Error(`the release artifact name ${artifactName} does not carry the platform`);
  }
  const zipName = `${artifactMiddle.slice(platformAt)}${nameSuffix}`;

  // The four settings a provider needs, from the empty template.
  const envNames = [...envTemplate.matchAll(/^(TUTOR_LLM_[A-Z_]+)=/gm)].map((match) => match[1]);
  // The file the template is copied to.
  const envFile = matches(
    envTemplate,
    /Copy this file to (\S+)/,
    "the environment file name from .env.example",
  )[1];

  // The candidate models and the hosts their sources name.
  const modelIds = [...manifest.matchAll(/^\s*id\s*=\s*"([^"]+)"/gm)].map((match) => match[1]);
  const downloadHosts = [
    ...new Set([...manifest.matchAll(/^\s*source\s*=\s*"https?:\/\/([^/"]+)/gm)].map((match) => match[1])),
  ];

  // The paths CONTRIBUTING.md must point at. The gate is the one the commit
  // hook runs, the licence register is the one the model manifest names, and
  // the environment record is the one the owner keeps the toolchain versions
  // in; the last one is named here because no file in the code refers to it.
  const verifyScript = matches(
    hook,
    /^\s*sh (scripts\/\S+\.sh)/m,
    "the gate script from .husky/pre-commit",
  )[1];
  const licenseRegister = matches(
    manifest,
    /docs\/LICENSE_REGISTER\.md/,
    "the licence register from models/manifest.toml",
  )[0];
  const environmentRecord = "docs/ENVIRONMENT.md";
  if (!existsSync(join(ROOT, environmentRecord))) {
    throw new Error(`missing ${environmentRecord}`);
  }

  const facts = [];
  const fact = (key, value, label) => facts.push({ key, value, label });
  fact("dataFolder", dataFolder, "the name of the data folder");
  fact("localAppData", `%${windows[1]}%`, "the per-user folder the environment names");
  fact("databaseFile", databaseFile, "the database file name");
  fact("providersFile", providersFile, "the providers file name");
  fact("cookieName", cookieName, "the session cookie name");
  fact("loopbackHost", loopbackHost, "the loopback address");
  fact("defaultPort", defaultPort, "the default port");
  fact("inspectorCapacity", inspectorCapacity, "the number of requests the inspector keeps");
  fact("inspectorBodyLimit", inspectorBodyLimit, "the size a captured body is cut at");
  fact("checksumFile", checksumFile, "the checksum file name");
  fact("zipName", zipName, "the release zip name");
  fact("checksumAlgorithm", checksumAlgorithm, "the checksum algorithm");
  fact("envFile", envFile, "the environment file name");
  fact("verifyScript", verifyScript, "the gate script");
  fact("licenseRegister", licenseRegister, "the licence register");
  fact("environmentRecord", environmentRecord, "the environment record");
  for (const name of envNames) fact(`env:${name}`, name, `the ${name} setting`);
  for (const id of modelIds) fact(`model:${id}`, id, `the ${id} model id`);
  for (const member of zipMembers) fact(`member:${member}`, member, `the ${member} archive member`);
  for (const host of downloadHosts) fact(`host:${host}`, host, `the ${host} download host`);

  if (envNames.length === 0 || modelIds.length === 0 || zipMembers.length === 0 || downloadHosts.length === 0) {
    throw new Error("a derived list came out empty");
  }

  return { facts, envNames, modelIds, zipMembers, downloadHosts };
}

/** Which facts each document must state. The values live in `facts`. */
function requiredFacts(derived) {
  const env = derived.envNames.map((name) => `env:${name}`);
  const models = derived.modelIds.map((id) => `model:${id}`);
  const members = derived.zipMembers.map((member) => `member:${member}`);
  const hosts = derived.downloadHosts.map((host) => `host:${host}`);
  return {
    "docs/PRIVACY.md": [
      "dataFolder",
      "localAppData",
      "databaseFile",
      "providersFile",
      "cookieName",
      "loopbackHost",
      "defaultPort",
      "inspectorCapacity",
      "inspectorBodyLimit",
      ...env,
      ...hosts,
    ],
    "docs/MODELS.md": [...models, ...hosts, "checksumAlgorithm"],
    "docs/LIMITATIONS.md": ["loopbackHost", "defaultPort", ...hosts, "checksumAlgorithm"],
    "docs/INSTALL.md": [
      "dataFolder",
      "localAppData",
      "loopbackHost",
      "defaultPort",
      ...members,
      "checksumFile",
      "zipName",
      ...env,
      "checksumAlgorithm",
    ],
    "docs/RELEASE_NOTES.md": [
      "loopbackHost",
      "defaultPort",
      ...members,
      "checksumFile",
      "zipName",
      ...env,
    ],
    "CONTRIBUTING.md": ["verifyScript", "licenseRegister", "environmentRecord", "providersFile", "envFile"],
  };
}

/** The violations in the headings of a document. */
function checkHeadings(path, text, headings) {
  const violations = [];
  const lines = text.split(/\r?\n/).map((line) => line.trim());
  let previous = -1;
  for (const heading of headings) {
    const pattern = new RegExp(`^${escapeRegExp(heading)}\\s*$`);
    const index = lines.findIndex((line) => pattern.test(line));
    if (index === -1) {
      violations.push(`${path}: the heading "${heading}" is missing`);
      continue;
    }
    if (index <= previous) {
      violations.push(`${path}: the heading "${heading}" is out of order`);
    }
    previous = index;
  }
  return violations;
}

/** The banned wording in a document. */
function checkBannedWords(path, text) {
  const violations = [];
  for (const banned of BANNED_WORDS) {
    if (banned.pattern.test(text)) {
      violations.push(`${path}: uses ${banned.wording}, which is banned wording`);
    }
  }
  return violations;
}

/** The unfinished tokens in a document. */
function checkPlaceholders(path, text) {
  const violations = [];
  for (const placeholder of PLACEHOLDERS) {
    if (placeholder.pattern.test(text)) {
      violations.push(`${path}: still has the placeholder ${placeholder.token}`);
    }
  }
  return violations;
}

/** The relative links in a document, with the target as it is written. */
function linkTargets(text) {
  const targets = [];
  for (const match of text.matchAll(/\[[^\]]*\]\(\s*([^)\s]+)/g)) {
    let target = match[1];
    if (target.startsWith("<") && target.endsWith(">")) target = target.slice(1, -1);
    targets.push(target);
  }
  return targets;
}

/** The relative links in a document that point at nothing. */
function checkLinks(path, text) {
  const violations = [];
  for (const target of linkTargets(text)) {
    if (/^[a-z][a-z0-9+.-]*:/i.test(target)) continue; // an absolute URL
    if (target.startsWith("/")) continue; // an absolute path
    const withoutAnchor = target.split("#")[0];
    if (withoutAnchor === "") continue; // an anchor in this document
    if (!existsSync(join(dirname(join(ROOT, path)), withoutAnchor))) {
      violations.push(`${path}: the link to "${target}" points at nothing`);
    }
  }
  return violations;
}

/**
 * The facts a document gets wrong, and the facts it leaves out.
 *
 * A document that names a derived value must name it exactly: `lumingo.sqlite`
 * and `lumingo.sqliteX` are not the same file, and two documents that disagree
 * about the data folder, the archive members or the hosts are both wrong.
 */
function checkFacts(path, text, facts, required) {
  const violations = [];
  for (const fact of facts) {
    const found = occurrences(text, fact.value);
    for (const stated of found) {
      if (stated !== fact.value) {
        violations.push(`${path}: states "${stated}" where the code says "${fact.value}" (${fact.label})`);
      }
    }
  }
  for (const key of required) {
    const fact = facts.find((entry) => entry.key === key);
    if (!fact) throw new Error(`no fact named ${key}`);
    if (!occurrences(text, fact.value).includes(fact.value)) {
      violations.push(`${path}: does not say ${fact.label} ("${fact.value}")`);
    }
  }
  return violations;
}

function main() {
  const derived = deriveFacts();
  const required = requiredFacts(derived);
  const violations = [];
  for (const path of DOCUMENTS) {
    const text = read(path);
    violations.push(...checkHeadings(path, text, HEADINGS[path]));
    violations.push(...checkBannedWords(path, text));
    violations.push(...checkPlaceholders(path, text));
    violations.push(...checkLinks(path, text));
    violations.push(...checkFacts(path, text, derived.facts, required[path]));
  }
  if (violations.length > 0) {
    console.error("The release documents must match the code. These do not:");
    for (const violation of violations) console.error(`  ${violation}`);
    console.error("Change the document or the code, so the two say the same thing.");
    process.exitCode = 1;
    return;
  }
  console.log(
    `Document check passed (${DOCUMENTS.length} documents, ${derived.facts.length} derived facts).`,
  );
}

try {
  main();
} catch (error) {
  console.error(`document check: internal error: ${error.message}`);
  process.exitCode = 2;
}
