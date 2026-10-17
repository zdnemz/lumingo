import { afterEach, test, type Browser } from "@e2e-dev/web";
import { expect, type Locator, type Screen } from "e2e";

/**
 * The shell's keyboard path, its type scale, and the engines panel, end to end,
 * against the offline server `pnpm test:e2e` starts by itself.
 *
 * **What "150 percent text" means here, plainly.** This harness has no `zoom`
 * call and no `deviceScaleFactor` setting, so nothing in this spec is a
 * browser-zoom simulation. What it does instead is the measurable part of the
 * same claim: the root font size is raised to 24px, the computed body size is
 * read before and after and must really move (20px to 30px, which only happens
 * on a rem-based type scale — on a px tree the number would not budge), the
 * document must not gain a horizontal scrollbar at 1280, must not gain one at a
 * 400px viewport either, and no `button` or `a.px-btn` may be clipped by the
 * viewport edge at that narrow width. That is "root font-size raised to 24px +
 * the computed size really moving + no horizontal overflow + no clipped
 * controls at a narrow viewport", not a browser zoom.
 *
 * **What the browser cannot reach offline.** With no audio backend and no
 * speech engines in this build, every engine the server reports is
 * `unavailable`, so the `loading` and `failed` rows and the engine fault banner
 * are NOT reachable in the browser: nothing here can make an engine load or
 * fail. Those states are proven with fixtures in vitest
 * (`apps/web/src/components/EngineStatusPanel.test.tsx` and
 * `EngineFaultBanner.test.tsx`), which render the same pure components from a
 * hand-written engine list. This spec asserts only the honest offline state and
 * says so rather than pretending to cover the rest.
 *
 * The first test reaches the diagnostics page through the menu, which is what
 * proves the nav entries end to end, and then reloads it fresh to walk the
 * keyboard from the top: the skip link is the first Tab stop, Enter lands focus
 * on `#main`, and the next Tab draws a ring too. The second test reads the
 * engines panel: six engines, each named and each saying it is not available,
 * the program's own sentence under every name, the retry control, and no
 * download or install control that offline could not honour.
 *
 * Interactions use `screen.tapAt` at a point inside the node rather than
 * `locator.tap`. Both are the same learner gesture, but a locator tap first
 * waits for the node to hold still across animation frames, and this headless
 * browser stops producing frames once the page has settled and nothing is
 * painted — so that wait never ends. A point tap is dispatched straight at the
 * point the node reports, which is what a person's finger does. The page is
 * laid out in a viewport tall enough to hold the whole screen, so a point is
 * always on screen and nothing has to be scrolled.
 */

/** A viewport tall enough that the whole screen fits: no scrolling, so a point tap always lands. */
const TALL_VIEWPORT = { width: 1280, height: 6000 };

/** A narrow phone-sized viewport: the width the no-overflow and no-clipping checks use. */
const NARROW_VIEWPORT = { width: 400, height: 900 };

/** The root size the text-scale step raises the document to, in pixels. */
const RAISED_ROOT_PX = 24;

/** The shell's, the settings sub-navigation's and the engines panel's strings, as the dictionaries spell them. */
const TEXT = {
  /** `nav.skip`, the skip link the first Tab lands on. */
  navSkip: "Skip to content",
  /** `nav.settings`, the menu entry this spec walks through. */
  navSettings: "Settings",
  /** `settings.nav`, the accessible name of the settings sub-navigation. */
  navLabel: "Settings sections",
  /** `settings.nav.diagnostics`, the sub-navigation entry to the diagnostics page. */
  navDiagnostics: "Diagnostics",
  /** `diag.title`, the diagnostics page's own `h1`. */
  diagTitle: "Diagnostics",
  /** `engine.panel.title`, rendered by `Panel` as an `h2`. */
  panelTitle: "Speech and audio engines",
  /** `engine.panel.intro`, the sentence under the panel title. */
  panelIntro: "Where each engine stands in this run. The sentence under a name is the program's own.",
  /** `engine.state.unavailable`: the one state every engine has offline. */
  unavailable: "Not available",
  /** `state.retry`, the engines panel's recovery control. */
  retry: "Try again",
  /** `engine.id.*`: every engine the program reports, in the order the panel lists them. */
  engines: ["Microphone", "Speaker", "Voice activity detection", "Speech to text", "Text to speech", "Pronunciation scoring"],
} as const;

