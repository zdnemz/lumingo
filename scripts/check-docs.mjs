#!/usr/bin/env node
// Fails when the release documents stop matching the code.
//
// The seven documents under check — README.md, docs/PRIVACY.md,
// docs/MODELS.md, docs/LIMITATIONS.md, docs/INSTALL.md, docs/RELEASE_NOTES.md
// and CONTRIBUTING.md — state facts about the program: where the data folder
// is, which hosts a download starts from, what the release zip holds, which
// names a provider needs. A document that drifts from the code is a document
// that lies, and a learner reads it before the program ever runs.
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

// The manifest path. It is the file that names the download hosts, so a
// document that names it names the hosts too.
const MANIFEST_PATH = "models/manifest.toml";

// The documents, in the order they are reported.
const DOCUMENTS = [
  "README.md",
  "docs/PRIVACY.md",
  "docs/MODELS.md",
  "docs/LIMITATIONS.md",
  "docs/INSTALL.md",
  "docs/RELEASE_NOTES.md",
  "CONTRIBUTING.md",
];

// The headings each document must carry, in the order they must carry them.
//
// A document with no list here is checked for its facts, its links and its
// wording only — its headings are not pinned. README.md is such a document:
// it is the owner's living front page, its sections change as the project
// changes, and its fenced code blocks carry lines that begin with `#`, so a
// heading list would read the code as headings and hold the front page
// hostage to a script update.
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

// The over-claims about the download path (PRD section 12.4). The path checks
// the URL, not the host: a redirect may reach another host. A document that
// names a download host may not say that the host set is the whole truth.
// These are wording patterns, like the banned words above, and they are
// checked in the paragraph that names the host — see `checkOverClaims`.
const OVER_CLAIMS = [
  { pattern: /\bcontact(?:s|ed|ing)?\s+only\b[^.]{0,60}\bhosts?\b/i, wording: '"contacts only ... hosts"' },
  { pattern: /\bcontact(?:s|ed|ing)?\s+only\s+[a-z0-9][a-z0-9.-]*\.[a-z]{2,}\b/i, wording: '"contacts only <host>"' },
  { pattern: /\bnothing\s+else\s+is\s+contacted\b/i, wording: '"nothing else is contacted"' },
];

