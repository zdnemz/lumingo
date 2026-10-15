import { afterEach, test, type Browser } from "@e2e-dev/web";
import { expect, type Locator, type Screen } from "e2e";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

/**
 * Graded reading, end to end, against the offline server `pnpm test:e2e` starts
 * by itself.
 *
 * The spec reaches the screen through the menu, which is what proves the nav
 * entry end to end. Offline there is no provider, so the honest outcome of a
 * request for a text is the authored fallback: the reading sets already written
 * in the units of the learner's level, with the server's own reason for offering
 * them instead of a text a model wrote. Everything the spec asserts about that
 * set — the passage, the questions, their options, the authored `answer_index`
 * values and the explanation texts — is read from the unit file the server
 * indexed, so the spec cannot disagree with the curriculum and never hard-codes
 * a question. Every question is answered with its authored answer, so the score
 * is the whole set right, and every explanation is shown.
 *
 * One thing this screen does not do, recorded here rather than papered over:
 * it starts a server session and offers no control that ends one — the writing
 * and conversation screens each have a finish button, this screen has none, so
 * the server's single session slot stays taken until something else stops it.
 * The spec measures that (no such control, and the server still holding the
 * session) and then frees the slot through the same route every screen's client
 * uses for it, because the suite shares one server and the next spec would be
 * refused a start with `Conflict`. It is a gap in the screen, not a gap in the
 * walk.
 *
 * Interactions use `screen.tapAt` at a point inside the node rather than
 * `locator.tap`. Both are the same learner gesture, but a locator tap first
 * waits for the node to hold still across animation frames, and this headless
 * browser stops producing frames once the page has settled and nothing is
 * painted — so that wait never ends. A point tap is dispatched straight at the
 * point the node reports, which is what a person's finger does. Typing uses
 * `fill`, which needs no frames. The page is laid out in a viewport tall enough
 * to hold the whole text and its questions, so a point is always on screen and
 * nothing has to be scrolled.
 */

/** The unit file `scripts/e2e-server.sh` indexes, relative to the project root. */
const UNIT_FILE = "curriculum/examples/a1-u01.example.json";

/** A viewport tall enough that the whole screen fits: no scrolling, so a point tap always lands. */
const TALL_VIEWPORT = { width: 1280, height: 6000 };

/** The topic the learner types. Offline no text is written, so it only has to be a topic. */
const TOPIC = "a day at the market";

/** One question of the authored reading set, as far as this spec reads it. */
interface AuthoredQuestion {
  stem: string;
  options: string[];
  answer_index: number;
  explanation: { en: string; id?: string };
}

interface AuthoredActivity {
  id: string;
  type: string;
  /** `reading_set`. */
  passage?: string;
  questions?: AuthoredQuestion[];
}

interface AuthoredUnit {
  id: string;
  level: string;
  title: { en: string; id?: string };
  activities: AuthoredActivity[];
}

const unit = JSON.parse(readFileSync(resolve(process.cwd(), UNIT_FILE), "utf8")) as AuthoredUnit;

/**
 * The reading set the offline server offers: the authored `reading_set` of the
 * example unit, which is the whole curriculum `scripts/e2e-server.sh` indexes.
 * A second one would mean this spec walks a screen the run does not show, so the
 * walk refuses rather than quietly asserting against the wrong set.
 */
function authoredSet(): { id: string; passage: string; questions: AuthoredQuestion[] } {
  const sets = unit.activities.filter((activity) => activity.type === "reading_set");
  if (sets.length !== 1) {
    throw new Error(`${UNIT_FILE} authors ${sets.length} reading sets; the offline run offers every set of the level`);
  }
  const set = sets[0];
  if (set === undefined || set.passage === undefined || set.questions === undefined) {
    throw new Error(`${UNIT_FILE} has a reading set with no passage or no questions`);
  }
  return { id: set.id, passage: set.passage, questions: set.questions };
}

const set = authoredSet();

/** The unit file writes text in blocks; a browser collapses that to single spaces. */
const flat = (text: string): string => text.replace(/\s+/g, " ").trim();

/** The screen's own strings, from `apps/web/src/i18n/parts/read.en.ts`, `unit.en.ts` and `shell.en.ts`. */
const TEXT = {
  /** `nav.read`: the menu entry this spec walks through. */
  navRead: "Read",
  /** `read.title`, the page's own heading. */
  title: "Graded reading",
  /** `read.topic`: the label of the topic field. */
  topic: "What to read about",
  /** `read.start`, the button that starts the session and asks for the first text. */
  start: "Get a text",
  /** `read.fallback.title`, the panel the authored sets are shown in. */
  fallbackTitle: "Texts from your units",
  /** `read.reason.provider_unavailable`: the server's own reason for the fallback. */
  reason:
    "The AI provider could not be reached, so no new text was written. These texts are already written and need no provider.",
  /** `read.level.fixed`: the level the server fixed when the session started. */
  levelFixed: (level: string) => `This text is at level ${level}. To read at another level, open the reading page again.`,
  /** `read.set.from`: which unit's set this is. */
  setFrom: (unitId: string) => `From unit ${unitId}`,
  /** `read.text`, the accessible label of the passage. */
  passageLabel: "Reading text",
  /** `activity.check`. */
  check: "Check",
  /** `read.score`. */
  score: (correct: number, total: number) => `${correct} of ${total} correct`,
  /** `activity.why`, the line each question's explanation is shown on. */
  why: (text: string) => `Why: ${text}`,
};

