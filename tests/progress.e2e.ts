import { afterEach, test, type Browser } from "@e2e-dev/web";
import { expect, type Locator, type Screen } from "e2e";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

/**
 * The progress screen, end to end, against the offline server `pnpm test:e2e`
 * starts by itself.
 *
 * The spec reaches the screen through the menu, which is what proves the nav
 * entry end to end, and then walks the page in the order a learner meets it:
 * the empty state first, then real state made through the lesson player, then
 * the drill-down from a skill to the counted attempt behind it and to that
 * attempt's stored evidence.
 *
 * Everything the spec asserts about the curriculum is read from the unit file
 * the server indexed, so it cannot disagree with it: the offered activities,
 * their authored answers, the first reading attempt (`a13-read-budi`, an mcq
 * with a passage, so the server files its evidence under reading) and the option
 * that attempt stores as its answer. The counts it asserts are the ones the
 * server really sent, read back through the page's own routes, never a fixed
 * number written into the spec.
 *
 * Two honest states of the offline server shape the walk, and both are the
 * server's behaviour rather than this spec's choice:
 *
 * - The run is stopped before its checkpoint is finished, so the server closes
 *   it as aborted (`UnitPlayer::finish` refuses an incomplete checkpoint) and
 *   `enrol_review_items` never runs for it. **No review item is enrolled, so the
 *   review queue stays empty**, and the populated page still shows
 *   `progress.reviews.empty`. That is the honest offline reason the queue is
 *   empty, not a missing assertion.
 * - One reading task is far from the estimation rule's evidence requirement, so
 *   all four skills keep the insufficient sentence and **no level appears
 *   anywhere on the page**. The populated walk asserts that too.
 *
 * The suite shares one server, and the specs run in file order, so by the time
 * this one starts the lesson-player spec has already left a unit run behind. The
 * empty state is therefore established the way the program itself establishes
 * it: the learner's record is cleared through `DELETE /api/data`, the route the
 * privacy screen's own delete control calls, and then the state a fresh data
 * directory has is asserted. A leftover session is stopped first, because the
 * server refuses that delete with `Conflict` while one runs.
 *
 * The drill-down's own honest states are each driven for real: loading and a
 * failed read with its retry through an attempt-scoped route, the empty state of
 * a skill with nothing counted, the truncated state by making the server count
 * more attempts than its list holds, and the pending state of a response the
 * provider never scored. Nothing is faked: every row, number and sentence comes
 * from the server through the page.
 *
 * Interactions use `screen.tapAt` at a point inside the node rather than
 * `locator.tap`. Both are the same learner gesture, but a locator tap first
 * waits for the node to hold still across animation frames, and this headless
 * browser stops producing frames once the page has settled and nothing is
 * painted — so that wait never ends. A point tap is dispatched straight at the
 * point the node reports, which is what a person's finger does. Typing uses
 * `fill`, which needs no frames. The page is laid out in a viewport tall enough
 * to hold the whole screen, so a point is always on screen and nothing has to
 * be scrolled.
 */

/** The unit file `scripts/e2e-server.sh` indexes, relative to the project root. */
const UNIT_FILE = "curriculum/examples/a1-u01.example.json";

/** A viewport tall enough that the whole screen fits: no scrolling, so a point tap always lands. */
const TALL_VIEWPORT = { width: 1280, height: 6000 };

/** The first reading attempt of the unit: an mcq with a passage, so its evidence skill is reading. */
const READING_ATTEMPT = "a13-read-budi";

/** The writing task the run stores while it waits for a provider: one response, four pending dimensions. */
const WRITING_ATTEMPT = "a12-write-introduce";

/** One activity of the unit file, as far as this spec reads it. */
interface AuthoredActivity {
  id: string;
  type: string;
  instructions: { en: string; id?: string };
  /** `mcq`. */
  stem?: string;
  options?: string[];
  answer_index?: number;
  /** `gap_fill`: one accepted group per gap, the authored answer first. */
  answers?: string[][];
  /** `reorder`. */
  tokens?: string[];
  answer?: string;
  /** `match`. */
  pairs?: { left: string; right: string }[];
  /** `reading_set`. */
  questions?: { stem: string; options: string[]; answer_index: number }[];
  /** `dictation` and `error_correction`. */
  accepted_answers?: string[];
  /** `guided_writing`. */
  model_answers?: { band: string; text: string }[];
}

interface AuthoredUnit {
  id: string;
  sequence: number;
  title: { en: string; id?: string };
  activities: AuthoredActivity[];
}

const unit = JSON.parse(readFileSync(resolve(process.cwd(), UNIT_FILE), "utf8")) as AuthoredUnit;

function activityById(id: string): AuthoredActivity {
  const found = unit.activities.find((activity) => activity.id === id);
  if (found === undefined) throw new Error(`${id} is not an activity of ${unit.id}`);
  return found;
}