// The false shapes about the provider's scheme. The code requires HTTPS and
// allows plain HTTP only for the loopback address, so a document that says
// otherwise is wrong. A sentence that carries a conditional marker is exempt:
// "the models can be downloaded once the manifest is filled in" is a true
// conditional, not a false claim.
//
// The ban is needed because a positive rule alone does not catch the false
// claim: "plain HTTP is allowed for any address and HTTPS is not required"
// still contains the words HTTPS and HTTP, so a rule that asks for them
// passes it. The positive rule catches the scheme rule being REMOVED; this
// ban catches the false claim being ADDED.
const SCHEME_FALSE_CLAIMS = [
  { pattern: /`?https`?\s+is\s+not\s+required\b/i, wording: '"https is not required"' },
  { pattern: /`?https`?\s+is\s+not\s+needed\b/i, wording: '"https is not needed"' },
  { pattern: /`?https`?\s+is\s+not\s+enforced\b/i, wording: '"https is not enforced"' },
  { pattern: /`?https`?\s+is\s+not\s+mandatory\b/i, wording: '"https is not mandatory"' },
  { pattern: /`?plain\s+`?http`?\s+is\s+allowed\s+for\s+any\b/i, wording: '"plain http is allowed for any"' },
  { pattern: /`?plain\s+`?http`?\s+is\s+allowed\s+for\s+every\b/i, wording: '"plain http is allowed for every"' },
  { pattern: /`?plain\s+`?http`?\s+is\s+allowed\s+for\s+all\b/i, wording: '"plain http is allowed for all"' },
];

// The false shapes about the models. They assert a total claim — every
// model, all of the models, any model, the models — so a sentence that
// carries a conditional marker is exempt, for the same reason as above.
//
// The ban is needed for the same reason too: a positive rule alone does not
// catch "Every model is ready to download now", because that sentence is
// ADDED to a document that already carries the true one, so the true
// sentence is still there and the positive rule passes it.
const DOWNLOAD_FALSE_CLAIMS = [
  /\bevery\s+model\b[^.]{0,80}?\b(?:can\s+be\s+(?:downloaded|fetched)|is\s+ready|ready\s+to\s+download)\b/i,
  /\ball\s+of\s+the\s+models\b[^.]{0,80}?\b(?:can\s+be\s+(?:downloaded|fetched)|is\s+ready|ready\s+to\s+download)\b/i,
  /\bany\s+model\b[^.]{0,80}?\bcan\s+be\s+(?:downloaded|fetched)\b/i,
  /\bthe\s+models\b[^.]{0,80}?\bcan\s+be\s+downloaded\b/i,
];

// The shape a sentence states that no model can be downloaded. The wording
// varies — "no model can be downloaded yet" and "no entry in the model
// manifest can be downloaded yet" say the same thing — so the shape is read
// as a negation of the downloadability of a model.
const INERTNESS_CLAIM = /\bno\s+(?:model|entry)\b[^.]{0,80}?\bcan\s+be\s+downloaded\b/i;

// The markers that make a sentence conditional. What a sentence says happens
// later is not a claim about what is true now.
const CONDITIONAL_MARKER = /\b(?:once|until|when|after|if|unless|whenever)\b/i;

// The gate script's quick guard: the line that opens the part of the gate that
// runs only in the full mode. The line that closes it is a `fi` in column 0 —
// the `fi`s of the conditions inside the guard are indented, so only a guard's
// own closing line sits in column 0.
const QUICK_GUARD = 'if [ "$quick" = 0 ]; then';
const GUARD_CLOSE = "fi";

// A step of the gate: the line that names the step. The steps inside a guard
// are indented, so the match tolerates leading whitespace.
const STEP_LINE = /^\s*step\s+"/;

// The shell keywords. A step whose first word is one of them is a condition
// and not a command, so it names nothing a document could name.
const SHELL_KEYWORDS = new Set([
  "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done",
  "case", "esac", "select", "function", "{", "}", "(", ")", "!", "&&", "||",
]);

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
  const manifest = read(MANIFEST_PATH);
  const urlPolicy = read("crates/model-manager/src/manifest.rs");
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

  // The transport rule the download path enforces, from `url_problem` in the
  // model manager: the scheme it allows, and the word the code uses for the
  // follow-up request. A document that names a download host has to state
  // this rule, in the code's own words.
  const scheme = matches(
    urlPolicy,
    /match url\.scheme\(\) \{\s*"([^"]+)" => None,/,
    "the allowed scheme from crates/model-manager/src/manifest.rs",
  )[1];
  const redirectWord = matches(
    urlPolicy,
    /every request and every ([a-z]+):/i,
    "the word the code uses for a redirect in crates/model-manager/src/manifest.rs",
  )[1];

  // The provider allowlist, from the HTTP client that guards every provider
  // request: the shape the allowlist holds and the scheme rule it enforces.
  // A document that states the allowlist has to state that rule in the code's
  // own words, so both are derived from the client.
  const providerScheme = deriveProviderScheme(read("crates/llm-client/src/http.rs"));

  // The state of the manifest: the entries that pass the code's own download
  // check. It is empty while every entry is a candidate, and the documents'
  // inertness claims follow it.
  const downloadableIds = deriveDownloadableIds(manifest, urlPolicy);

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

  // The commands the gate's quick mode runs. The gate script is the source:
  // its step list is read out of the script, never written down here.
  const gateCommands = deriveGateCommands(verifyScript);

  return {
    facts,
    envNames,
    modelIds,
    zipMembers,
    downloadHosts,
    manifestPath: MANIFEST_PATH,
    scheme,
    redirectWord,
    hostPair: providerScheme.hostPair,
    schemeTokens: providerScheme.schemeTokens,
    downloadableIds,
    verifyScript,
    gateCommands,
  };
}

/**
 * The commands the gate's quick mode runs, read out of the gate script.
 *
 * The gate script is split into segments at its quick guards: what is outside
 * a guard runs in the quick mode, what is inside runs only in the full mode.
 * A step is a `step "…"` line and the lines under it, and a step's command is
 * the first line of its body that says something. The list is derived, so a
 * step added to the gate is a step the documents have to name — and a gate
 * this cannot read is a broken source, so it throws rather than passing.
 */
function deriveGateCommands(gatePath) {
  const lines = read(gatePath).split(/\r?\n/);
  const segments = [];
  let segment = [];
  let inside = false;
  for (const line of lines) {
    if (!inside) {
      if (line === QUICK_GUARD) {
        inside = true;
        continue;
      }
      segment.push(line);
    } else if (line === GUARD_CLOSE) {
      inside = false;
      segments.push(segment);
      segment = [];
    }
  }
  if (inside) throw new Error(`${gatePath} has a quick guard that is never closed`);
  segments.push(segment);

  const commands = [];
  for (const block of segments) {
    for (const body of stepBodies(block)) {
      const command = stepCommand(body);
      if (command) commands.push(command);
    }
  }
  if (commands.length === 0) throw new Error(`${gatePath} has no step to read a command from`);
  return commands;
}

/** The bodies of the steps in a block of the gate script. */
function stepBodies(block) {
  const bodies = [];
  let index = 0;
  while (index < block.length) {
    if (!STEP_LINE.test(block[index])) {
      index += 1;
      continue;
    }
    let end = index + 1;
    while (end < block.length && !STEP_LINE.test(block[end])) end += 1;
    bodies.push(block.slice(index + 1, end));
    index = end;
  }
  return bodies;
}

/**
 * The command a step runs: the first line of its body that says something.
 *
 * A line that is blank or a comment says nothing. The rest is read as one
 * line: the trailing backslash of a continued line is dropped and the
 * whitespace is collapsed. A first word that is a shell keyword means the
 * step is a condition and not a command, so it names nothing. A leading
 * `node ` is dropped: the interpreter is not the program, and the file name
 * is what a document names.
 */
function stepCommand(body) {
  for (const line of body) {
    const trimmed = line.trim();
    if (trimmed === "" || trimmed.startsWith("#")) continue;
    const collapsed = trimmed.replace(/\\\s*$/, "").replace(/\s+/g, " ").trim();
    if (SHELL_KEYWORDS.has(collapsed.split(" ")[0])) return null;
    return collapsed.startsWith("node ") ? collapsed.slice("node ".length) : collapsed;
  }
  return null;
}

/**
 * The provider allowlist's shape and the scheme rule it enforces, read out of
 * the HTTP client.
 *
 * The allowlist holds a host and a port, and its own `Display` writes them
 * as `host:port` — that is the shape a document names when it talks about
 * the allowlist. The scheme rule is the arms of the `match` on the URL's
 * scheme: `https` is always allowed and `http` only behind the loopback
 * guard, and the word for that guard is its own name, so the rule is asked
 * for in the code's own words.
 *
 * A source this cannot read is a broken source, so it throws rather than
 * passing: a client with no scheme arm would allow any scheme.
 */
function deriveProviderScheme(httpClient) {
  const hostPair = matches(
    httpClient,
    /write!\(f, "\{\}:\{\}", self\.(\w+), self\.(\w+)\)/,
    "the host:port shape from crates/llm-client/src/http.rs",
  );
  const arms = [
    ...matches(
      httpClient,
      /match url\.scheme\(\) \{([\s\S]*?)\n        \}/,
      "the scheme arms from crates/llm-client/src/http.rs",
    )[1].matchAll(/^\s*"([^"]+)"\s*(?:if\s+([a-z_][a-z0-9_]*)\([^)]*\)\s*)?=>/gm),
  ];
  const schemes = [...new Set(arms.map((arm) => arm[1]))];
  if (schemes.length === 0) {
    throw new Error("no scheme arm in crates/llm-client/src/http.rs");
  }
  const guard = arms.find((arm) => arm[2]);
  if (!guard) {
    throw new Error("no guarded scheme arm in crates/llm-client/src/http.rs");
  }
  return {
    hostPair: `${hostPair[1]}:${hostPair[2]}`,
    schemeTokens: [...schemes, longestWord(guard[2])],
  };
}

/**
 * The word a snake_case name names: its longest segment.
 *
 * The guard that allows plain HTTP is `is_loopback_url`, so the word the
 * scheme rule is about is `loopback`.
 */
function longestWord(name) {
  const parts = name.split("_").filter((part) => part !== "");
  if (parts.length === 0) throw new Error(`cannot read a word from ${name}`);
  return parts.reduce((longest, part) => (part.length > longest.length ? part : longest));
}

/**
 * The manifest entries that can be downloaded.
 *
 * The conditions are the ones the code's own `check_downloadable` uses: a
 * licence that is not empty and not the word `unknown`, a licence URL, a
 * file list that is not empty, and a checksum of 64 hexadecimal characters
 * on every file. An entry that fails any of them is refused before the
 * downloader makes a request, so the entries that pass them are the entries
 * that can be downloaded.
 */
function deriveDownloadableIds(manifest, urlPolicy) {
  const shaLength = matches(
    urlPolicy,
    /text\.len\(\) == (\d+) && text\.bytes\(\)\.all\(\|b\| b\.is_ascii_hexdigit\(\)\)/,
    "the checksum length from crates/model-manager/src/manifest.rs",
  )[1];
  const unknownLicence = matches(
    urlPolicy,
    /eq_ignore_ascii_case\("([^"]+)"\)/,
    "the word the code uses for an unread licence in crates/model-manager/src/manifest.rs",
  )[1];
  const ids = [];
  for (const entry of manifestEntries(manifest)) {
    const licence = entry.license.trim();
    if (licence === "" || licence.toLowerCase() === unknownLicence) continue;
    if (entry.licenseUrl.trim() === "") continue;
    if (entry.files.length === 0) continue;
    if (!entry.files.every((file) => isChecksum(file, shaLength))) continue;
    ids.push(entry.id);
  }
  return ids;
}

/**
 * The manifest entries, with the fields the download check reads.
 *
 * The manifest is read as text, not parsed as TOML: an entry is a `[[model]]`
 * block, and the fields are the ones the code checks.
 */
function manifestEntries(manifest) {
  const entries = [];
  for (const block of manifest.split(/^\[\[model\]\]$/m).slice(1)) {
    entries.push({
      id: stringField(block, "id"),
      license: stringField(block, "license"),
      licenseUrl: stringField(block, "license_url"),
      files: fileEntries(block),
    });
  }
  return entries;
}

/** A string field of a manifest block, or an empty string when it has none. */
function stringField(block, key) {
  const found = block.match(new RegExp(`^\\s*${key}\\s*=\\s*"([^"]*)"`, "m"));
  return found ? found[1] : "";
}

