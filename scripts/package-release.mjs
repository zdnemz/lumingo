// The release packaging script.
//
// Writes <out>/lumingo-<version>-<platform>.zip and, next to it,
// <out>/checksums.txt.
//
// The archive is written here, byte by byte: no platform zip tool, so the
// same code produces the same archive on Linux and on Windows. Every entry
// carries a fixed timestamp (1980-01-01 00:00:00), so the same inputs always
// give a byte-identical zip.
//
// The archive is flat and holds exactly five members: '.env.example',
// 'LICENSE' and 'NOTICE' from the repository root, 'README.md' from
// --readme, and 'lumingo.exe' from --exe.
//
// The script reads those five files and nothing else: no secret file, no
// network, and nothing is written outside --out.
//
// Exit codes: 0 the zip and the checksums were written; 1 a usage or input
// error (a missing flag, a file that does not exist, an unwritable output
// directory); 2 an internal error.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { crc32, deflateRawSync } from "node:zlib";

// The repository root, found from this file's own path, so the script works
// from any working directory.
const REPO_ROOT = dirname(dirname(fileURLToPath(import.meta.url)));

const FLAGS = ["--exe", "--readme", "--out", "--version", "--platform"];

const USAGE = `Usage:
  node scripts/package-release.mjs --exe <path> --readme <path> --out <dir> --version <string> --platform <string>

Flags:
  --exe <path>        the executable; it goes into the zip as lumingo.exe
  --readme <path>     the README source; it goes into the zip as README.md
  --out <dir>         the output directory; it is created if it is missing
  --version <string>  the version; it is part of the zip's name
  --platform <string> the platform; it is part of the zip's name

Writes <out>/lumingo-<version>-<platform>.zip and <out>/checksums.txt.
`;

// 1980-01-01 00:00:00, the oldest timestamp a zip entry can carry. A fixed
// timestamp is what makes the archive deterministic.
const DOS_DATE = 0x0021;
const DOS_TIME = 0x0000;

/** An error the caller caused: a missing flag, a missing file, a bad directory. */
class InputError extends Error {}

function fail(message) {
  throw new InputError(message);
}

/**
 * The command line. Every flag is required, there are no others, and --help
 * is the only exception.
 */
function parseArgs(argv) {
  if (argv.includes("--help")) return { help: true };

  const options = {};
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (!flag.startsWith("--")) fail(`unexpected argument: ${flag}`);
    if (!FLAGS.includes(flag)) fail(`unknown flag: ${flag}`);
    if (options[flag] !== undefined) fail(`${flag} was given twice`);
    const value = argv[i + 1];
    if (value === undefined || value.startsWith("--")) fail(`${flag} needs a value`);
    options[flag] = value;
    i += 1;
  }
  for (const flag of FLAGS) {
    if (options[flag] === undefined) fail(`missing flag: ${flag}`);
  }
  return options;
}

/** A file's bytes, or an input error when it is not there. */
function readSource(path, what) {
  let stats;
  try {
    stats = statSync(path);
  } catch {
    fail(`${what} does not exist: ${path}`);
  }
  if (!stats.isFile()) fail(`${what} is not a regular file: ${path}`);
  try {
    return readFileSync(path);
  } catch {
    fail(`${what} cannot be read: ${path}`);
  }
}

/**
 * The archive. Every entry is written as a local header, the data, and a
 * central directory entry; the directory is closed by an end-of-directory
 * record. Entries are deflated when that is smaller and stored when it is
 * not; both are methods Windows Explorer reads.
 */
