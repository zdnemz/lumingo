import type { Browser } from "@e2e-dev/web";

/**
 * The one retrying settle every browser test shares.
 *
 * **Why a retry and not a longer wait.** A step that taps a link and then asserts
 * the URL is done as soon as the URL changes, and that is before the new document
 * has finished swapping in: the old document is gone and the new one is not yet
 * readable. A locator read aimed into that gap does not fail fast — it hangs for
 * its entire timeout and reports **zero reads**, because not one query ever
 * reached a document. That is not a node that is slow to appear, so a longer
 * timeout cannot help it: it only gives the same hung read more time to hang, and
 * raising it is the mistake that made an earlier attempt at this look fixed. A
 * *fresh* read against the document that is current now is what escapes it, so
 * both exports here re-run their read until it answers, and only the total budget
 * ends the wait.
 *
 * **What the helper does not do.** It changes *when* a dead read fails, never
 * *whether* a real failure fails. A rejection is retried only when it is a read
 * that never reached a document — an `ACTION_FAILED` or a message about a
 * timeout. A wrong value, a wrong count or a wrong URL throws a comparison error,
 * which is rethrown at once and unchanged. When the budget does run out, the
 * last real error is what surfaces — its own code, its own stack, its own
 * message — so a genuinely absent node still fails as itself, after the budget
 * instead of after one read's timeout, and never as a timeout of this helper's.
 */

/**
 * The total budget for one settle, the first slice of it one attempt takes, the
 * ceiling that slice grows to, and the pause between attempts.
 *
 * The budget is well above the 15 s a single wait was given, because what it
 * buys is another *read*, not a longer wait: see above for why that distinction
 * is the whole fix. The budget is measured, not guessed: the longest stall seen
 * at one call site — the navigation right after the export download — ran four
 * consecutive dead reads, 32 s of no answer at all, and then settled. A 30 s
 * budget gave up in the middle of exactly that stall and failed the run, so the
 * budget has to clear it. Thirty-five seconds does, with margin, and is also as
 * large as this can safely go: the stall costs the budget in full, and the whole
 * spec still has to finish inside its own attempt deadline.
 *
 * The attempt grows because the two ways a read fails here pull in opposite
 * directions, and no single size serves both.
 *
 * - A read that lands in the document swap hangs and burns its **whole** timeout
 *   without answering — measured at 6001 ms of a 6 s attempt, twice in a row, in
 *   a run that then settled on the next read. Only a *fresh* read escapes it, so
 *   a short first attempt notices the swap ending soonest.
 * - A read on a loaded machine is merely slow and needs an attempt long enough to
 *   finish. The longest honest wait seen here is 5.3 s, so a probe that only ever
 *   tried 2 s would cut that read off forever and never settle at all.
 *
 * So the slice starts short, to get past a swap quickly, and doubles after each
 * failure up to the ceiling, so a genuinely slow read still gets an attempt it
 * can finish in. The pause is a cadence between reads, not a guess at how long a
 * swap takes.
 */
const SETTLE_BUDGET_MS = 35_000;
const SETTLE_PAUSE_MS = 150;
const SHELL_PROBE_FIRST_ATTEMPT_MS = 2_000;
const SHELL_PROBE_ATTEMPT_CEILING_MS = 10_000;

/**
 * Whether a rejection is a read that never reached a document — the one failure a
 * fresh read escapes — rather than an answer this helper must not touch.
 */
function isDeadRead(cause: unknown): boolean {
  if (typeof cause !== "object" || cause === null) return false;
  const { code, message } = cause as { code?: unknown; message?: unknown };
  if (code === "ACTION_FAILED") return true;
  return typeof message === "string" && /timed out|timeout/i.test(message);
}

/**
 * Runs one read until it answers, re-invoking it on a fresh locator each attempt.
 *
 * `read` is a thunk, not a promise: every attempt builds its locator against the
 * document that is current *now*, which is the mechanism. A healthy read costs
 * one call and no pause. `what` names the read, so a failure says which one never
 * answered.
 */
export async function readSettled<T>(what: string, read: () => Promise<T>): Promise<T> {
  const deadline = Date.now() + SETTLE_BUDGET_MS;
  for (;;) {
    try {
      return await read();
    } catch (cause) {
      // Not a dead read: a wrong value, a wrong count, a real absence. Rethrow it
      // at once and unchanged — this helper must never turn a failure into a pass.
      if (!isDeadRead(cause)) throw cause;
      if (Date.now() + SETTLE_PAUSE_MS >= deadline) {
        // The budget is gone. Throw the last real error, not a timeout of this
        // helper's own: the code and the stack stay the read's, and `what` goes in
        // front of its own message so the report says which read never answered.
        if (cause instanceof Error) {
          try {
            cause.message = `${what} never answered: ${cause.message}`;
          } catch {
            // A frozen error object: the read's own message is still what surfaces.
          }
        }
        throw cause;
      }
      await new Promise((resolve) => setTimeout(resolve, SETTLE_PAUSE_MS));
    }
  }
}

/**
 * Waits until the shell has finished drawing, which is when nothing above the
 * page's own content is still moving.
 *
 * Every page mounts its own `AppShell`, and the export ships nothing but the
 * boot screen, so a navigation replaces the document and the shell is drawn
 * after it: the header and the heading first, then the banners once the server
 * snapshot arrives, and a banner pushes everything below it down by its own
 * height. Waiting for that region is waiting for the last thing above the
 * content to stop moving, and it is what keeps the next measurement from being
 * taken against a page that is still arriving.
 *
 * Every spec now waits through this one helper, so the empty region has to be a
 * settled state: offline before a profile is saved a banner is drawn, and after
 * one is saved the shell honestly draws no banner at all, so waiting for content
 * would never end in that half. The wait is therefore for the shell to be drawn
 * and for the content under its banners to hold still, which is the same fact the
 * banner stood for and holds in both halves of a walk.
 *
 * The "content region holds still" measurement stays, because it is what makes
 * the helper right at the call sites that navigate and at the ones that do not:
 * being drawn is not the same as being finished drawing.
 */
export async function shellSettled(browser: Browser): Promise<void> {
  let attemptMs = SHELL_PROBE_FIRST_ATTEMPT_MS;
  await readSettled("the shell to finish drawing", async () => {
    try {
      // Only that the shell is drawn: after a provider profile is saved this
      // region is legitimately empty, so its *content* is never what is waited for.
      await browser.locator(".shell__banners").waitFor({ state: "attached", timeout: attemptMs });
      const main = browser.locator(".shell__main");
      let box = await main.boundingBox();
      if (box === null) throw new Error("the shell has no content region to measure");
      for (let attempt = 0; attempt < 20; attempt += 1) {
        await new Promise((resolve) => setTimeout(resolve, 100));
        const again = await main.boundingBox();
        if (again === null) throw new Error("the shell's content region went away");
        const steady =
          again.x === box.x && again.y === box.y && again.width === box.width && again.height === box.height;
        box = again;
        if (steady) break;
      }
    } catch (cause) {
      // A read that landed in the document swap. Give the next attempt more room,
      // in case this was a slow read rather than a dead one; `readSettled` is what
      // re-runs the probe from the top, on a fresh read, for as long as the budget.
      attemptMs = Math.min(attemptMs * 2, SHELL_PROBE_ATTEMPT_CEILING_MS);
      throw cause;
    }
  });
}
