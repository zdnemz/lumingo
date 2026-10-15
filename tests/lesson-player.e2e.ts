import { test } from "@e2e-dev/web";
import { expect, type Locator, type Screen } from "e2e";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

/**
 * The lesson player, end to end, against the offline server `pnpm test:e2e`
 * starts by itself.
 *
 * The spec walks the example unit the way a learner does: it opens the picker,
 * follows the link into the player, and answers every activity the run offers
 * with the answer the unit file authored, until the server has nothing left to
 * offer and the screen settles the checkpoint. Every fact about the unit — its
 * activities, their bodies, the correct answers, the Indonesian instructions,
 * the checkpoint — is read from the unit file the server indexed, so the spec
 * cannot disagree with the curriculum.
 *
 * What the run cannot do is the server's to say, not this spec's: the server
 * reports those activities with its own sentences, and the spec checks the
 * properties of that list — every entry names an activity of the unit, every
 * entry gives a reason, and the counts add up — rather than which activities
 * they are. Offline there is no provider, so the writing is stored and queued,
 * the roleplay reaches nobody, and the honest end state is a checkpoint that is
 * not passed.
 *
 * Interactions use `screen.tapAt` at a point inside the node rather than
 * `locator.tap`. Both are the same learner gesture, but a locator tap first
 * waits for the node to hold still across animation frames, and this headless
 * browser stops producing frames once the page has settled and nothing is
 * painted — so that wait never ends. A point tap is dispatched straight at the
 * point the node reports, which is what a person's finger does. The page is
 * laid out in a viewport tall enough to hold the whole run, so a point is always
 * on screen and nothing has to be scrolled.
 */

/** The unit file `scripts/e2e-server.sh` indexes, relative to the project root. */
const UNIT_FILE = "curriculum/examples/a1-u01.example.json";

/** A viewport tall enough that the whole screen fits: no scrolling, so a point tap always lands. */
const TALL_VIEWPORT = { width: 1280, height: 6000 };

/** One activity of the unit file, as far as this spec reads it. */
interface AuthoredActivity {
  id: string;
  type: string;
  skill: string;
  instructions: { en: string; id?: string };
  /** `mcq`. */
  stem?: string;
  options?: string[];
  answer_index?: number;
  /** `gap_fill`: one accepted group per gap, the authored answer first. */
  text?: string;
  answers?: string[][];
  /** `reorder`. */
  tokens?: string[];
  answer?: string;
  /** `match`. */
  pairs?: { left: string; right: string }[];
  /** `reading_set` and `listening_set`. */
  passage?: string;
  questions?: { stem: string; options: string[]; answer_index: number }[];
  /** `dictation` and `error_correction`. */
  accepted_answers?: string[];
  sentence?: string;
  /** `guided_writing` and `guided_speaking`. */
  prompt?: { en: string; id?: string };
  model_answers?: { band: string; text: string }[];
  /** `roleplay`. */
  scenario?: { en: string; id?: string };
}

interface AuthoredUnit {
  id: string;
  sequence: number;
  title: { en: string; id?: string };
  activities: AuthoredActivity[];
  checkpoint: { pass_score: number; activity_ids: string[] };
}

const unit = JSON.parse(readFileSync(resolve(process.cwd(), UNIT_FILE), "utf8")) as AuthoredUnit;

function activityById(id: string): AuthoredActivity {
  const found = unit.activities.find((activity) => activity.id === id);
  if (found === undefined) throw new Error(`${id} is not an activity of ${unit.id}`);
  return found;
}

/** The unit file writes text in blocks; a browser collapses that to single spaces. */
const flat = (text: string): string => text.replace(/\s+/g, " ").trim();

/** A 0 to 1 score as the checkpoint panel writes it. */
const percent = (value: number): number => Math.round(value * 100);

