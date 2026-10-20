# Lumingo v1.0.0 release: the owner's steps

Four steps, in order, by hand. Each step says the exact command, what to look
for, and what to write down. Nothing below has been done for you.

## What this run did and did not do

- **The run pushed nothing, tagged nothing, and created no release.** There is
  no `v1.0.0` tag and no draft release anywhere. The four steps below are
  yours to run.
- The local `development` branch is **5 commits ahead of `origin/development`**:
  the local tip is `22f2123`, and `origin/development` is at `b6102bf`. Those
  are the numbers as of writing — check your own clone before you trust them.
- **Pushing is the owner's decision**, so the run did not push. The five
  commits sit on this machine until you push them.
- **The Windows build has never been executed by an agent.** The release
  workflow runs on `windows-latest` in GitHub CI, and this run could not run
  CI. The Windows half of this release is therefore **reviewable but
  unproven**: nobody has run the Windows binary, and the pipeline has not run
  either.
- The only local evidence is a **Linux stand-in build** — the same packaging
  code, on Linux, not the Windows artifact. Nothing in this work says the
  Windows binary works.
- The workflow, the packaging script, the export guard and the release README
  are part of this release; none of them has ever run on Windows.

Do not treat the release as ready, and do not report CI as green. No CI run
happened in this work, and none of the four steps below has been carried out.

## Step 1 — push

Push the five commits that are sitting on this machine.

```sh
git push origin development
```

Look for:

- the output ends with `development -> development` (or `Everything up-to-date`
  if your clone is already in this state);
- `git status -sb` shows `## development...origin/development` with **no**
  ahead count.

Write down: the commit you pushed (`git rev-parse --short HEAD`, expect
`22f2123`) and the date.

## Step 2 — tag v1.0.0

PRD line 358: "Tag `v1.0.0` exists with the release zip and checksums
attached."

The tag must point at the commit you pushed in step 1, so step 1 comes first.
Tag the branch you just pushed, then push the tag.

```sh
git tag -a v1.0.0 -m "Lumingo v1.0.0"
git push origin v1.0.0
```

Look for:

- `git tag -l` lists `v1.0.0`;
- `git rev-parse v1.0.0^{commit}` gives the same commit as
  `git rev-parse origin/development` — the tag and the pushed branch sit on one
  commit;
- the push output says `new tag 'v1.0.0'`.

Write down: the tag name and the commit it points at.

## Step 3 — the draft release

Pushing the tag triggers `.github/workflows/release.yml`. It runs on
`windows-latest`: it exports the web app, builds the server for
`x86_64-pc-windows-msvc`, packages the zip, checks the zip's size and its
members, smoke-tests the executable, and then creates a **draft** release.
The draft is not published — you publish it.

Look for, in the repository's Actions tab, a green `release` run for the tag
`v1.0.0`, and then a **draft** release titled `Lumingo 1.0.0` holding two
assets:

- `lumingo-1.0.0-windows-x64.zip` — the artifact;
- `checksums.txt` — a **sibling** of the zip, in `sha256sum` format: one line
  per member, `<64 hex digits><two spaces><member name>`. It covers the five
  members inside the zip — `.env.example`, `LICENSE`, `NOTICE`, `README.md`
  and `lumingo.exe` — and has no line for the zip itself.

Inside `lumingo-1.0.0-windows-x64.zip`, the executable is named `lumingo.exe`.

The draft is still a draft. Publish it yourself (the release page's
"Publish release" button) once you are satisfied with it — that is what makes
the release public. PRD line 358 is satisfied only after that: the tag
`v1.0.0` must exist with the release zip and checksums attached.

Write down: the release URL, and the commit the run built from.

## Step 4 — the clean-account walk

On a **fresh Windows 11 account with no Node and no Rust installed** — the
point is that the learner needs nothing but the zip.

1. Download `lumingo-1.0.0-windows-x64.zip` and `checksums.txt` from the
   release.
2. **Check the sums before you unpack or run anything.** Unpack the zip and
   compare each of the five files' SHA-256 sums with the line for it in
   `checksums.txt` (`Get-FileHash -Algorithm SHA256` in PowerShell,
   `sha256sum` in Git Bash). All five must match.
3. Run `lumingo.exe` from the unpacked folder.
4. The browser opens on `http://127.0.0.1:8765` and the program starts. If the
   browser does not open, open that address by hand.