/**
 * The middle of a node: the point a learner's finger lands on.
 *
 * The box is read again a moment later, and only a box that did not move is
 * tapped. The shell draws in stages — the header and the heading first, then the
 * banners once the server snapshot arrives, and a banner pushes everything below
 * it down by its own height. A point measured during one of those stages is a
 * point somewhere else by the time the finger lands, which is how a tap aimed at
 * the settings sub-navigation can miss it entirely.
 *
 * `locator.tap` cannot be used for this at all: it waits for the node to hold
 * still across animation frames, and this headless browser stops producing
 * frames once the page settles, so that wait never ends.
 */
async function tapAt(screen: Screen, locator: Locator): Promise<void> {
  let box = await locator.boundingBox();
  if (box === null) throw new Error("the node has no box to tap");
  for (let attempt = 0; attempt < 20; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 100));
    const again = await locator.boundingBox();
    if (again === null) throw new Error("the node has no box to tap");
    const steady =
      again.x === box.x && again.y === box.y && again.width === box.width && again.height === box.height;
    box = again;
    if (steady) break;
  }
  await screen.tapAt({ x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

/**
 * Waits until the shell has finished drawing, which is when the banners are in.
 *
 * Every page mounts its own `AppShell`, so a navigation starts a new shell with
 * an empty `.shell__banners`; the banner the offline snapshot brings then
 * arrives and pushes the page below it down. Waiting for that region to be drawn
 * is waiting for the last thing above the content to stop moving. Offline the
 * no-provider banner is always there, so this is the shell's own signal.
 */
async function shellSettled(browser: Browser): Promise<void> {
  await browser.locator(".shell__banners:not(:empty)").waitFor({ state: "visible", timeout: 15_000 });
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

/** Ends a session through `POST /api/sessions/{id}/stop`, the route every screen's client calls for its finish control. */
async function stopSession(browser: Browser, id: number): Promise<number> {
  return (await browser.evaluate(async (sessionId: number) => {
    const response = await fetch(`/api/sessions/${sessionId}/stop`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });
    return response.status;
  }, id)) as number;
}

/**
 * Whatever this spec did, the next one must find the server's single session
 * slot free. The walk below starts no session at all, so this belt should never
 * have anything to stop — it is here because one leaked session fails the other
 * seven specs with `Conflict`, and six lines is cheaper than that. A page that
 * never reached the app has no origin to ask, and then there is nothing this
 * spec could have started either.
 */
afterEach(async ({ browser }) => {
  try {
    const running = await activeSessionId(browser);
    if (running !== null) await stopSession(browser, running);
  } catch {
    // The page never reached the app, so it holds no session of this spec's.
  }
});

/** What the page says about whatever currently holds focus. */
type FocusReport = {
  tag: string;
  href: string | null;
  text: string;
  outlineStyle: string;
  outlineWidth: number;
};

/**
 * Reads the focused node and the ring the browser paints on it. `getComputedStyle`
 * is the only place the real ring can be read: jsdom never loads the stylesheet,
 * so the vitest walk can only check that `base.css` declares one, while this
 * reads what the engine actually resolved for the focused element.
 */
async function focusReport(browser: Browser): Promise<FocusReport> {
  return (await browser.evaluate(() => {
    const active = document.activeElement;
    if (active === null) return { tag: "", href: null, text: "", outlineStyle: "none", outlineWidth: 0 };
    const style = getComputedStyle(active);
    return {
      tag: active.tagName.toLowerCase(),
      href: active.getAttribute("href"),
      text: (active.textContent ?? "").trim(),
      outlineStyle: style.outlineStyle,
      outlineWidth: Number.parseFloat(style.outlineWidth),
    };
  })) as FocusReport;
}

/** How much wider the document is than its viewport: a positive number is a sideways scrollbar. */
interface WidthReport {
  scrollWidth: number;
  clientWidth: number;
}

async function widthReport(browser: Browser): Promise<WidthReport> {
  return (await browser.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }))) as WidthReport;
}

/** One control that reaches past a viewport edge. */
interface ClippedControl {
  tag: string;
  text: string;
  left: number;
  right: number;
  edge: number;
}

/**
 * Every `button` and `a.px-btn` whose box leaves the viewport horizontally. The
 * page is not scrolled, so a control the learner cannot reach without dragging
 * the page sideways is exactly what this finds.
 */