/** The screen's own strings, from `apps/web/src/i18n/en.ts`. */
const TEXT = {
  check: "Check",
  next: "Next",
  correct: "Correct!",
  /** `activity.pending`: the server queued the answer for a provider to mark. */
  queued: "Saved. Lumi will mark this when the AI provider is reachable.",
  /** `banner.noprovider.title`: the screen's own line for a provider that is not set up. */
  noProvider: "No AI provider is set up",
  helpToggle: /Help in Indonesian/,
  roleplayStart: "Start the conversation",
  roleplayFinish: "Finish the conversation",
  /** The server's own sentence for a practice activity it stored without a score. */
  roleplayUnscored: /practice and has no score/,
  checkpoint: "The checkpoint",
  notPassed: "The checkpoint is not passed yet.",
  /** `unit.checkpoint.unanswered`, the cell of a row with no score. */
  unanswered: "Not answered",
  /** The four skills of `apps/web/src/unit/model.ts`, in the order the screen draws them. */
  skills: ["Listening", "Speaking", "Reading", "Writing"] as const,
  /** `unit.run.progress`. */
  runProgress: /^This run: (\d+) of (\d+) activities answered\.$/,
  /** `unit.skills.progress`, the per-skill count of this run. */
  skillProgress: /^(\d+) of (\d+) activities answered in this run$/,
  /** `unit.checkpoint.mean`. */
  mean: /^Mean: (\d+) percent$/,
  /** `unit.checkpoint.passmark`. */
  passMark: /^Pass mark: (\d+) percent$/,
  /** `unit.checkpoint.answered`. */
  checkpointAnswered: /^(\d+) of (\d+) checkpoint activities answered\.$/,
  /** `unit.checkpoint.percent`, the cell of a row the server scored. */
  scored: /^(\d+) percent$/,
};

/**
 * The four-skill rule of `apps/web/src/unit/model.ts`: both speaking skills and
 * pronunciation are speaking, and grammar, vocabulary and mediation are none of
 * the four, so they get no section.
 */
function fourSkillOf(skill: string): string | null {
  switch (skill) {
    case "listening":
      return "Listening";
    case "reading":
      return "Reading";
    case "writing":
      return "Writing";
    case "speaking_production":
    case "speaking_interaction":
    case "pronunciation":
      return "Speaking";
    default:
      return null;
  }
}

/** How many activities of each of the four skills the unit file authors. */
function authoredSkillTotals(): Map<string, number> {
  const totals = new Map<string, number>(TEXT.skills.map((skill) => [skill, 0]));
  for (const activity of unit.activities) {
    const skill = fourSkillOf(activity.skill);
    if (skill !== null) totals.set(skill, (totals.get(skill) ?? 0) + 1);
  }
  return totals;
}