/**
 * The reading attempt this spec walks to. A unit file whose `a13-read-budi` is
 * not an mcq with a passage would have its evidence filed under another skill
 * and the drill-down would list nothing, so the walk refuses rather than
 * quietly asserting against the wrong skill.
 */
const reading = activityById(READING_ATTEMPT);
if (reading.type !== "mcq" || reading.options === undefined || reading.answer_index === undefined) {
  throw new Error(`${READING_ATTEMPT} is not the authored mcq this spec walks to`);
}
/** The option the unit file authored as the answer: what the server stores as that attempt's evidence. */
const READING_ANSWER = reading.options[reading.answer_index];
if (READING_ANSWER === undefined) throw new Error(`${READING_ATTEMPT} has no authored choice`);

/** The authored text of the writing task, used for the runs that fill the attempts list. */
const writing = activityById(WRITING_ATTEMPT);
const WRITING_TEXT = writing.model_answers?.find((answer) => answer.band === "at")?.text;
if (WRITING_TEXT === undefined) throw new Error(`${WRITING_ATTEMPT} has no authored answer`);

/** The unit file writes text in blocks; a browser collapses that to single spaces. */
const flat = (text: string): string => text.replace(/\s+/g, " ").trim();

/**
 * A CEFR level, spelled as `apps/web/src/progress/ProgressScreen.test.tsx`
 * spells it. No level may appear anywhere on this page while every skill is
 * short of the estimation rule's evidence: a level is an estimate and belongs in
 * a sentence that says so, and a page with no estimate has none.
 */
const LEVEL = /\b(pre-)?[ABC][12]\b/;

/** The screen's own strings, from `apps/web/src/i18n/parts/{progress,unit,shell}.en.ts`. */
const TEXT = {
  /** `nav.progress`: the menu entry this spec walks through. */
  navProgress: "Progress",
  /** `progress.title`, the page's own heading. */
  title: "Progress",
  /** `progress.empty.title`, the banner a fresh data directory shows. */
  emptyTitle: "Nothing is recorded yet",
  /** `progress.empty.body`, the same state in a sentence. */
  emptyBody: "Finish a lesson and your work will show up on this page.",
  /** `skill.profile`, the accessible name of the four-skill list. */
  profile: "Your four skills",
  /** `skill.insufficient.general`, the sentence a skill short of evidence shows. */
  insufficient: (skill: string) => `Not enough evidence yet for ${skill}. More scored ${skill} tasks will help.`,
  /** `skill.evidence`: the control that opens a skill's drill-down; its name carries the skill. */
  evidence: (skill: string) => `See the work behind this estimate (${skill})`,
  /** `progress.drill.title`, the drill-down panel's own heading, which also names its region. */
  drillTitle: (skill: string) => `The work behind ${skill}`,
  /** `progress.drill.status.insufficient`, the drill-down's status fact for a skill without a level. */
  drillStatusInsufficient: "Not enough evidence yet",
  /** `progress.drill.attempts.intro`, the sentence that says what the attempts list is. */
  attemptsIntro: "Every counted attempt behind this estimate, newest first. Open one to see its stored evidence and quotes.",
  /** `progress.drill.attempts.empty`, the honest state of a skill with no counted attempt. */
  attemptsEmpty:
    "No counted attempts for this skill yet. Only authored lesson, checkpoint and placement tasks count; free practice and generated items never appear here.",
  /** `progress.drill.attempts.truncated`, the honest state of a list the server cut. */
  attemptsTruncated: "More than 50 attempts are stored. The 50 newest are shown.",
  /** `progress.drill.attempts.pending`: a row the server stored waiting for a provider, never as a score. */
  attemptsPending: "Waiting for the AI provider to score this attempt.",
  /** `progress.drill.attempts.loading`, the panel's own loading sentence. */
  attemptsLoading: "Loading the attempts",
  /** `progress.drill.attempts.error`, the panel's own read-failure sentence, with a retry. */
  attemptsError: "The attempts could not be loaded.",
  /** `state.retry`, the retry control of that state. */
  retry: "Try again",
  /** `common.close`, the control that closes the drill-down. */
  close: "Close",
  /**
   * `progress.drill.attempts.row`: one attempt's own line. The browser appends
   * the formatted time, so this is the prefix the row starts with.
   */
  attemptRowPrefix: (id: number, activity: string, status: string) => `Attempt ${id}: ${activity}, ${status},`,
  /** `progress.drill.attempts.open`, the row control's accessible name. */
  openAttempt: (id: number) => `Open the evidence for attempt ${id}`,
  /** `progress.evidence.title`, the evidence panel's heading. */
  evidenceTitle: (id: number) => `Evidence for attempt ${id}`,
  /** `progress.evidence.activity`, `progress.evidence.skill` and `progress.evidence.dimension`. */
  evidenceActivity: "Activity",
  evidenceSkill: "Skill",
  evidenceDimension: "Dimension",
  /** `progress.evidence.kind.response_text`: the group the stored answer is drawn in. */
  evidenceAnswer: "What was answered",
  /** `progress.evidence.kind.metric`: the group the scorer's versions are drawn in. */
  evidenceScored: "How it was scored",
  /** `progress.evidence.scorer` with `progress.scorer.deterministic`: an mcq is scored by fixed rules. */
  evidenceScorer: "Fixed rules in the program",
  /** `progress.evidence.algorithm`: the scoring method's version. */
  evidenceAlgorithm: "Scoring method version",
  /** `progress.units.title`, the panel the unit row lives in. */
  unitsTitle: "Units",
  /** `progress.units.nocheckpoint`, the unit row's honest state while no checkpoint was taken. */
  unitsNoCheckpoint: "No checkpoint taken yet",
  /** `progress.reviews.empty`, which stays on screen: the aborted run enrolled no review item. */
  reviewsEmpty: "Nothing is due for review right now.",
  /** `progress.errors.empty`. */
  errorsEmpty: "No mistakes are recorded yet.",
  /** `progress.units.empty`. */
  unitsEmpty: "No unit progress is recorded yet.",
  /** `progress.recent.empty`. */
  recentEmpty: "No practice sessions yet.",
  /** `progress.recent.title`, the panel the session row lives in. */
  recentTitle: "Recent practice",
  /** `progress.kind.lesson` and `progress.status.aborted`: what the stopped run is stored as. */
  sessionKind: "Lesson",
  sessionStatus: "stopped early",
  /** `unit.run.progress`, the line the run draws only once it has a step. */
  runProgress: /^This run: (\d+) of (\d+) activities answered\.$/,
  /** `activity.check`, the control that hands in an answer. */
  check: "Check",
  /** `activity.next`. */
  next: "Next",
  /** `activity.correct`, the server's verdict on an authored answer. */
  correct: "Correct!",
  /** `activity.pending`: the provider never scored the writing, so the screen says it waits. */
  queued: "Saved. Lumi will mark this when the AI provider is reachable.",
  /** `unit.roleplay.start` and `unit.roleplay.finish`: the roleplay is driven by its own panel. */
  roleplayStart: "Start the conversation",
  roleplayFinish: "Finish the conversation",
  /** The server's own sentence for a practice activity it stored without a score. */
  roleplayUnscored: /practice and has no score/,
};

