import { test, type Browser } from "@e2e-dev/web";
import { expect, type Locator, type Screen } from "e2e";

/**
 * The conversation screen, end to end, against the offline server
 * `pnpm test:e2e` starts by itself.
 *
 * The spec reaches the screen through the menu, which is what proves the nav
 * entry end to end: a learner who lands on any shell page taps "Talk" and gets
 * the conversation screen. The screen is opened with the language pinned to
 * English, the language the strings below are read in.
 *
 * Offline there is no provider, and this build has no microphone, speakers, VAD,
 * STT or TTS, so the two halves of the screen are both honest about what they
 * cannot do: the voice channel says the speech part is missing and offers no
 * control at all, and a text chat is refused by the server itself with the coded
 * `provider_not_configured`, which the screen shows as it came. The spec asserts
 * both, and never starts a session that succeeds, because offline there is none:
 * the refusal leaves the session slot free, which is checked at the end.
 *
 * Interactions use `screen.tapAt` at a point inside the node rather than
 * `locator.tap`. Both are the same learner gesture, but a locator tap first
 * waits for the node to hold still across animation frames, and this headless
 * browser stops producing frames once the page has settled and nothing is
 * painted — so that wait never ends. A point tap is dispatched straight at the
 * point the node reports, which is what a person's finger does. The page is laid
 * out in a viewport tall enough to hold the whole screen, so a point is always
 * on screen and nothing has to be scrolled.
 */

/** A viewport tall enough that the whole screen fits: no scrolling, so a point tap always lands. */
const TALL_VIEWPORT = { width: 1280, height: 6000 };

/** The screen's own strings, from `apps/web/src/i18n/parts/talk.en.ts` and `shell.en.ts`. */
const TEXT = {
  /** `nav.talk`: the menu entry this spec walks through. */
  navTalk: "Talk",
  /** `talk.title`, the page's own heading. */
  title: "Talk with Lumi",
  /** `talk.channel`, the legend of the channel switch. */
  channel: "How to talk",
  /** `talk.channel.text`. */
  channelText: "By typing",
  /** `talk.channel.voice`. */
  channelVoice: "By speaking",
  /** `talk.mode`, the legend of the feedback-mode switch. */
  mode: "Feedback while we talk",
  /** `talk.mode.fluency`, the mode the screen starts in. */
  fluency: "Fluency: keep going, no interruptions",
  /** `talk.mode.accuracy`. */
  accuracy: "Accuracy: correct me as we go",
  /** `talk.topic`, the label of the topic field. */
  topic: "What shall we talk about?",
  /** `talk.start`. */
  start: "Start talking",
  /** `talk.conversation`, the panel only a running session draws. */
  conversation: "The conversation",
  /** `unavailable.title`, the note the voice channel shows instead of a control. */
  unavailableTitle: "Not available in this build",
  /** `unavailable.body` with `feature.speech`: this build has no speech part. */
  unavailableBody:
    "Speech: microphone, speakers and voices: not part of this version of the program yet, so there is no control for it here.",
  /** `error.code.provider_not_configured`: the server's coded refusal, which offline is the honest state. */
  refusalTitle: "No AI provider is set up yet. Add one first.",
  /** `state.details`, the server's own sentence on the details line of that banner. */
  refusalDetails: "Details from the program: no provider is configured",
  /** Every voice control of `talk.en.ts` (`mic.*`): none of them may be offered while speech is missing. */
  voiceControls: /Hold to talk|Microphone|Stop Lumi talking/,
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

test("the conversation screen refuses the offline start with the server's own code", async ({ app, browser, screen }) => {
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

  // 1. The screen is reached through the menu. The picker page is a shell page
  //    with no first-run gate, so a learner who has set nothing up still has the
  //    menu in front of them.
  await app.open("/unit/");
  await tapAt(screen, screen.getByRole("link", { name: TEXT.navTalk }));
  await expect(browser).toHaveURL("/talk/", { timeout: 15_000 });
  await expect(screen.getByRole("heading", { level: 1, name: TEXT.title })).toBeVisible();

  // 2. The typing half: the channel switch, the feedback-mode switch and the
  //    topic field are all on screen, with the fluency mode and the typing
  //    channel the screen starts in. Each switch is a radio group named by its
  //    own legend.
  const channelGroup = screen.getByRole("group", { name: TEXT.channel });
  await expect(channelGroup).toBeVisible();
  await expect(channelGroup.getByRole("radio", { name: TEXT.channelText })).toBeChecked();
  const modeGroup = screen.getByRole("group", { name: TEXT.mode });
  await expect(modeGroup).toBeVisible();
  await expect(modeGroup.getByRole("radio", { name: TEXT.fluency })).toBeChecked();
  await expect(modeGroup.getByRole("radio", { name: TEXT.accuracy })).toBeVisible();
  await expect(screen.getByLabel(TEXT.topic)).toBeVisible();

  // 3. The voice half is reported as missing, never faked: the note names the
  //    speech part this build does not contain, and no voice control is drawn —
  //    not a push-to-talk button, not a microphone, and no start either.
  await tapAt(screen, screen.getByRole("radio", { name: TEXT.channelVoice }));
  await expect(screen.getByText(TEXT.unavailableTitle)).toBeVisible();
  await expect(screen.getByText(TEXT.unavailableBody)).toBeVisible();
  const voiceNote = browser.locator(".unavailable");
  expect(await voiceNote.count()).toBe(1);
  expect(await voiceNote.getByRole("button").count()).toBe(0);
  expect(await screen.getByRole("button", { name: TEXT.voiceControls }).count()).toBe(0);
  expect(await screen.getByRole("button", { name: TEXT.start }).count()).toBe(0);
  expect(await screen.getByLabel(TEXT.topic).count()).toBe(0);
  expect(await screen.getByRole("group", { name: TEXT.mode }).count()).toBe(0);

  // 4. Back to typing: the topic field and the start control return, because the
  //    voice half has no control of its own to go back from.
  await tapAt(screen, screen.getByRole("radio", { name: TEXT.channelText }));
  const topic = screen.getByLabel(TEXT.topic);
  await expect(topic).toBeVisible();
  await topic.fill("Food in my city");

  // 5. Starting a chat offline is refused by the server with its own code: the
  //    coded line names the missing provider and the details line carries the
  //    server's sentence, not a sentence this page wrote.
  const start = screen.getByRole("button", { name: TEXT.start });
  await expect(start).toBeEnabled();
  await tapAt(screen, start);
  const refusal = screen.getByRole("alert").filter({ hasText: TEXT.refusalTitle });
  await expect(refusal).toContainText(TEXT.refusalTitle, { timeout: 15_000 });
  await expect(refusal).toContainText(TEXT.refusalDetails);

  // 6. The refusal is the state, not the end: the start control stays, and no
  //    conversation panel appeared, because no session was ever opened.
  await expect(screen.getByRole("button", { name: TEXT.start })).toBeVisible();
  expect(await screen.getByText(TEXT.conversation).count()).toBe(0);

  // 7. And the server really has no session: nothing was left running for the
  //    next spec, which would be refused with `Conflict`.
  expect(await activeSessionId(browser)).toBeNull();
});