/** The middle of a node: the point a learner's finger lands on. */
async function tapAt(screen: Screen, locator: Locator): Promise<void> {
  const box = await locator.boundingBox();
  if (box === null) throw new Error("the node has no box to tap");
  await screen.tapAt({ x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

/**
 * Answers the activity on screen with the answer the unit file authored, using
 * the control that body is drawn with. The run only offers bodies this screen
 * can answer, so anything else is a body this spec does not know and it fails
 * loudly rather than skipping the activity.
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
      const gaps = activity.answers ?? [];
      for (const [index, accepted] of gaps.entries()) {
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
    case "reading_set":
    case "listening_set": {
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

test("the example unit is played to its checkpoint in a browser", { timeout: 240_000 }, async ({ app, browser, screen }) => {
  // The learner reads in English, which is the language the unit file and these
  // strings are read in. Nothing else is changed: the settings are the state a
  // learner has, and the run proves that state works.
  await browser.addInitScript(() => {
    try {
      const stored = JSON.parse(localStorage.getItem("lumingo.prefs.v1") ?? "{}") as Record<string, unknown>;
      localStorage.setItem("lumingo.prefs.v1", JSON.stringify({ ...stored, language: "en" }));
    } catch {
      // Storage can be blocked; the page works without it.
    }
  });
  await browser.setViewport(TALL_VIEWPORT);

  // 1. The picker lists the unit. The link's accessible name is the map's own: "Unit N: title".
  await app.open("/unit/");
  const unitLink = screen.getByRole("link", { name: `Unit ${unit.sequence}: ${unit.title.en}` });
  await expect(unitLink).toBeVisible();
  await expect(unitLink).toHaveAttribute("href", `/unit/?id=${unit.id}`);

  // 2. The player. It mounts no first-run gate, so a learner who opened a unit
  //    gets the lesson even with no provider set up.
  await app.open(`/unit/?id=${unit.id}`);
  await expect(screen.getByRole("heading", { level: 1, name: unit.title.en })).toBeVisible();

  // 3. What the run will not do is the server's own list, with the server's own
  //    sentences: every entry names an activity of this unit and gives a reason.
  const unavailableInstructions = await browser.locator("li.unavailable p.unavailable__title").allTextContents();
  const unavailableReasons = await browser.locator("li.unavailable p.px-hint").allTextContents();
  expect(unavailableInstructions.length).toBeGreaterThan(0);
  expect(unavailableInstructions).toHaveLength(unavailableReasons.length);
  for (const reason of unavailableReasons) expect(reason.trim().length).toBeGreaterThan(0);
  const authoredInstructions = new Set(unit.activities.map((activity) => flat(activity.instructions.en)));
  for (const instruction of unavailableInstructions) expect(authoredInstructions.has(instruction)).toBe(true);
  expect(new Set(unavailableInstructions).size).toBe(unavailableInstructions.length);

  // The unavailable entries carry no control: the screen says so instead of
  // offering a button that would fail.
  const unavailableList = browser.locator("li.unavailable");
  expect(await unavailableList.count()).toBe(unavailableInstructions.length);
  expect(await unavailableList.getByRole("button").count()).toBe(0);
  expect(await unavailableList.getByRole("textbox").count()).toBe(0);

  // 4. The four skill sections are the unit's own, with the unit's own totals.
  const skillTotals = authoredSkillTotals();
  const skillLines = await screen.getByText(TEXT.skillProgress).allTextContents();
  expect(skillLines).toHaveLength(TEXT.skills.length);
  TEXT.skills.forEach((skill, index) => {
    const line = TEXT.skillProgress.exec(skillLines[index] ?? "");
    expect(line?.[2], `${skill} section total`).toBe(String(skillTotals.get(skill)));
    expect(line?.[1], `${skill} answered before the walk`).toBe("0");
  });

  // 5. The help toggle shows the unit's own Indonesian text, and hides it again.
  const help = screen.getByRole("switch", TEXT.helpToggle);
  await expect(help).toBeChecked();
  const indonesian = flat(activityById(unit.activities[0]?.id ?? "").instructions.id ?? "");
  expect(indonesian.length).toBeGreaterThan(0);
  await expect(screen.getByText(indonesian).first()).toBeVisible();
  await tapAt(screen, help);
  await expect(screen.getByText(indonesian)).toHaveCount(0);
  await tapAt(screen, help);
  await expect(screen.getByText(indonesian).first()).toBeVisible();

  // 6. The walk. The run offers the unit's activities in the file's own order,
  //    minus the ones the server just reported it cannot run. Each is answered
  //    with the authored answer and followed on, until the server offers none.
  const offered = unit.activities.filter(
    (activity) => !unavailableInstructions.includes(flat(activity.instructions.en)),
  );
  expect(offered.length + unavailableInstructions.length).toBe(unit.activities.length);
  expect(offered.length).toBeGreaterThan(0);

  for (const activity of offered) {
    if (activity.type === "roleplay") {
      // A roleplay is started and finished by hand, and offline no provider
      // answers it: the screen says the provider is missing and offers the way on.
      const scenario = activity.scenario;
      if (scenario === undefined) throw new Error(`${activity.id} has no scenario`);
      await expect(screen.getByRole("heading", { name: flat(scenario.en) })).toBeVisible();
      await expect(screen.getByText(TEXT.noProvider)).toBeVisible();
      await tapAt(screen, screen.getByRole("button", TEXT.roleplayStart));
      await expect(screen.getByRole("button", TEXT.roleplayFinish)).toBeVisible();
      await tapAt(screen, screen.getByRole("button", TEXT.roleplayFinish));
      // The conversation reached no provider, so the server stored it without a
      // score and the walk shows the server's reason instead of a verdict.
      await expect(screen.getByText(TEXT.roleplayUnscored)).toBeVisible();
      await tapAt(screen, screen.getByRole("button", TEXT.next));
      continue;
    }

    // The activity is on screen: the frame carries the file's own instruction.
    await expect(screen.getByRole("article").getByText(flat(activity.instructions.en)).first()).toBeVisible();
    await answerAuthored(screen, activity);
    await tapAt(screen, screen.getByRole("button", TEXT.check));
    if (activity.type === "guided_writing") {
      // No provider, so the server stored the writing and queued it for one.
      await expect(screen.getByText(TEXT.queued)).toBeVisible();
    } else {
      await expect(screen.getByText(TEXT.correct)).toBeVisible();
    }
    await tapAt(screen, screen.getByRole("button", TEXT.next));
  }

  // 7. The run's own count: what it answered plus what the server would not run
  //    is the unit's whole activity list.
  const progress = TEXT.runProgress.exec((await screen.getByText(TEXT.runProgress).textContent()) ?? "");
  expect(progress).not.toBeNull();
  expect(Number(progress?.[1])).toBe(offered.length);
  expect(Number(progress?.[2])).toBe(unit.activities.length);
  expect(offered.length + unavailableInstructions.length).toBe(Number(progress?.[2]));

  // 8. Nothing left to offer: the screen stopped the run itself and settled the
  //    checkpoint, whose rows are the unit file's checkpoint activities.
  await expect(screen.getByRole("heading", { name: TEXT.checkpoint })).toBeVisible();
  const rows = await screen.getByRole("table").getByRole("rowheader").allTextContents();
  const cells = await screen.getByRole("table").getByRole("cell").allTextContents();
  expect(rows).toHaveLength(unit.checkpoint.activity_ids.length);
  const checkpointInstructions = new Set(
    unit.checkpoint.activity_ids.map((id) => flat(activityById(id).instructions.en)),
  );
  for (const row of rows) expect(checkpointInstructions.has(row)).toBe(true);

  // The mean the screen shows is the mean of the rows it scored, and the pass
  // mark is the unit's own.
  const scoredRows = cells.filter((cell) => TEXT.scored.test(cell));
  expect(scoredRows.length).toBeGreaterThan(0);
  const rowMean = Math.round(
    scoredRows.reduce((total, cell) => total + Number(TEXT.scored.exec(cell)?.[1]), 0) / scoredRows.length,
  );
  const mean = TEXT.mean.exec((await screen.getByText(TEXT.mean).textContent()) ?? "");
  expect(mean).not.toBeNull();
  expect(Number(mean?.[1])).toBe(rowMean);
  await expect(screen.getByText(`Pass mark: ${percent(unit.checkpoint.pass_score)} percent`)).toBeVisible();

  // How many checkpoint items the run answered, out of the unit's own list.
  const answered = TEXT.checkpointAnswered.exec((await screen.getByText(TEXT.checkpointAnswered).textContent()) ?? "");
  expect(answered).not.toBeNull();
  expect(Number(answered?.[2])).toBe(unit.checkpoint.activity_ids.length);
  expect(Number(answered?.[1])).toBeGreaterThan(0);
  expect(Number(answered?.[1])).toBeLessThanOrEqual(unit.checkpoint.activity_ids.length);

  // The checkpoint items the server would not run are the rows the run could not
  // answer, so the checkpoint is honestly not passed.
  const unansweredRows = rows.filter((row) => unavailableInstructions.includes(row));
  expect(unansweredRows.length).toBeGreaterThan(0);
  for (const row of unansweredRows) {
    expect(cells[rows.indexOf(row)], `${row} row`).toBe(TEXT.unanswered);
  }
  await expect(screen.getByText(TEXT.notPassed)).toBeVisible();
});