function buildZip(entries) {
  const parts = [];
  const directory = [];
  let offset = 0;

  for (const entry of entries) {
    const name = Buffer.from(entry.name, "utf8");
    const deflated = deflateRawSync(entry.data);
    const stored = deflated.length >= entry.data.length;
    const body = stored ? entry.data : deflated;
    const method = stored ? 0 : 8;
    const checksum = crc32(entry.data) >>> 0;

    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0); // local file header signature
    local.writeUInt16LE(20, 4); // version needed to extract
    local.writeUInt16LE(0, 6); // general purpose flags
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(DOS_TIME, 10);
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(body.length, 18); // compressed size
    local.writeUInt32LE(entry.data.length, 22); // uncompressed size
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28); // extra field length
    parts.push(local, name, body);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0); // central file header signature
    central.writeUInt16LE(20, 4); // version made by
    central.writeUInt16LE(20, 6); // version needed to extract
    central.writeUInt16LE(0, 8); // general purpose flags
    central.writeUInt16LE(method, 10);
    central.writeUInt16LE(DOS_TIME, 12);
    central.writeUInt16LE(DOS_DATE, 14);
    central.writeUInt32LE(checksum, 16);
    central.writeUInt32LE(body.length, 20);
    central.writeUInt32LE(entry.data.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt16LE(0, 30); // extra field length
    central.writeUInt16LE(0, 32); // comment length
    central.writeUInt16LE(0, 34); // disk number start
    central.writeUInt16LE(0, 36); // internal file attributes
    central.writeUInt32LE(0, 38); // external file attributes
    central.writeUInt32LE(offset, 42); // offset of the local header
    directory.push(central, name);

    offset += local.length + name.length + body.length;
  }

  const start = offset;
  const centralDirectory = Buffer.concat(directory);

  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); // end of central directory signature
  end.writeUInt16LE(0, 4); // number of this disk
  end.writeUInt16LE(0, 6); // disk with the central directory
  end.writeUInt16LE(entries.length, 8); // entries on this disk
  end.writeUInt16LE(entries.length, 10); // entries in total
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(start, 16);
  end.writeUInt16LE(0, 20); // comment length

  return Buffer.concat([...parts, centralDirectory, end]);
}

/** The members, in ascending byte order of their names. */
function collectMembers(options) {
  const members = [
    { name: '.env.example', data: readSource(join(REPO_ROOT, '.env.example'), "the environment template") },
    { name: "LICENSE", data: readSource(join(REPO_ROOT, "LICENSE"), "the licence") },
    { name: "NOTICE", data: readSource(join(REPO_ROOT, "NOTICE"), "the notice") },
    { name: "README.md", data: readSource(resolve(options["--readme"]), "the README") },
    { name: "lumingo.exe", data: readSource(resolve(options["--exe"]), "the executable") },
  ];
  return members.sort((a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name)));
}

/** The checksum file: the sha256sum format, one line per member. */
function buildChecksums(members) {
  const lines = members.map((member) => {
    const digest = createHash("sha256").update(member.data).digest("hex");
    return `${digest}  ${member.name}`;
  });
  return `${lines.join("\n")}\n`;
}

function run(argv) {
  const options = parseArgs(argv);
  if (options.help) {
    process.stdout.write(USAGE);
    return 0;
  }

  const version = options["--version"];
  const platform = options["--platform"];
  const namePart = /^[A-Za-z0-9][A-Za-z0-9._+-]*$/;
  if (!namePart.test(version)) fail(`--version is not a usable name part: ${version}`);
  if (!namePart.test(platform)) fail(`--platform is not a usable name part: ${platform}`);

  const outDir = resolve(options["--out"]);
  const zipPath = join(outDir, `lumingo-${version}-${platform}.zip`);
  const checksumsPath = join(outDir, "checksums.txt");

  const members = collectMembers(options);
  const zip = buildZip(members);
  const checksums = buildChecksums(members);

  try {
    mkdirSync(outDir, { recursive: true });
    writeFileSync(zipPath, zip);
    writeFileSync(checksumsPath, checksums, "utf8");
  } catch {
    fail(`the output directory cannot be written: ${outDir}`);
  }

  const report = [
    `zip: ${zipPath}`,
    `size: ${zip.length} bytes`,
    ...members.map((member) => `${member.name}: ${member.data.length} bytes`),
    `checksums: ${checksumsPath}`,
  ];
  process.stdout.write(`${report.join("\n")}\n`);
  return 0;
}

try {
  process.exitCode = run(process.argv.slice(2));
} catch (error) {
  if (error instanceof InputError) {
    process.stderr.write(`error: ${error.message}\n`);
    process.exitCode = 1;
  } else {
    process.stderr.write(`internal error: ${error?.stack ?? error}\n`);
    process.exitCode = 2;
  }
}
