import { afterEach, test, type Browser } from "@e2e-dev/web";
import { expect, type Locator, type Screen } from "e2e";
import { shellSettled } from "./shell";

/**
 * The writing workshop, end to end, against the offline server
 * `pnpm test:e2e` starts by itself.
 *
 * The spec reaches the screen through the menu, which is what proves the nav
 * entry end to end. Writing is the one free mode that works offline: a session
 * with the learner's own prompt starts without a provider, and layer one — the
 * word count and the rule checker's findings — comes back from the server at
 * once. The draft here carries the mistake `docs/GRAMMAR_CHECK.md` measured as
 * the one the linked checker really catches ("This is an test." → an article
 * finding), so the findings list is non-empty for a reason the project recorded
 * rather than a reason this spec invented.
 *
 * Layers two and three — the errors and the rubric bands — need a provider.
 * Offline the server stores the draft and answers `status: "pending"`, which the
 * screen shows as waiting. The spec asserts exactly that and checks that nothing
 * invents a verdict: the findings are the server's own sentences, so the spec
 * asserts properties of that list (non-empty, every entry non-empty) and never a
 * particular harper sentence and never an exact count, and it checks that no
 * rubric band and no revision comparison is claimed while the provider is
 * missing. The session is then finished through the screen's own control, so the
 * next spec finds the server's single session slot free.
 *
 * Interactions use `screen.tapAt` at a point inside the node rather than
 * `locator.tap`. Both are the same learner gesture, but a locator tap first
 * waits for the node to hold still across animation frames, and this headless
 * browser stops producing frames once the page has settled and nothing is
 * painted — so that wait never ends. A point tap is dispatched straight at the
 * point the node reports, which is what a person's finger does. Typing uses
 * `fill`, which needs no frames. The page is laid out in a viewport tall enough
 * to hold the whole run, so a point is always on screen and nothing has to be
 * scrolled.
 */

/** A viewport tall enough that the whole screen fits: no scrolling, so a point tap always lands. */
const TALL_VIEWPORT = { width: 1280, height: 6000 };

/** The task the learner sets, and the draft handed in. */
const PROMPT = "Write about my city.";
/** The draft carries the measured catch of `docs/GRAMMAR_CHECK.md`: the article in "an test". */
const DRAFT = "This is an test. I am from Surabaya and I live near the market.";

/** The screen's own strings, from `apps/web/src/i18n/parts/write.en.ts` and `shell.en.ts`. */
const TEXT = {
  /** `nav.write`: the menu entry this spec walks through. */
  navWrite: "Write",
  /** `write.title`, the page's own heading. */
  title: "Writing workshop",
  /** `write.prompt.label`: the label of the task field, and the title of the panel after the start. */
  promptLabel: "Your task",
  /** `write.start`. */
  start: "Start writing",
  /** `write.draft.label`: the label of the draft editor. */
  draftLabel: "Your draft",
  /** `write.check`. */
  check: "Check this draft",
  /** `write.finish`. */
  finish: "Finish the session",
  /** `write.ended.title`, the panel the finished session shows. */
  ended: "Session finished",
  /** `write.rules.title`, the heading of layer one. */
  rulesTitle: "What the rule checker found",
  /** `write.feedback.title`, the heading of layers two and three. */
  feedbackTitle: "Errors and bands",
  /** `write.feedback.pending.title`: the server answered `status: "pending"`, so the screen says it waits. */
  pendingTitle: "Waiting for an AI provider",
  /** `write.feedback.pending.body`, the same state in a sentence. */
  pendingBody: "The draft is stored. Its errors and bands are added when an AI provider is available.",
  /** `write.rules.notchecked`, which must not appear: the server's checker is linked and it did run. */
  notChecked: "Not checked: this program has no rule checker, so nothing about this draft was checked.",
  /** `write.rules.none`, which must not appear: the measured draft has a finding. */
  noFindings: "The rule checker found nothing to mark.",
  /** `write.rubric.title`: no rubric band may be claimed offline. */
  rubricTitle: "Bands",
  /** `write.revision`: no revision comparison may be claimed offline. */
  revision: "Compared with your first draft",
  /** `write.words`: the server's own count of the draft. */
  words: (count: number) => `Words, as the program counted them: ${count}`,
};