/** The four skills of `apps/web/src/progress/model.ts`, in the order the profile draws them. */
const SKILLS = ["Listening", "Speaking", "Reading", "Writing"] as const;

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
 * screen's client calls for its own finish control. This spec stops a run whose
 * checkpoint is not finished, so the server closes it as aborted. Returns the
 * HTTP status.
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
 * Clears the learner's record through `DELETE /api/data`, the route the privacy
 * screen's own delete control calls. The suite shares one server and the
 * lesson-player spec runs before this one, so without this the page would not be
 * in the state a fresh data directory has. The guard refuses a request that
 * changes state without a JSON content type, so this carries one, exactly as
 * `stopSession` does. Returns the HTTP status.
 */
async function clearLearnerRecord(browser: Browser): Promise<number> {
  return (await browser.evaluate(async () => {
    const response = await fetch("/api/data", { method: "DELETE", headers: { "Content-Type": "application/json" } });
    return response.status;
  })) as number;
}

/** Every counted attempt one skill has, exactly as `GET /api/attempts?skill=...` answers it. */
interface AttemptListAnswer {
  skill: string;
  attempts: { id: number; activity_id: string; normalized: number | null; status: string }[];
  truncated: boolean;
}

async function listAttempts(browser: Browser, skill: string): Promise<AttemptListAnswer> {
  return (await browser.evaluate(async (name: string) => {
    return await (await fetch(`/api/attempts?skill=${encodeURIComponent(name)}`)).json();
  }, skill)) as AttemptListAnswer;
}

/** The learner's progress, exactly as `GET /api/progress` answers it. */
interface ProgressAnswer {
  units: { unit_id: string; status: string; best_checkpoint: number | null }[];
  reviews_due: unknown[];
  estimates: { skill: string; level: string | null; status: string; evidence_count: number }[];
  recent_sessions: { id: number; kind: string; status: string }[];
}

async function readProgress(browser: Browser): Promise<ProgressAnswer> {
  return (await browser.evaluate(async () => {
    return await (await fetch("/api/progress")).json();
  })) as ProgressAnswer;
}

/** One attempt's stored evidence, exactly as `GET /api/attempts/{id}/evidence` answers it. */
interface EvidenceAnswer {
  attempt_id: number;
  activity_id: string;
  skill: string;
  dimension: string;
  evidence: { kind: string; content: string | null; data: Record<string, unknown> | null }[];
}

async function readEvidence(browser: Browser, attemptId: number): Promise<EvidenceAnswer> {
  return (await browser.evaluate(async (id: number) => {
    return await (await fetch(`/api/attempts/${id}/evidence`)).json();
  }, attemptId)) as EvidenceAnswer;
}

