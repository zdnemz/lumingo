// lint-staged: format and fix only the staged files, before the checks in
// scripts/verify.sh run. See .husky/pre-commit.
import path from "node:path";

export default {
  // The workspace edition is 2024; rustfmt defaults to 2015 without it.
  // skip_children: format only the file lint-staged passed, not its `mod`
  // children — a child rustfmt touched would not be restaged, so the commit
  // could stay unformatted while the working tree looks clean.
  "**/*.rs": "rustfmt --edition 2024 --config skip_children=true",
  // The web app has its own ESLint config. lint-staged passes paths relative to
  // the repository root, and ESLint must run from `apps/web`, so each path is
  // made relative to that directory and quoted.
  "apps/web/**/*.{ts,tsx,mts,mjs}": (files) => {
    const relative = files.map((file) => path.relative("apps/web", file));
    return `pnpm --dir apps/web exec eslint --fix --no-warn-ignored ${relative
      .map((file) => JSON.stringify(file))
      .join(" ")}`;
  },
};
