// License gate for JavaScript packages (docs/LICENSE_REGISTER.md, rule 6).
// Production dependencies may only use permissive licenses. Development tools
// may also use a few more, because none of them reaches the exported UI.
import { execFileSync } from "node:child_process";

const PERMISSIVE = new Set([
  "MIT",
  "MIT-0",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "0BSD",
  "CC0-1.0",
  "Unlicense",
  "Zlib",
]);

// Packages that are data or tooling with a known, reviewed license.
const REVIEWED_PRODUCTION = new Map([
  // Browser support data used by the Next.js build. Attribution is in NOTICE.
  ["caniuse-lite", "CC-BY-4.0"],
]);
const DEV_ONLY_EXTRA = new Set(["BlueOak-1.0.0", "Python-2.0", "MPL-2.0"]);

function listLicenses(args) {
  const out = execFileSync("pnpm", ["licenses", "list", "--json", ...args], {
    encoding: "utf8",
    shell: process.platform === "win32",
    stdio: ["ignore", "pipe", "ignore"],
  });
  return JSON.parse(out);
}

function violations(listing, allowedLicenses, reviewed) {
  const bad = [];
  for (const [license, packages] of Object.entries(listing)) {
    for (const pkg of packages) {
      const exception = reviewed.get(pkg.name);
      if (allowedLicenses.has(license) || exception === license) continue;
      bad.push(`${pkg.name}@${pkg.versions.join(",")} uses ${license}`);
    }
  }
  return bad;
}

const production = violations(listLicenses(["--prod"]), PERMISSIVE, REVIEWED_PRODUCTION);
const everything = violations(
  listLicenses([]),
  new Set([...PERMISSIVE, ...DEV_ONLY_EXTRA]),
  REVIEWED_PRODUCTION,
);

const problems = [...new Set([...production, ...everything])];
if (problems.length > 0) {
  console.error("License check failed:");
  for (const line of problems) console.error(`  ${line}`);
  process.exit(1);
}
console.log("License check passed.");
