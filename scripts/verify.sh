#!/usr/bin/env sh
# The checks .github/workflows/ci.yml runs, on this machine, before a commit.
#
# CI runs on windows-latest; this script runs the same commands on the machine
# that is committing, against the working tree (not the index: keep unrelated
# edits out of the tree while committing). A green run here is what a green
# push looks like.
#
#   sh scripts/verify.sh                      # everything, like CI
#   LUMINGO_PRECOMMIT=quick sh scripts/verify.sh
#       the path case check, format, lint and typecheck only. clippy, the tests,
#       the build and the license checks are skipped and CI may still fail.
set -eu

# Run from the repository root, whatever the caller's directory was. A clear
# message when this is not inside a checkout, instead of a confusing failure
# from the first cargo command.
if ! root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    echo "pre-commit: this is not inside a git checkout. Run it from the Lumingo repository." >&2
    exit 1
fi
cd "$root" || exit 1

if [ "${LUMINGO_PRECOMMIT:-full}" = "quick" ]; then
    quick=1
else
    quick=0
fi

if ! command -v pnpm >/dev/null 2>&1; then
    echo "pre-commit: pnpm is not on PATH. Install it (the web app needs it) and commit again." >&2
    exit 1
fi

started=$(date +%s)
current=""
on_exit() {
    code=$?
    if [ "$code" -ne 0 ] && [ -n "$current" ]; then
        printf '\npre-commit: FAILED at: %s\n' "$current" >&2
        printf 'pre-commit: fix the error above, then commit again.\n' >&2
        printf 'pre-commit: `git commit --no-verify` skips this hook; CI still runs every check.\n' >&2
    fi
}
trap on_exit EXIT

step() {
    current="$1"
    printf '\n==> %s\n' "$1"
}

if [ "$quick" = 1 ]; then
    printf 'pre-commit: LUMINGO_PRECOMMIT=quick — running format, lint, typecheck and the path case check only.\n'
    printf 'pre-commit: clippy, the tests, the build and the license checks are skipped.\n'
fi

# First, because it is instant and because the mistake it catches is invisible
# on this machine: TypeScript strips the extension before resolving an import,
# so `KeyGuide.tsx` and `keyGuide.ts` are one module on Windows, where CI runs,
# while this file system keeps them apart.
step "no two tracked paths differ only in case"
node scripts/check-case-collisions.mjs

step "cargo fmt --all -- --check"
cargo fmt --all -- --check

if [ "$quick" = 0 ]; then
    step "cargo clippy --workspace --all-targets -- -D warnings"
    cargo clippy --workspace --all-targets -- -D warnings

    step "cargo test --workspace"
    cargo test --workspace

    step "the generated TypeScript types are up to date"
    if ! git diff --quiet -- apps/web/src/generated \
        || [ -n "$(git ls-files --others --exclude-standard -- apps/web/src/generated)" ]; then
        printf 'The tests regenerated apps/web/src/generated and it differs from what is committed.\n'
        printf 'Stage it and commit again: git add apps/web/src/generated\n' >&2
        git status --short -- apps/web/src/generated >&2
        false
    fi

    if command -v cargo-deny >/dev/null 2>&1; then
        step "cargo deny check licenses"
        cargo deny check licenses
    else
        printf '\n==> skipped: cargo deny (not installed). Install with: cargo install cargo-deny --locked\n'
    fi

    step "pnpm install --frozen-lockfile (root tooling)"
    pnpm install --frozen-lockfile

    step "pnpm --dir apps/web install --frozen-lockfile"
    pnpm --dir apps/web install --frozen-lockfile
fi

step "pnpm --dir apps/web lint"
pnpm --dir apps/web lint

step "pnpm --dir apps/web typecheck"
pnpm --dir apps/web typecheck

if [ "$quick" = 0 ]; then
    step "pnpm --dir apps/web test"
    pnpm --dir apps/web test

    step "pnpm --dir apps/web build"
    pnpm --dir apps/web build

    step "pnpm --dir apps/web run check:licenses"
    pnpm --dir apps/web run check:licenses
fi

current=""
printf '\npre-commit: all checks passed in %ss.\n' "$(( $(date +%s) - started ))"