/** The file entries of a manifest block: the inline tables of its `files`. */
function fileEntries(block) {
  const array = arrayField(block, "files");
  if (array === null) return [];
  const files = [];
  let depth = 0;
  let start = -1;
  for (let i = 0; i < array.length; i += 1) {
    if (array[i] === "{") {
      if (depth === 0) start = i;
      depth += 1;
    } else if (array[i] === "}") {
      depth -= 1;
      if (depth === 0) files.push(array.slice(start, i + 1));
    }
  }
  return files;
}

/** The text of an array field of a manifest block, brackets included. */
function arrayField(block, key) {
  const at = block.search(new RegExp(`^\\s*${key}\\s*=\\s*\\[`, "m"));
  if (at === -1) return null;
  const open = block.indexOf("[", at);
  let depth = 0;
  for (let i = open; i < block.length; i += 1) {
    if (block[i] === "[") depth += 1;
    else if (block[i] === "]") {
      depth -= 1;
      if (depth === 0) return block.slice(open, i + 1);
    }
  }
  return null;
}

/** A file's checksum: the hexadecimal characters the code requires. */
function isChecksum(file, length) {
  const found = file.match(/sha256\s*=\s*"([^"]*)"/);
  if (!found) return false;
  return new RegExp(`^[0-9a-f]{${length}}$`, "i").test(found[1].trim());
}