/** The middle of a node: the point a learner's finger lands on. */
async function tapAt(screen: Screen, locator: Locator): Promise<void> {
  const box = await locator.boundingBox();
  if (box === null) throw new Error("the node has no box to tap");
  await screen.tapAt({ x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

/**
 * How many words the server counts in a draft: `split_whitespace` over the
 * trimmed text, which is the rule of `Workshop::submit_draft`. The spec derives
 * the expected count from the draft it typed instead of hard-coding a number, so
 * the two cannot disagree.
 */
function serverWordCount(text: string): number {
  return text.trim().split(/\s+/).filter((part) => part !== "").length;
}

/**
 * The id of the session the server is holding, read through the page's own state
 * route. `null` means the server has no session, which is the state every spec
 * has to leave it in: the server runs one session at a time and refuses a second
 * start with `Conflict`.
 */
async function activeSessionId(browser: Browser): Promise<number | null> {
  return (await browser.evaluate(async () => {
    const state = await (await fetch("/api/state")).json();
    return state.active_session === null ? null : state.active_session.id;
  })) as number | null;
}

/**
 * Whatever this spec did, the next one must find the server's single session
 * slot free. The walk below finishes its own session through the screen's own
 * control, and this is the belt to that brace: an assertion that fails half way
 * through the walk must not leave a session running for the reading or
 * conversation spec to trip over. A page that never reached the app — a failure
 * on the very first step — has no origin to ask, and then there is nothing this
 * spec could have started either.
 */
afterEach(async ({ browser }) => {
  try {
    const running = await activeSessionId(browser);
    if (running === null) return;
    // The status is returned rather than dropped: `browser.evaluate` carries
    // JSON-safe values across, and a `void` is not one.
    await browser.evaluate(async (sessionId) => {
      const response = await fetch(`/api/sessions/${sessionId}/stop`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
      });
      return response.status;
    }, running);
  } catch {
    // The page never reached the app, so it holds no session of this spec's.
  }
});

test("a draft is checked by rules offline and its provider feedback is honestly pending", async ({
  app,
  browser,
  screen,
}) => {
  // The learner reads in English, which is the language these strings are read
  // in. Nothing else is changed: the settings are the state a learner has.
  await browser.addInitScript(() => {
    try {
      const stored = JSON.parse(localStorage.getItem("lumingo.prefs.v1") ?? "{}") as Record<string, unknown>;
      localStorage.setItem("lumingo.prefs.v1", JSON.stringify({ ...stored, language: "en" }));
    } catch {
      // Storage can be blocked; the page works without it.
    }
  });
  await browser.setViewport(TALL_VIEWPORT);

  // 1. The screen is reached through the menu.
  await app.open("/unit/");
  await shellSettled(browser);
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navWrite }));
  await expect(browser).toHaveURL("/write/", { timeout: 15_000 });
  await shellSettled(browser);
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.title })).toBeVisible();

  // 2. The learner sets the task and starts. A session with the learner's own
  //    prompt needs no provider, so the start works offline. The start control
  //    is enabled only once the field holds the prompt, so the tap waits for
  //    that: a tap on a disabled button would land and do nothing.
  const prompt = screen.getByLabel(TEXT.promptLabel);
  await expect(prompt).toBeVisible();
  await prompt.fill(PROMPT);
  const start = screen.getByRole("button", { name: TEXT.start });
  await expect(start).toBeEnabled();
  await tapAt(screen, start);
  const editor = screen.getByLabel(TEXT.draftLabel);
  await expect(editor).toBeVisible({ timeout: 15_000 });
  await expect(screen.getByText(PROMPT)).toBeVisible();

  // 3. The draft with the measured catch is handed in. The check control is
  //    enabled only once the editor holds text, so the tap waits for that.
  await editor.fill(DRAFT);
  const check = screen.getByRole("button", { name: TEXT.check });
  await expect(check).toBeEnabled();
  await tapAt(screen, check);

  // 4. Layer one came back: the draft as it was handed in, and the server's own
  //    count of it, derived from the text this spec typed.
  const handedIn = screen.getByText(DRAFT);
  await expect(handedIn).toBeVisible({ timeout: 15_000 });
  await expect(screen.getByText(TEXT.words(serverWordCount(DRAFT)))).toBeVisible();

  // 5. The rule checker ran and found something: the findings list is non-empty
  //    and every entry carries the server's own sentence. The sentences are the
  //    server's, so this spec asserts the properties of the list and never a
  //    particular harper sentence and never an exact count.
  await expect(screen.getByRole("heading", { name: TEXT.rulesTitle })).toBeVisible();
  expect(await screen.getByText(TEXT.notChecked).count()).toBe(0);
  expect(await screen.getByText(TEXT.noFindings).count()).toBe(0);
  const findings = browser.locator("ul.write__findings > li");
  const sentences = await findings.allTextContents();
  expect(sentences.length).toBeGreaterThan(0);
  for (const sentence of sentences) expect(sentence.trim().length).toBeGreaterThan(0);

  // 6. Layers two and three need a provider. Offline the server stored the draft
  //    and answered `status: "pending"`, so the screen says it is waiting for one
  //    instead of showing an error or an invented verdict.
  await expect(screen.getByRole("heading", { name: TEXT.feedbackTitle })).toBeVisible();
  await expect(screen.getByRole("heading", { name: TEXT.pendingTitle })).toBeVisible({ timeout: 30_000 });
  await expect(screen.getByText(TEXT.pendingBody)).toBeVisible();

  // 7. Nothing is claimed that the server did not send: no rubric band and no
  //    revision comparison, and no error marked in the draft either.
  expect(await screen.getByRole("heading", { name: TEXT.rubricTitle }).count()).toBe(0);
  expect(await screen.getByText(TEXT.revision).count()).toBe(0);
  expect(await browser.locator("ul.write__bands").count()).toBe(0);
  expect(await browser.locator(".revision").count()).toBe(0);
  expect(await browser.locator(".annotated__text").count()).toBe(0);

  // 8. The session is finished through the screen's own control, and the screen
  //    reports the end the server answered with.
  await tapAt(screen, screen.getByRole("button", { name: TEXT.finish }));
  await expect(screen.getByRole("heading", { name: TEXT.ended })).toBeVisible({ timeout: 30_000 });

  // 9. Nothing is left running for the next spec: the server's one session slot
  //    is free again.
  expect(await activeSessionId(browser)).toBeNull();
});