5. Windows shows a SmartScreen warning: the program is **unsigned**, so
   Windows cannot check who published it. The README in the zip explains it —
   choose "More info", then "Run anyway".
6. Follow the README in the zip: set up the provider (it names the four
   settings the program needs) and test the connection. Check that the program
   runs.

**What this walk can and cannot prove.**

**It cannot prove that unit A1-01 can be completed.** The zip carries **no
curriculum** — the curriculum is not one of the five members, and the shipped
program starts with an empty unit list. So PRD §13.8 ("a fresh Windows 11
account can … complete unit A1-01 …, following only the README") is **not
reachable from this artifact**, and no claim is made that it is. The walk
proves three things only: the program starts, it serves its UI, and a provider
can be set up. It cannot prove that a unit is completed.

**It cannot run the speaking parts.** This build has **no microphone** and no
speech engines — the audio features are off, so the speaking steps cannot be
run at all. Skip them; do not write them down as passed.

Write down: whether the sums matched, whether the program started, what the
browser showed, and anything that confused you.

## What to write down

After each step, record:

- **Step 1:** the commit hash you pushed, and the date.
- **Step 2:** the tag name, and the commit the tag points at.
- **Step 3:** the release URL, the release title, and the two asset names.
- **Step 4:** whether the five sums matched; whether `lumingo.exe` started;
  what the browser showed at `http://127.0.0.1:8765`; whether the provider
  was set up; and anything that confused you — the confusion is the point of
  the walk, not a failure of it.
- **The limits you did not test:** unit A1-01 (no curriculum in the zip) and
  the speaking parts (no microphone in this build). Write them down as
  untested, never as passed.

## If a step fails

**Step 1 — the push is rejected.** Either the remote has commits you do not
have (fetch and rebase: `git pull --rebase origin development`, then push
again) or your credentials have no write access to the branch. Nothing else
in the release can start until the branch is on the remote.

**Step 2 — the tag push is rejected.** The tag already exists, locally
(`git tag -l` shows `v1.0.0`) or on the remote. If it is local and points at
the wrong commit, delete it (`git tag -d v1.0.0`) and tag again. If it
already exists on the remote, do not force it: check where it points and
decide deliberately whether to delete the remote tag. The other cause is that
the branch was not pushed first: push it (step 1), then tag the pushed
commit, then push the tag again.

**Step 3 — the workflow fails.** Three gates fail on their own:

- *The export guard* (its step 5) exits non-zero and names the offender: a
  server feature was added to the web app — a `route.ts` / `middleware.ts` /
  `proxy.ts` file, a `*.server.ts` file, a `"use server"` line, or a
  `next.config` change that breaks the static export. Move that feature out
  of `apps/web` or remove it, then push the tag again.
- *The size gate* (its step 10) fails: the zip is over 100 MB (the workflow
  fails above 100000000 bytes). The fix is a release-profile change in the
  root `Cargo.toml` (`lto = "thin"`, maybe `strip = "symbols"`), and it also
  moves the benchmark profile, so it is a follow-up decision, not something to
  rush inside the release.
- *The member and checksum check* (its step 11) fails: the unpacked file
  names are not exactly the five, or a file's SHA-256 does not match its line
  in `checksums.txt`. The packaging and the check disagree, so the artifact
  is not publishable. Do not publish the draft; fix the packaging and re-run
  the workflow.

If the smoke test (its step 12) fails with "has not been built" in the page,
the web export was not embedded in the executable — the export has to be
built before the server build. If the run never started at all, check that
the tagged commit actually contains the workflow:
`git ls-tree v1.0.0 .github/workflows/release.yml` must print a line. If the
draft release is missing but the run was green, that run was not a tag push
(a `workflow_dispatch` run creates no release).

**Step 4 — the sums do not match.** The download is corrupted: download the
zip and `checksums.txt` again and check again. Do not run an artifact whose
sums do not match. If they still do not match, the published artifact and the
checksums are not the same build — report it and leave the release as a
draft.

**Step 4 — the program does not start.** Read the error in the console
window; check that the account may run the unpacked program, and that the
machine meets the README's minimum spec. A program that starts but cannot
reach the provider is a provider problem, not a packaging problem — record it,
and check the provider settings the README names.