/** Which facts each document must state. The values live in `facts`. */
function requiredFacts(derived) {
  const env = derived.envNames.map((name) => `env:${name}`);
  const models = derived.modelIds.map((id) => `model:${id}`);
  const members = derived.zipMembers.map((member) => `member:${member}`);
  const hosts = derived.downloadHosts.map((host) => `host:${host}`);
  return {
    "README.md": [
      "dataFolder",
      "localAppData",
      "providersFile",
      "loopbackHost",
      "defaultPort",
      "inspectorCapacity",
      "inspectorBodyLimit",
      "checksumAlgorithm",
      "envFile",
      "verifyScript",
      "licenseRegister",
      "environmentRecord",
      ...env,
      ...hosts,
    ],
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
  if (!headings) return violations; // a document with no pinned heading list
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

/**
 * Whether a text names a download host.
 *
 * A host is one of the hosts the manifest names, or the manifest path itself
 * — that file is what names those hosts, so a document that points at it
 * speaks about the download path.
 */
function mentionsDownloadHost(text, derived) {
  if (occurrences(text, derived.manifestPath).length > 0) return true;
  return derived.downloadHosts.some((host) => occurrences(text, host).length > 0);
}

/** A regex that matches `value` as a word, whatever its case. */
function wordPattern(value) {
  return new RegExp(`(?<!\\w)${escapeRegExp(value)}(?!\\w)`, "i");
}

/**
 * The redirect rule a document that names a download host has to state.
 *
 * The download path checks the URL, not the host: a redirect is checked with
 * the same rule and then followed, so it may reach another host. A document
 * that names the hosts and never says that is free to imply that the host set
 * is the whole truth, so the rule has to be stated — with the word the code
 * uses for it and with the scheme the code allows. The rule is positive, so
 * it fires only when a document stops saying the rule, never on a change of
 * wording around it.
 */
function checkRedirectRule(path, text, derived) {
  const violations = [];
  if (!mentionsDownloadHost(text, derived)) return violations;
  if (!wordPattern(derived.redirectWord).test(text)) {
    violations.push(
      `${path}: names a download host but does not state the redirect rule (the word "${derived.redirectWord}")`,
    );
  }
  if (!wordPattern(derived.scheme).test(text)) {
    violations.push(
      `${path}: names a download host but does not state the scheme rule (the word "${derived.scheme}")`,
    );
  }
  return violations;
}

/** The paragraphs of a document: the blocks a blank line separates. */
function paragraphs(text) {
  return text.split(/\n\s*\n/).filter((block) => block.trim() !== "");
}

/**
 * The sentences of a document, with the table rows left out.
 *
 * A table cell is a list, not a statement, so a line that carries a table
 * separator is not prose: the scan leaves it out, and a gap table cannot
 * stand in for the sentence a rule asks for. The lines of a paragraph are
 * joined first, because a sentence can wrap over several of them.
 */
function sentences(text) {
  const found = [];
  for (const block of paragraphs(text)) {
    const prose = block
      .split(/\r?\n/)
      .filter((line) => !line.includes("|"))
      .join(" ");
    for (const sentence of prose.split(/(?<=[.!?])\s+/)) {
      if (sentence.trim() !== "") found.push(sentence);
    }
  }
  return found;
}

/**
 * The over-claims in a paragraph that names a download host.
 *
 * The scope is the paragraph, not the document: `docs/PRIVACY.md` says "the
 * browser tab contacts only the local server", which is true, is not about
 * the download path, and must not be flagged. The ban is scoped to the
 * download context so it cannot fire on a true statement about the provider
 * path or the browser tab.
 */
function checkOverClaims(path, text, derived) {
  const violations = [];
  for (const paragraph of paragraphs(text)) {
    if (!mentionsDownloadHost(paragraph, derived)) continue;
    for (const claim of OVER_CLAIMS) {
      if (claim.pattern.test(paragraph)) {
        violations.push(
          `${path}: says ${claim.wording}, which pins the host set the download path does not pin`,
        );
      }
    }
  }
  return violations;
}

/**
 * The gate commands a document that names the gate has to name too.
 *
 * Wave 1 added the gate to the commit hook and the documents did not follow:
 * a document that names the gate script and does not say what it runs sends
 * the reader to a script whose steps they cannot know. So a document that
 * names the gate has to name every command the quick mode runs. The rule is
 * positive — it asks for the command to be present — so it fires only when a
 * document stops naming a command, never on a change of wording around it.
 */
function checkGateCommands(path, text, derived) {
  const violations = [];
  if (occurrences(text, derived.verifyScript).length === 0) return violations;
  for (const command of derived.gateCommands) {
    if (!occurrences(text, command).includes(command)) {
      violations.push(`${path}: does not name the gate command "${command}"`);
    }
  }
  return violations;
}

/**
 * The provider allowlist a document states, and the scheme rule it misses.
 *
 * A document that names the allowlist's shape has to state the scheme rule
 * the client enforces, in the code's own words: every scheme the arms accept
 * and the word for the loopback guard. The rule is positive, so it fires
 * only when a document stops saying the rule, never on a change of wording
 * around it.
 *
 * The ban is the other half of the rule: a document may not say a falsehood
 * about the scheme. A positive rule alone does not catch that — "plain HTTP
 * is allowed for any address and HTTPS is not required" still contains the
 * words HTTPS and HTTP, so the rule above passes it. The rule catches the
 * scheme rule being REMOVED; the ban catches the false claim being ADDED.
 */
function checkProviderScheme(path, text, derived) {
  const violations = [];
  for (const block of paragraphs(text)) {
    if (!occurrences(block, derived.hostPair).includes(derived.hostPair)) continue;
    for (const token of derived.schemeTokens) {
      if (!wordPattern(token).test(block)) {
        violations.push(`${path}: states the provider allowlist but does not state the ${token} rule`);
      }
    }
  }
  for (const sentence of sentences(text)) {
    if (CONDITIONAL_MARKER.test(sentence)) continue;
    for (const claim of SCHEME_FALSE_CLAIMS) {
      if (claim.pattern.test(sentence)) {
        violations.push(`${path}: says ${claim.wording}, which the code does not allow`);
      }
    }
  }
  return violations;
}

/**
 * The state of the manifest, and what a document says about it.
 *
 * Nothing in the manifest can be downloaded yet: every entry is a candidate
 * with an empty file list, and the model manager refuses such an entry before
 * it makes a request. A document that names a download host or the manifest
 * has to say so — a reader who finds the host list in a document should not
 * walk away believing a download is one click away.
 *
 * The rule follows the manifest. The moment the owner fills in `files` and
 * `sha256`, the derived set is no longer empty, the documents' inertness
 * claims become false, and the rule reports them instead. That half is
 * document-wide, because a document that makes the claim need not name a
 * host.
 */
function checkManifestState(path, text, derived) {
  const violations = [];
  const said = sentences(text).filter((sentence) => INERTNESS_CLAIM.test(sentence));
  if (derived.downloadableIds.length === 0) {
    if (mentionsDownloadHost(text, derived) && said.length === 0) {
      violations.push(
        `${path}: names a download host but does not state that no model can be downloaded yet`,
      );
    }
    for (const claim of DOWNLOAD_FALSE_CLAIMS) {
      const saidIt = sentences(text).some(
        (sentence) => !CONDITIONAL_MARKER.test(sentence) && claim.test(sentence),
      );
      if (saidIt) {
        violations.push(
          `${path}: says a model can be downloaded, but no entry in the manifest is downloadable`,
        );
      }
    }
  } else if (said.length > 0) {
    violations.push(
      `${path}: says no model can be downloaded yet, but the manifest has ${derived.downloadableIds.join(", ")}`,
    );
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
    violations.push(...checkRedirectRule(path, text, derived));
    violations.push(...checkOverClaims(path, text, derived));
    violations.push(...checkGateCommands(path, text, derived));
    violations.push(...checkProviderScheme(path, text, derived));
    violations.push(...checkManifestState(path, text, derived));
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