/** The middle of a node: the point a learner's finger lands on. */
async function tapAt(screen: Screen, locator: Locator): Promise<void> {
  const box = await locator.boundingBox();
  if (box === null) throw new Error("the node has no box to tap");
  await screen.tapAt({ x: box.x + box.width / 2, y: box.y + box.height / 2 });
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
 * Ends a session through `POST /api/sessions/{id}/stop`, the route every
 * screen's client calls for its own finish button. This screen has no such
 * button, so the spec calls the route the same way that client does: same
 * origin, JSON content type, the page's own cookie. Returns the HTTP status.
 */
async function stopSession(browser: Browser, id: number): Promise<number> {
  return (await browser.evaluate(async (sessionId: number) => {
    const response = await fetch(`/api/sessions/${sessionId}/stop`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
    });
    return response.status;
  }, id)) as number;
}

/**
 * Whatever this spec did, the next one must find the server's single session
 * slot free. The walk below stops its own session, and this is the belt to that
 * brace: an assertion that fails half way through the walk must not leave a
 * session running for the writing or conversation spec to trip over. A page
 * that never reached the app — a failure on the very first step — has no origin
 * to ask, and then there is nothing this spec could have started either.
 */
afterEach(async ({ browser }) => {
  try {
    const running = await activeSessionId(browser);
    if (running !== null) await stopSession(browser, running);
  } catch {
    // The page never reached the app, so it holds no session of this spec's.
  }
});

test("the authored reading set is offered offline and every authored answer scores", async ({ app, browser, screen }) => {
  // The learner reads in English, which is the language the unit file's
  // explanations and these strings are read in.
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
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navRead }));
  await expect(browser).toHaveURL("/read/", { timeout: 15_000 });
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.title })).toBeVisible();

  // 2. A level and a topic. The level the session starts at is the unit file's
  //    own level, so the server's fallback is the set this spec derived. The
  //    start control is enabled only once the topic field holds text, so the tap
  //    waits for that.
  const topic = screen.getByLabel(TEXT.topic);
  await expect(topic).toBeVisible();
  await topic.fill(TOPIC);
  const start = screen.getByRole("button", { name: TEXT.start });
  await expect(start).toBeEnabled();
  await tapAt(screen, start);

  // 3. No provider, so no text was written: the screen shows the authored sets
  //    of the learner's level with the server's own reason, never an invented
  //    text and never an error.
  await expect(screen.getByText(TEXT.fallbackTitle)).toBeVisible({ timeout: 15_000 });
  await expect(screen.getByText(TEXT.reason)).toBeVisible();
  await expect(screen.getByText(TEXT.levelFixed(unit.level))).toBeVisible();
  await expect(screen.getByText(TEXT.setFrom(unit.id))).toBeVisible();

  // 4. The set on screen is the authored one: its title is the unit's own, and
  //    the passage the file wrote is shown in full.
  await expect(screen.getByRole("heading", { name: unit.title.en })).toBeVisible();
  await expect(screen.getByLabel(TEXT.passageLabel)).toContainText(flat(set.passage));

  // 5. Every authored question and every authored option is on screen, in the
  //    radio group the question's own stem names.
  for (const question of set.questions) {
    await expect(screen.getByRole("group", { name: question.stem })).toBeVisible();
    for (const option of question.options) {
      await expect(screen.getByRole("group", { name: question.stem }).getByRole("radio", { name: option })).toBeVisible();
    }
  }

  // 6. Every question is answered with the answer the unit file authored.
  for (const question of set.questions) {
    const choice = question.options[question.answer_index];
    if (choice === undefined) throw new Error(`${set.id} question "${question.stem}" has no authored choice`);
    await tapAt(screen, screen.getByRole("group", { name: question.stem }).getByRole("radio", { name: choice }));
  }

  // 7. The score is the server's, and the whole set is right: the authored
  //    answers were handed in, so `correct` is the question count.
  await tapAt(screen, screen.getByRole("button", { name: TEXT.check }));
  const total = set.questions.length;
  await expect(screen.getByText(TEXT.score(total, total))).toBeVisible({ timeout: 15_000 });

  // 8. Every explanation is shown, one line per question, and each is the text
  //    the unit file authored for that question.
  const whyLines = await browser.locator("p.read-question__why").allTextContents();
  expect(whyLines).toHaveLength(total);
  for (const question of set.questions) {
    const line = TEXT.why(question.explanation.en);
    expect(whyLines).toContain(line);
    await expect(screen.getByText(line).first()).toBeVisible();
  }

  // 9. This screen offers no control that ends the session it started, so the
  //    server still holds it. That is the screen's real behaviour, measured
  //    rather than assumed: a learner who leaves this page leaves the session
  //    running.
  expect(await screen.getByRole("button", { name: /finish|stop|end the/i }).count()).toBe(0);
  const running = await activeSessionId(browser);
  expect(running).not.toBeNull();

  // 10. The slot has to be free for the next spec, so the session is stopped
  //     through the route the screen's client would have used, and the slot is
  //     asserted free again.
  if (running !== null) expect(await stopSession(browser, running)).toBe(200);
  expect(await activeSessionId(browser)).toBeNull();
});