/** Everything the page shows, as text: what the no-level rule is checked against. */
async function pageText(browser: Browser): Promise<string> {
  return (await browser.evaluate(() => document.body.textContent ?? "")) as string;
}

/**
 * The panel whose title is `title`. The progress panels carry no accessible
 * name, so the title heading is the one thing that scopes them, which is what a
 * reader uses too.
 */
function panel(browser: Browser, title: string): Locator {
  return browser.locator(`section.px-panel--raised:has(h2:text-is("${title}"))`);
}

/**
 * The drill-down's attempts list, as the screen drew it: one control per row,
 * each named with its attempt number, plus each row's own text.
 */
async function attemptRows(browser: Browser): Promise<{ ids: number[]; texts: string[] }> {
  const rows = browser.locator("#skill-drilldown .attempts__open");
  const texts = await rows.allTextContents();
  const ids = texts.map((text) => Number(/^Attempt (\d+):/.exec(text)?.[1] ?? NaN));
  return { ids, texts };
}

/**
 * Whatever this spec did, the next one must find the server's single session
 * slot free. The walk below stops its own session, and this is the belt to that
 * brace: an assertion that fails half way through the walk must not leave a
 * session running for the reading, settings or writing spec to trip over. A page
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

/**
 * Answers the activity on screen with the answer the unit file authored, using
 * the control that body is drawn with — the helper `tests/lesson-player.e2e.ts`
 * carries. The run only offers bodies this screen can answer, so anything else
 * is a body this spec does not know and it fails loudly rather than skipping the
 * activity. The roleplay is driven by its own panel and is not answered here.
 */
async function answerAuthored(screen: Screen, activity: AuthoredActivity): Promise<void> {
  switch (activity.type) {
    case "mcq": {
      const choice = activity.options?.[activity.answer_index ?? -1];
      if (choice === undefined) throw new Error(`${activity.id} has no authored choice`);
      await tapAt(screen, screen.getByRole("group", { name: activity.stem ?? "" }).getByRole("radio", { name: choice }));
      return;
    }
    case "gap_fill": {
      for (const [index, accepted] of (activity.answers ?? []).entries()) {
        const authored = accepted[0];
        if (authored === undefined) throw new Error(`${activity.id} gap ${index + 1} has no authored answer`);
        await screen.getByLabel(`Gap ${index + 1}`).fill(authored);
      }
      return;
    }
    case "reorder": {
      for (const word of (activity.answer ?? "").split(/\s+/)) {
        await tapAt(screen, screen.getByRole("button", { name: `Add: ${word}` }));
      }
      return;
    }
    case "match": {
      for (const pair of activity.pairs ?? []) {
        await screen.getByLabel(`Match for: ${pair.left}`).selectOption({ label: pair.right });
      }
      return;
    }
    case "reading_set": {
      for (const question of activity.questions ?? []) {
        const choice = question.options[question.answer_index];
        if (choice === undefined) throw new Error(`${activity.id} question has no authored choice`);
        await tapAt(screen, screen.getByRole("group", { name: question.stem }).getByRole("radio", { name: choice }));
      }
      return;
    }
    case "dictation":
    case "error_correction": {
      const authored = activity.accepted_answers?.[0];
      if (authored === undefined) throw new Error(`${activity.id} has no authored answer`);
      await screen.getByLabel("Your answer").fill(authored);
      return;
    }
    case "guided_writing": {
      const authored = activity.model_answers?.find((answer) => answer.band === "at")?.text;
      if (authored === undefined) throw new Error(`${activity.id} has no authored answer`);
      await screen.getByLabel("Your answer").fill(authored);
      return;
    }
    default:
      throw new Error(`${activity.id} (${activity.type}) has no control this spec knows how to use`);
  }
}