async function clippedControls(browser: Browser): Promise<ClippedControl[]> {
  return (await browser.evaluate(() => {
    const edge = document.documentElement.clientWidth;
    return Array.from(document.querySelectorAll("button, a.px-btn"))
      .map((node) => {
        const rect = node.getBoundingClientRect();
        return {
          tag: node.tagName.toLowerCase(),
          text: (node.textContent ?? "").trim(),
          left: Math.round(rect.left * 10) / 10,
          right: Math.round(rect.right * 10) / 10,
          edge,
        };
      })
      .filter((control) => control.left < 0 || control.right > control.edge);
  })) as ClippedControl[];
}

test("the menu reaches diagnostics, then keyboard focus and 150 percent text hold", async ({ app, browser, screen }) => {
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

  // 1. The route proof: a learner on the quest map taps "Settings", then the
  //    sub-navigation's "Diagnostics", and lands on the diagnostics page. Both
  //    are real menu entries, so this is the nav contract and not a URL typed in.
  //
  //    Each hop waits for the shell to finish drawing before measuring a point
  //    inside a node: the URL changes before the new screen is rendered, and the
  //    banners arrive after it and push everything below them down. A point read
  //    before either shift is a point somewhere else when the finger lands.
  await app.open("/unit/");
  await shellSettled(browser);
  const settingsLink = screen.getByRole("link", { name: TEXT.navSettings });
  await expect(settingsLink).toBeVisible();
  await tapAt(screen, settingsLink);
  await expect(browser).toHaveURL("/settings/", { timeout: 15_000 });
  const subnav = screen.getByRole("navigation", { name: TEXT.navLabel });
  await expect(subnav).toBeVisible();
  await shellSettled(browser);
  const diagnosticsLink = subnav.getByRole("link", { name: TEXT.navDiagnostics });
  await expect(diagnosticsLink).toBeVisible();
  await tapAt(screen, diagnosticsLink);
  await expect(browser).toHaveURL("/settings/diagnostics/", { timeout: 15_000 });
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.diagTitle })).toBeVisible();

  // 2. Reloaded fresh, so focus starts on the body and the walk below starts
  //    where a keyboard user starts. The first Tab must reach the skip link —
  //    the first focusable node in the document — and it must be a real anchor
  //    to `#main`, not a button that runs script.
  await app.open("/settings/diagnostics/");
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.diagTitle })).toBeVisible();
  await browser.keyboard.press("Tab");
  const skip = await focusReport(browser);
  expect(skip.href, `the first Tab stop is <${skip.tag}> "${skip.text}", not the skip link`).toBe("#main");
  expect(skip.text).toBe(TEXT.navSkip);

  // 3. And it is visibly focused: the ring is resolved by the engine, not just
  //    declared in a stylesheet. A `none` style or a zero width paints nothing,
  //    which is the failure a keyboard user actually meets.
  expect(skip.outlineStyle, `the focused skip link resolves outline-style ${skip.outlineStyle}`).not.toBe("none");
  expect(skip.outlineWidth, "the focused skip link resolves a zero-width outline").toBeGreaterThan(0);

  // 4. Enter follows the link, and the fragment target is focusable, so the
  //    keyboard lands on the main landmark instead of scrolling past it. `main`
  //    carries `tabindex="-1"`: it takes focus without joining the tab order.
  await browser.keyboard.press("Enter");
  const landed = (await browser.evaluate(() => document.activeElement?.id ?? "")) as string;
  expect(landed, "Enter on the skip link did not move focus to the main landmark").toBe("main");

  // 5. The ring is not a one-off for the skip link: the next Tab reaches a real
  //    control and that control draws a ring too. If the stylesheet only styled
  //    the first focusable node, this is where it shows.
  await browser.keyboard.press("Tab");
  const next = await focusReport(browser);
  expect(next.tag, `the second Tab stop is <${next.tag}> "${next.text}", not a control`).not.toBe("body");
  expect(next.outlineStyle, `the second Tab stop resolves outline-style ${next.outlineStyle}`).not.toBe("none");
  expect(next.outlineWidth, "the second Tab stop resolves a zero-width outline").toBeGreaterThan(0);

  // 6. 150 percent text, the measurable part of it. `body` takes its size from
  //    `--text-md`, a rem value, so raising the root to 24px moves the computed
  //    size from 20px to exactly 30px. A px type tree would leave it at 20px and
  //    this pair would be equal — which is the regression this step exists for.
  const scale = (await browser.evaluate(() => {
    const before = Number.parseFloat(getComputedStyle(document.body).fontSize);
    document.documentElement.style.fontSize = "24px";
    const after = Number.parseFloat(getComputedStyle(document.body).fontSize);
    return { before, after };
  })) as { before: number; after: number };
  expect(scale.before, `the body starts at ${scale.before}px, not the 20px this check is written against`).toBe(20);
  expect(
    scale.after,
    `raising the root to 24px left the body at ${scale.after}px, so the type scale is not rem-based`,
  ).toBe(scale.before * 1.5);

  // 7. Nothing gained a sideways scrollbar at 1280 with the larger text.
  const wide = await widthReport(browser);
  expect(
    wide.scrollWidth,
    `at 1280 the document is ${wide.scrollWidth}px wide inside a ${wide.clientWidth}px viewport`,
  ).toBeLessThanOrEqual(wide.clientWidth);

  // 8. Nor at a 400px viewport, where the same text has far less room: the long
  //    sentences and the display headings have to wrap instead of pushing the
  //    page sideways.
  await browser.setViewport(NARROW_VIEWPORT);
  const narrow = await widthReport(browser);
  expect(
    narrow.scrollWidth,
    `at 400 the document is ${narrow.scrollWidth}px wide inside a ${narrow.clientWidth}px viewport`,
  ).toBeLessThanOrEqual(narrow.clientWidth);

  // 9. And no control is clipped by the viewport edge: every button and every
  //    button-shaped link sits wholly between the two edges, so a thumb at 400px
  //    can reach all of them without dragging the page.
  const clipped = await clippedControls(browser);
  expect(clipped, `controls leave the ${NARROW_VIEWPORT.width}px viewport: ${JSON.stringify(clipped)}`).toEqual([]);
});

