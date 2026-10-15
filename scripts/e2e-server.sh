#!/usr/bin/env sh
# The offline server `pnpm test:e2e` runs against.
#
# `e2e.config.ts` declares this script as its target's `command`, so the runner
# reserves a free port, hands it over as `{port}` and polls the URL until this
# script's server answers. The harness gives a command child only `PATH`,
# `HOME` and the temp-directory variables, so nothing here may depend on the
# developer's shell: the repository is found from this script's own path, and
# the server is built before it is started, so a first run is a cold build of
# the whole workspace.
#
# The server itself runs from a fresh temporary working directory with a fresh
# data directory and the example curriculum, which is what makes the suite
# offline: the LLM client reads `.env` from the working directory, and a data
# directory that was just created holds neither a provider profile nor a
# learner database, so the program has no provider and calls no model.
set -eu

root="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"
port="$1"

cd "$root"

# The debug server serves the UI from `apps/web/out` at its compile-time path,
# so the export has to exist before the server is built.
pnpm --dir apps/web build
cargo build -p tutor-server

# Absolute paths for everything the server reads: the working directory is a
# temporary one, and `curriculum/units` is intentionally empty, so the example
# unit is the curriculum the run indexes.
cd "$(mktemp -d)"
exec "$root/target/debug/tutor-server" \
    --port "$port" \
    --no-open \
    --data-dir "$(mktemp -d)" \
    --curriculum-dir "$root/curriculum/examples"