test("the progress screen shows the empty state, then real work and its evidence", { timeout: 240_000 }, async ({
  app,
  browser,
  screen,
}) => {
  // The learner reads in English, which is the language the unit file and these
  // strings are read in. Nothing else is changed: the settings are the state a
  // learner has.
  await browser.addInitScript(() => {
    try {
      const stored = JSON.parse(localStorage.getItem("lumingo.prefs.v1") ?? "{}") as Record<string, unknown>;
      localStorage.setItem("lumingo.prefs.v1", JSON.stringify({ ...stored, language: "en" }));
    } catch {
      // Storage can be blocked; the page works without it.
    }
  });
  await browser.setViewport(TALL_VIEWPORT);

  // 1. A fresh record, the state the offline data directory starts with. The
  //    suite shares one server and the lesson-player spec ran first, so a run of
  //    it is still stored; it is cleared through the route the privacy screen's
  //    own delete control calls, after a leftover session — which the server
  //    refuses that delete for — is stopped.
  await app.open("/unit/");
  const leftover = await activeSessionId(browser);
  if (leftover !== null) await stopSession(browser, leftover);
  expect(await clearLearnerRecord(browser)).toBe(200);

  // 2. The screen is reached through the menu.
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navProgress }));
  await expect(browser).toHaveURL("/progress/", { timeout: 15_000 });
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.title })).toBeVisible();

  // 3. The empty state. The banner is the page's own title and sentence for it,
  //    and the four panels with nothing to show each say so.
  await expect(screen.getByText(TEXT.emptyTitle)).toBeVisible({ timeout: 15_000 });
  await expect(screen.getByText(TEXT.emptyBody)).toBeVisible();
  await expect(screen.getByText(TEXT.reviewsEmpty)).toBeVisible();
  await expect(screen.getByText(TEXT.errorsEmpty)).toBeVisible();
  await expect(screen.getByText(TEXT.unitsEmpty)).toBeVisible();
  await expect(screen.getByText(TEXT.recentEmpty)).toBeVisible();

  // 4. Each of the four skills says it has not enough evidence yet, in the rule's
  //    own words, and no level appears anywhere on the page: with no estimate
  //    there is no level to show, and the page shows none.
  const profile = screen.getByRole("list", { name: TEXT.profile });
  await expect(profile).toBeVisible();
  const emptySentences = await browser.locator("ul.profile p.profile__sentence").allTextContents();
  expect(emptySentences).toHaveLength(SKILLS.length);
  SKILLS.forEach((skill, index) => {
    expect(emptySentences[index]).toBe(TEXT.insufficient(skill.toLowerCase()));
  });
  expect(await pageText(browser)).not.toMatch(LEVEL);

  // 5. Real state, made through the screen. The picker's link names the unit by
  //    its own sequence and title, and the run's own progress line renders only
  //    once the run has a step, so waiting for it is waiting for the run.
  await app.open("/unit/");
  const unitLink = screen.getByRole("link", { name: `Unit ${unit.sequence}: ${unit.title.en}` });
  await expect(unitLink).toBeVisible();
  await expect(unitLink).toHaveAttribute("href", `/unit/?id=${unit.id}`);
  await tapAt(screen, unitLink);
  await expect(screen.getByRole("heading", { level: 1, name: unit.title.en })).toBeVisible();
  await expect(screen.getByText(TEXT.runProgress)).toBeVisible({ timeout: 15_000 });

  // 6. The walk: the activities the server offers are answered with the unit
  //    file's authored answers, in the file's own order, until the first reading
  //    attempt has been answered. The activities the server reports it cannot run
  //    offline (the ones that need speech) are the ones this walk skips.
  const unavailable = await browser.locator("li.unavailable p.unavailable__title").allTextContents();
  const offered = unit.activities.filter((activity) => !unavailable.includes(flat(activity.instructions.en)));
  expect(offered).toContain(reading);
  expect(offered.length).toBeGreaterThan(0);

  for (const activity of offered) {
    if (activity.type === "roleplay") {
      // The roleplay is started and finished by hand; offline no provider
      // answers it, so the screen says so and offers the way on.
      await expect(screen.getByRole("button", { name: TEXT.roleplayStart })).toBeVisible({ timeout: 15_000 });
      await tapAt(screen, screen.getByRole("button", { name: TEXT.roleplayStart }));
      await expect(screen.getByRole("button", { name: TEXT.roleplayFinish })).toBeVisible({ timeout: 15_000 });
      await tapAt(screen, screen.getByRole("button", { name: TEXT.roleplayFinish }));
      await expect(screen.getByText(TEXT.roleplayUnscored)).toBeVisible({ timeout: 15_000 });
      await tapAt(screen, screen.getByRole("button", { name: TEXT.next }));
      continue;
    }

    // The activity is on screen: the frame carries the file's own instruction.
    await expect(screen.getByRole("article").getByText(flat(activity.instructions.en)).first()).toBeVisible();
    await answerAuthored(screen, activity);
    // The check control is enabled only once the answer is complete, so waiting
    // for that is waiting for the body to hold what the file authored.
    const check = screen.getByRole("button", { name: TEXT.check });
    await expect(check).toBeEnabled();
    await tapAt(screen, check);
    // The server's own verdict: the authored answers are right, except for the
    // writing, which waits for a provider that offline never comes.
    await expect(screen.getByText(activity.type === "guided_writing" ? TEXT.queued : TEXT.correct)).toBeVisible({
      timeout: 15_000,
    });
    if (activity.id === READING_ATTEMPT) break;
    await tapAt(screen, screen.getByRole("button", { name: TEXT.next }));
  }

  // 7. The reading attempt was answered, so the run has real state to show. The
  //    run is stopped through the page's own route, the technique the wave-1
  //    specs use to free the server's single session slot: the checkpoint is not
  //    finished, so the server closes the run as aborted and enrols no review
  //    item for it.
  const runProgress = await screen.getByText(TEXT.runProgress).textContent();
  const answered = Number(TEXT.runProgress.exec(runProgress ?? "")?.[1] ?? 0);
  expect(answered).toBeGreaterThan(0);
  const session = await activeSessionId(browser);
  expect(session).not.toBeNull();
  if (session !== null) expect(await stopSession(browser, session)).toBe(200);
  expect(await activeSessionId(browser)).toBeNull();

  // 8. The populated page. Everything asserted here is what the server really
  //    sent: the overview and the skill's attempts are read back through the
  //    page's own routes, and the numbers on screen are checked against them.
  await app.open("/progress/");
  const overview = await readProgress(browser);
  const readingAttempts = await listAttempts(browser, "reading");

  // The empty banner is gone and the learner's own unit is on the page, with the
  // honest "no checkpoint taken yet" of a run that was stopped.
  await expect(screen.getByText(TEXT.emptyTitle)).toHaveCount(0);
  const unitsPanel = panel(browser, TEXT.unitsTitle);
  await expect(unitsPanel.getByText(unit.title.en, { exact: true })).toBeVisible({ timeout: 15_000 });
  expect(overview.units.map((row) => row.unit_id)).toContain(unit.id);
  const unitRow = overview.units.find((row) => row.unit_id === unit.id);
  expect(unitRow?.best_checkpoint).toBeNull();
  await expect(unitsPanel.getByText(TEXT.unitsNoCheckpoint)).toBeVisible();

  // The run's own session row, with the number and the status word the server
  // sent, and as many rows as the server listed.
  const recentPanel = panel(browser, TEXT.recentTitle);
  expect(overview.recent_sessions.length).toBeGreaterThan(0);
  const newest = overview.recent_sessions[0];
  expect(newest).toBeDefined();
  if (newest !== undefined) {
    expect(newest.status).toBe("aborted");
    expect(newest.kind).toBe("lesson");
    const row = new RegExp(`^Session ${newest.id}: ${TEXT.sessionKind}, ${TEXT.sessionStatus}, `);
    expect(await recentPanel.getByText(row).count()).toBe(1);
    await expect(recentPanel.getByText(row)).toBeVisible();
  }
  await expect(browser.locator(`section.px-panel--raised:has(h2:text-is("${TEXT.recentTitle}")) ul.rows > li`)).toHaveCount(
    overview.recent_sessions.length,
  );

  // The review queue is still empty, and now for a reason the walk created: the
  // server enrolled no item for the aborted run. The four skills are still short
  // of the evidence rule, so no level appears on the page.
  expect(overview.reviews_due).toHaveLength(0);
  await expect(screen.getByText(TEXT.reviewsEmpty)).toBeVisible();
  const populatedSentences = await browser.locator("ul.profile p.profile__sentence").allTextContents();
  expect(populatedSentences).toHaveLength(SKILLS.length);
  SKILLS.forEach((skill, index) => {
    expect(populatedSentences[index]).toBe(TEXT.insufficient(skill.toLowerCase()));
  });
  expect(await pageText(browser)).not.toMatch(LEVEL);

  // 9. The drill-down, from Reading's estimate to the attempts behind it. The
  //    attempt just answered is the one the server counts for reading, and the
  //    list is the server's own, with the sentence that says what it is.
  await tapAt(screen, screen.getByRole("button", { name: TEXT.evidence("Reading") }));
  const drill = screen.getByRole("region", { name: TEXT.drillTitle("Reading") });
  await expect(drill.getByRole("heading", { name: TEXT.drillTitle("Reading") })).toBeVisible({ timeout: 15_000 });
  await expect(drill.getByText(TEXT.attemptsIntro)).toBeVisible();
  expect(readingAttempts.truncated).toBe(false);
  expect(readingAttempts.attempts.length).toBeGreaterThan(0);
  const counted = readingAttempts.attempts.find((attempt) => attempt.activity_id === READING_ATTEMPT);
  expect(counted).toBeDefined();
  if (counted === undefined) throw new Error(`the server counted no reading attempt for ${READING_ATTEMPT}`);
  // The authored answer was right, and the server stored the attempt as scored.
  expect(counted.normalized).toBe(1);
  expect(counted.status).toBe("scored");

  // The rows on screen are the server's own list: the same count, the same
  // numbers, and the reading attempt's own line and control.
  const readingRows = await attemptRows(browser);
  expect(readingRows.ids).toHaveLength(readingAttempts.attempts.length);
  expect(new Set(readingRows.ids)).toEqual(new Set(readingAttempts.attempts.map((attempt) => attempt.id)));
  expect(
    readingRows.texts.some((text) => text.startsWith(TEXT.attemptRowPrefix(counted.id, READING_ATTEMPT, "scored"))),
  ).toBe(true);
  await expect(drill.getByRole("button", { name: TEXT.openAttempt(counted.id) })).toBeVisible();
  // One reading task is far from the rule's evidence, so the panel's own status
  // fact says the rule is not met, and it still shows no level.
  await expect(drill.getByText(TEXT.drillStatusInsufficient)).toBeVisible();
  expect(await drill.textContent()).not.toMatch(LEVEL);

  // 10. The row opens the stored evidence of that attempt: the answer the server
  //     kept — the mcq's chosen option, the authored `answer_index` value — and
  //     the facts of the scorer that produced the score.
  await tapAt(screen, drill.getByRole("button", { name: TEXT.openAttempt(counted.id) }));
  const evidencePanel = browser.locator("#skill-drilldown .px-panel--inset");
  await expect(evidencePanel.getByRole("heading", { name: TEXT.evidenceTitle(counted.id) })).toBeVisible({
    timeout: 15_000,
  });
  const stored = await readEvidence(browser, counted.id);
  const answer = stored.evidence.find((entry) => entry.kind === "response_text");
  expect(answer?.content).toBe(READING_ANSWER);
  const metric = stored.evidence.find((entry) => entry.kind === "metric");
  expect(metric?.data?.scorer).toBe("deterministic");
  const algorithm = metric?.data?.algorithm_version;
  expect(typeof algorithm).toBe("string");

  // The panel shows what the route sent: the attempt's own activity, skill and
  // dimension, the stored answer as text, and the scorer's versions.
  await expect(evidencePanel.getByText(TEXT.evidenceActivity)).toBeVisible();
  await expect(evidencePanel.getByText(stored.activity_id, { exact: true })).toBeVisible();
  await expect(evidencePanel.getByText(TEXT.evidenceSkill)).toBeVisible();
  await expect(evidencePanel.getByText(stored.skill, { exact: true })).toBeVisible();
  await expect(evidencePanel.getByText(TEXT.evidenceDimension)).toBeVisible();
  await expect(evidencePanel.getByText(stored.dimension, { exact: true })).toBeVisible();
  await expect(evidencePanel.getByRole("heading", { name: TEXT.evidenceAnswer })).toBeVisible();
  await expect(evidencePanel.getByText(READING_ANSWER, { exact: true })).toBeVisible();
  await expect(evidencePanel.getByRole("heading", { name: TEXT.evidenceScored })).toBeVisible();
  await expect(evidencePanel.getByText(TEXT.evidenceScorer)).toBeVisible();
  await expect(evidencePanel.getByText(TEXT.evidenceAlgorithm)).toBeVisible();
  await expect(evidencePanel.getByText(String(algorithm))).toBeVisible();
  // One attempt's record says nothing about a level, and the panel shows none.
  expect(await evidencePanel.textContent()).not.toMatch(LEVEL);

  // 11. The list the server cuts. More attempts than it holds are made through
  //     the page's own routes — a run per writing response, each stored waiting
  //     for the provider that offline never comes — until the server says its
  //     list is truncated. Nothing here invents a score: every row is pending.
  await tapAt(screen, drill.getByRole("button", { name: TEXT.close }));
  await expect(screen.getByRole("region", { name: TEXT.drillTitle("Reading") })).toHaveCount(0);
  let writingAttempts = await listAttempts(browser, "writing");
  for (let round = 0; round < 20 && !writingAttempts.truncated; round += 1) {
    const failure = (await browser.evaluate(
      async ({ unitId, activityId, text }: { unitId: string; activityId: string; text: string }) => {
        const post = (path: string, body: unknown) =>
          fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
        const started = await post("/api/sessions", {
          kind: "lesson",
          unit_id: unitId,
          activity_id: null,
          mode: null,
          topic: null,
          level: null,
          speak: null,
        });
        if (!started.ok) return `start: ${started.status}`;
        const session = (await started.json()) as { id: number };
        const submitted = await post("/api/activities/submit", {
          session_id: session.id,
          activity_id: activityId,
          answer: { kind: "text", text },
        });
        if (!submitted.ok) return `submit: ${submitted.status}`;
        const stopped = await post(`/api/sessions/${session.id}/stop`, {});
        if (!stopped.ok) return `stop: ${stopped.status}`;
        return "";
      },
      { unitId: unit.id, activityId: WRITING_ATTEMPT, text: WRITING_TEXT },
    )) as string;
    expect(failure, `the run that fills the attempts list failed: ${failure}`).toBe("");
    writingAttempts = await listAttempts(browser, "writing");
    // The server's list never grows past its own limit.
    expect(writingAttempts.attempts.length).toBeLessThanOrEqual(50);
  }
  expect(writingAttempts.truncated, "the server never reported the attempts list as truncated").toBe(true);

  // 12. The truncated list, on screen: the sentence says the list was cut, the
  //     rows are the 50 the server sent, and the waiting rows are shown as
  //     waiting, never as a score. The skill is still short of the evidence rule,
  //     so no level appears either.
  await app.open("/progress/");
  await tapAt(screen, screen.getByRole("button", { name: TEXT.evidence("Writing") }));
  const writingDrill = screen.getByRole("region", { name: TEXT.drillTitle("Writing") });
  await expect(writingDrill.getByRole("heading", { name: TEXT.drillTitle("Writing") })).toBeVisible({ timeout: 15_000 });
  await expect(writingDrill.getByText(TEXT.attemptsIntro)).toBeVisible();
  await expect(writingDrill.getByText(TEXT.attemptsTruncated)).toBeVisible({ timeout: 15_000 });
  expect(writingAttempts.attempts.length).toBe(50);
  expect(writingAttempts.attempts.every((attempt) => attempt.status === "pending_llm")).toBe(true);
  expect(writingAttempts.attempts.every((attempt) => attempt.normalized === null)).toBe(true);
  const writingRows = await attemptRows(browser);
  expect(writingRows.ids).toHaveLength(writingAttempts.attempts.length);
  expect(new Set(writingRows.ids)).toEqual(new Set(writingAttempts.attempts.map((attempt) => attempt.id)));
  const pending = writingAttempts.attempts[0];
  expect(pending).toBeDefined();
  if (pending !== undefined) {
    expect(
      writingRows.texts.some((text) =>
        text.startsWith(TEXT.attemptRowPrefix(pending.id, WRITING_ATTEMPT, "pending_llm")),
      ),
    ).toBe(true);
    await expect(writingDrill.getByRole("button", { name: TEXT.openAttempt(pending.id) })).toBeVisible();
  }
  // A row waiting for a provider says so, and the list is not the empty state.
  await expect(writingDrill.getByText(TEXT.attemptsPending).first()).toBeVisible();
  await expect(writingDrill.getByText(TEXT.attemptsEmpty)).toHaveCount(0);
  expect(await writingDrill.textContent()).not.toMatch(LEVEL);
  await tapAt(screen, writingDrill.getByRole("button", { name: TEXT.close }));

  // 13. A skill with nothing counted shows the honest empty sentence instead of
  //     an error or an invented row. Offline no listening item can run (they all
  //     need speech output), so listening has no counted attempt at all.
  const listeningAttempts = await listAttempts(browser, "listening");
  expect(listeningAttempts.attempts).toHaveLength(0);
  await tapAt(screen, screen.getByRole("button", { name: TEXT.evidence("Listening") }));
  const listening = screen.getByRole("region", { name: TEXT.drillTitle("Listening") });
  await expect(listening.getByRole("heading", { name: TEXT.drillTitle("Listening") })).toBeVisible({ timeout: 15_000 });
  await expect(listening.getByText(TEXT.attemptsEmpty)).toBeVisible({ timeout: 15_000 });
  expect(await browser.locator("#skill-drilldown .attempts__open").count()).toBe(0);
  expect(await listening.textContent()).not.toMatch(LEVEL);
  await tapAt(screen, listening.getByRole("button", { name: TEXT.close }));

  // 14. The drill-down's two remaining honest states, driven through the page's
  //     own request. While the read is in flight the panel says it is loading;
  //     a read that fails says so and offers the retry that fixes it. Both
  //     routes are this attempt's, so the server itself is untouched.
  await browser.route(/\/api\/attempts\?skill=reading$/, async (route) => {
    // Held until the assertion below has seen the loading state, then let through.
    await new Promise((release) => setTimeout(release, 3_000));
    await route.continue();
  });
  await tapAt(screen, screen.getByRole("button", { name: TEXT.evidence("Reading") }));
  const loading = screen.getByRole("region", { name: TEXT.drillTitle("Reading") });
  await expect(loading.getByText(TEXT.attemptsLoading)).toBeVisible({ timeout: 15_000 });
  await expect(loading.getByText(TEXT.attemptsIntro)).toBeVisible();
  await expect(loading.getByRole("button", { name: TEXT.openAttempt(counted.id) })).toBeVisible({ timeout: 15_000 });
  await browser.unroute(/\/api\/attempts\?skill=reading$/);
  await tapAt(screen, loading.getByRole("button", { name: TEXT.close }));

  await browser.route(/\/api\/attempts\?skill=reading$/, (route) =>
    route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "internal" }) }),
  );
  await tapAt(screen, screen.getByRole("button", { name: TEXT.evidence("Reading") }));
  const failed = screen.getByRole("region", { name: TEXT.drillTitle("Reading") });
  await expect(failed.getByText(TEXT.attemptsError)).toBeVisible({ timeout: 15_000 });
  expect(await browser.locator("#skill-drilldown .attempts__open").count()).toBe(0);
  await browser.unroute(/\/api\/attempts\?skill=reading$/);
  await tapAt(screen, failed.getByRole("button", { name: TEXT.retry }));
  await expect(failed.getByRole("button", { name: TEXT.openAttempt(counted.id) })).toBeVisible({ timeout: 15_000 });
  await expect(failed.getByText(TEXT.attemptsError)).toHaveCount(0);

  // 15. Nothing is left running for the next spec: the server's one session slot
  //     is free.
  expect(await activeSessionId(browser)).toBeNull();
});