test("the engines panel shows every engine and its state, offline", async ({ app, browser, screen }) => {
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

  // 1. The panel is the diagnostics page's own section, mounted after the
  //    diagnostics report. Its title is the `h2` `Panel` draws.
  await app.open("/settings/diagnostics/");
  await expect(screen.getByRole("heading", { level: 2, name: TEXT.panelTitle })).toBeVisible();
  await expect(screen.getByText(TEXT.panelIntro)).toBeVisible();

  // 2. Every engine the program reports is named in the list, so a new engine on
  //    the server cannot arrive as a blank row.
  for (const name of TEXT.engines) {
    await expect(screen.getByText(name)).toBeVisible();
  }

  // 3. Every one of them says the same thing offline, and the anchored pattern
  //    is what makes that a real count: `/Not available/` would also swallow
  //    "Not available in this build", while `/^Not available$/` cannot. Six
  //    nodes is six engines, one state word each.
  await expect(screen.getByText(/^Not available$/)).toHaveCount(6);

  // 4. Under each name is the program's own sentence, rendered as it came and
  //    never paraphrased. Six of them, and none blank: an empty detail would be
  //    a row that says nothing about why the engine is missing.
  const details = await browser.locator(".engine-list__detail").allTextContents();
  expect(details, `the panel drew ${details.length} detail sentences, not six`).toHaveLength(6);
  for (const detail of details) {
    expect(detail.trim(), "an engine row carries a blank detail sentence").not.toBe("");
  }

  // 5. The recovery action is offered: the panel's retry re-reads the snapshot,
  //    which is the only thing a learner can do about a missing engine.
  await expect(screen.getByRole("button", { name: TEXT.retry })).toBeVisible();

  // 6. And no control promises something this build cannot do. Offline nothing
  //    can be downloaded or installed, so the panel must not offer either — a
  //    control that could only fail is worse than no control.
  const panel = browser.locator("section.px-panel").filter({ hasText: TEXT.panelTitle });
  await expect(panel).toBeVisible();
  expect(
    await panel.getByText(/download|install/i).count(),
    "the engines panel offers a download or install control that offline could only fail",
  ).toBe(0);

  // 7. The server's single session slot is free, so the next spec can start.
  expect(await activeSessionId(browser), "this spec left a session running for the next one").toBeNull();
});
