# The owner walk

The tests in this repository prove that the surfaces exist and behave. This walk proves that
they work on your machine: with your network, your provider, your display and your browser.
Nothing in this document has been walked yet. It is the checklist, not the result, and it says
nothing about any row having been seen, produced or judged good.

**What this is.** A mechanical checklist for the ten states the interface can render, one
section each. Every section says how to produce the state with the shipped interface and the
program's own controls, what the screen must say while you are in it (quoted from the shipped
strings, with the i18n key next to each sentence), what to write down, and which test proves
that the surface exists. It is meant to be followed in order, offline, at your own keyboard.

**Who it is for.** The owner of this repository, on the machine that runs Lumingo. It is
written for a reader who has the program built and open in front of them, not for a reader of
the source.

**The roadmap box.** S6-11's verify is this walk. Its box in the roadmap stays unticked until
you have done the walk yourself and judged what you saw. No agent, no test and no document can
tick it for you, and this document does not ask you to tick anything.

## Before you start

**Use the program's own build, offline.** Build and start the program as the README's "Get
started" section says: build the interface, build the server, run the server, and let it open
the interface in your browser. Open only the address the program prints in its window. The
program refuses pages opened from anywhere else, on purpose.

**Most rows need no internet at all.** Rows 1, 2, 3, 4, 7, 8, 9 and 10 are produced with the
machine offline. Row 5 needs no internet either, because the address it uses is on your own
computer. Row 6 is the only one that needs a live provider, and it says so.

**Where each screen lives.**

| What | How to get there |
|---|---|
| The shell's banners | Above the content on every page, on every screen except the setup wizard |
| Settings, General | The menu's "Settings", then the sub-navigation's "General" (`/settings/`) |
| The AI provider panel | Settings → General, the first panel, "AI provider" |
| The Speech models panel | Settings → General, further down, "Speech models" |
| Diagnostics, and the engines panel | Settings → Diagnostics (`/settings/diagnostics/`), the panel "Speech and audio engines" is the last section |
| Talk | The menu's "Talk" (`/talk/`) |
| The setup wizard | The program sends a first run here (`/onboarding/`); it is also reachable from Settings |

**The one thing that is easy to get wrong: 150% is the browser's text size, not zoom.**
`Ctrl`+`+` changes the zoom level and is the wrong control for row `text_150`. Use the
browser's own text-size or default-font-size setting, which is a different control in a
different place, and set it to 150%. The whole point of the row is that the interface's type
scale follows the text-size setting; zoom would make the row pass without testing anything.

**Two habits that make the walk mechanical.** Read the sentence on screen before you read this
document's quotation of it, so a drifted string is noticed rather than confirmed. And write the
row down while you are still on the screen, not afterwards from memory.

## 1. `server_not_reachable` — server not reachable

**Produce it.**

1. Start the program as the README says and let it open the interface in your browser. Note the
   address it prints; that is the only address this row is walked at.
2. Leave the page open on any screen. Do not close the browser tab at any point in this row.
3. Close the program's window, or stop it the way you normally stop it. The tab stays open.
4. Look at the top of the page, above the content. The shell's banner region is there.
5. For the cold variant of this state: with the program still stopped and the tab still open,
   choose Settings from the menu, or open any other screen whose data comes from the program.
   A screen that asks the program for something while the program is not there is the cold
   case. A browser tab that has never loaded the page while the program was running has nothing
   to show: what appears then is the browser's own error page, not the program's. If that is
   what you see, record the cold variant as "not produced" and write down why.

**The screen must say.**

- "Connection to the program lost" `banner.lost.title`, with a button "Try now" `banner.lost.action`.
- Under the title: "This page keeps trying by itself. If the program was closed, start it again and this page reconnects." `banner.lost.body`.
- Cold variant: "The program is not answering" `error.unreachable.title`, with a button "Try again" `state.retry`.
- Under that title: "Lumingo runs on your own computer. Check that its window is still open. If you closed it, start it again, then try again here." `error.unreachable.body`.

**Write down.**

- Produced or not produced, and which variant you produced (warm, cold, or both).
- The title sentence you read on screen, and the label of the button or link next to it.
- Whether pressing "Try now" or "Try again" recovered once you started the program again, and
  how long it took.
- If the cold variant showed the browser's own error page instead, that sentence, verbatim.

**Covered by.** `apps/web/src/components/Banners.test.tsx`: the tests "says the connection is
lost and reconnects when asked" and "says the program is not answering when no answer came, and
offers another try". Both render the banner from a dropped connection and from a request that
never got an answer, and both click the recovery action.

## 2. `loading_models` — loading models

**Produce it.**

This row cannot be produced on this line. It needs the speech engines built into the program
and a speech model on disk, and this build has neither: the engines are behind off-by-default
cargo features and the shipped model manifest has no entry that can be fetched yet.

On a machine that has both, the steps are:

1. Start the program with the speech engines built in, as the README's speech section
   describes.
2. Open the interface, go to Talk, and start a voice session, so that the program begins
   loading the model the session needs.
3. While it loads, open Settings → Diagnostics and find the panel "Speech and audio engines".
4. Find the row of the engine that is loading, and watch it until it settles.

On this line, record "not produced" and do not tick. The fixture below is what proves the row
exists; it does not make the row produced.

**The screen must say.**

- In the engines panel, the loading engine's row says "Loading the model" `engine.state.loading`.
- When the model is in memory, the same row says "Ready" `engine.state.ready`, and the model's
  own id and version appear under the sentence.
- The sentence under the engine's name is the program's own. It has no fixed wording: write
  down what it said, do not expect a particular sentence.
- The panel offers "Try again" `state.retry`.

**Write down.**

- Not produced, and the reason (no speech engines in this build, no model on disk).
- If you did produce it on another machine: the engine's name, the state word before and after
  the load, the program's own sentence, and whether the row ever said "Ready".

**Covered by.** Fixtures only, and the reason is the one above: with no speech engines and no
model on disk, no browser run on this line can reach a loading row, so the state is proven from
a fixture instead of from a live load. `apps/web/src/components/EngineStatusPanel.test.tsx`:
the tests "says an engine is loading its model" and "says an engine is ready and shows the
model the engine itself reported".

## 3. `model_missing` — model missing

**Produce it.**

This row has two halves, and one of them is producible on this line.

1. Open Settings → General and find the panel "Speech models". Every entry the program has not
   installed says the sentence quoted below. On this line that is every entry, so this half is
   produced by opening the panel.
2. For the engine half: on a machine where a model is installed, rename one of the model files
   that the engines file names, restart the program, and open Settings → Diagnostics.
3. Find the row of the engine whose file you renamed, in the panel "Speech and audio engines".

**The screen must say.**

- In the Speech models panel, for an entry that is not installed: "Not installed on this computer yet." `settings.models.notinstalled`.
- Where the program itself says why an entry cannot be fetched: "Why it cannot be downloaded: {reason}" `settings.models.why`, with the placeholder filled by the program's own sentence. Quote that sentence as it appears; it has no fixed wording.
- In the engines panel, the engine whose file is missing says "Not available" `engine.state.unavailable`, with the program's own sentence about the missing file under it.
- The engines panel offers "Try again" `state.retry`.

**Write down.**

- Which half you produced: the panel sentence, the engine row, or both.
- The program's own sentences, verbatim, from both places.
- If this machine has no installed model to rename, record the engine half as "not produced" and
  say so. The panel half alone does not make the whole row produced.

**Covered by.** `apps/web/src/settings/ModelsPanel.test.tsx`: the tests "shows the installed
record when the program has one, and says so when it has none" and "gives the server's own
reason and no download control when the entry cannot be downloaded". The engine row is proven
in `apps/web/src/components/EngineStatusPanel.test.tsx`: "says an engine is not available, and
repeats the program's own reason a file is missing".

## 4. `provider_not_configured` — provider not configured

**Produce it.**

1. Close the program.
2. Start it with a data folder that has never held a provider profile: the program's
   `--data-dir` option points it at a folder you have just created and left empty. Make sure
   there are no provider values in your environment or in a `.env` file next to the program,
   because a profile from either of those counts as configured.
3. The program opens the setup wizard on a first run. Choose Settings in the menu, or any other
   screen except the setup wizard: the banner is deliberately not shown on the wizard itself.
4. Look above the content.

**The screen must say.**

- "No AI provider is set up" `banner.noprovider.title`, with a link "Set up a provider" `banner.noprovider.action`.
- Under the title: "The tutor needs an AI provider. Text you write is sent to the provider you choose." `banner.noprovider.body`.

**Write down.**

- Produced or not produced.
- The title sentence, and the label of the link beside it.
- Whether the link led to the setup wizard.
- Whether the banner was absent on the wizard and present on the other screens, as described.

**Covered by.** `apps/web/src/components/Banners.test.tsx`: "warns when no provider is set up,
with a link to the setup guide". It renders the banner from a snapshot with no provider and
checks that the link points at the setup wizard.

## 5. `provider_unreachable` — provider unreachable

**Produce it.**

1. Open Settings → General and find the panel "AI provider".
2. Choose "Add a profile".
3. Fill in a profile name, the OpenAI-compatible protocol, and a base address on this computer
   where nothing is listening, for example port 9 on the loopback address. Write it in full,
   with the `http` or `https` prefix in front: the form refuses an address without one, and a
   server on this computer may use either. Add a model name, and leave the key empty, which is
   allowed for a server on this computer that needs no key.
4. Save the profile, then press "Test the connection" on that profile's row.
5. Wait for the test to finish. It fails, which is the point of the row.

**The screen must say.**

- "The provider could not be reached" `probe.fail.network.title`, with a button "Test again" `probe.action.retest` and a button "Edit the profile" `probe.action.edit`.
- Under the title: "Check your internet connection and the base URL. If both are right, the provider may be down. Try again in a minute." `probe.fail.network.body`.
- The program's own detail sentence, on a line of its own, prefixed as the interface prefixes it.

**Write down.**

- Produced or not produced.
- The title sentence, both button labels, and the program's own detail line.
- Whether "Edit the profile" opened the form with the address still in it, and whether "Test
  again" re-ran the test rather than doing nothing.
- Delete the throwaway profile when you are done with the row.

**Covered by.** `apps/web/src/providers/ProbeResult.test.tsx`: "an unreachable provider offers
the address as well as another try". It renders the network failure and checks that the edit
control is offered and that the wrong-key control is not.

## 6. `rate_limited` — rate limited

**Produce it.**

This row cannot be produced on this line. It needs a live key and a provider that answers with
a rate-limit refusal (HTTP status 429). No test in this repository sends a request to a
provider, and the offline build has no key at all.

On a machine with a live key, the steps are:

1. Open Settings → General, find the panel "AI provider", and make sure the profile you want to
   use has a key saved and is the one in use.
2. Press "Test the connection" on that profile.
3. Repeat the test until the provider itself refuses with a rate limit. On a free tier this
   takes a handful of tests in a minute; the refusal comes from the provider, not from the
   program.
4. Stop as soon as the screen shows the sentence below.

On this line, record "not produced" and do not tick.

**The screen must say.**

- "The provider is limiting requests" `probe.fail.rate_limited.title`, with a button "Test again" `probe.action.retest`.
- Under the title: "Free tiers allow only a few requests. Wait a minute, then test again." `probe.fail.rate_limited.body`.
- The program's own detail sentence, on a line of its own, if the provider sent one.

**Write down.**

- Not produced, and the reason (no live key, no provider answering 429).
- If you did produce it: the title sentence, the detail line, and whether waiting a minute and
  pressing "Test again" got a successful test.

**Covered by.** Fixtures only, and the reason is the one above: a live 429 cannot be provoked
from this repository, so the state is proven from a recorded failure rather than from a live
one. `apps/web/src/providers/ProbeResult.test.tsx`: "a %s failure is an alert with a recovery
action and the program's details line", whose `rate_limited` row is this state and checks that
the alert carries a recovery action.

## 7. `out_of_memory` — out of memory

**Produce it.**

This row cannot be produced on this line. It needs the speech engines built in and a model far
larger than the machine's free memory, and this build has neither the engines nor a model on
disk.

On a machine that has both, the steps are:

1. Start the program with the speech engines built in.
2. Load a model far larger than the memory the machine has free, through a voice session or
   through whatever control the program offers for that model.
3. Open Settings → Diagnostics and find the row of that engine in the panel "Speech and audio
   engines".
4. Read the row, including the program's own sentence under the engine's name.

On this line, record "not produced" and do not tick.

**The screen must say.**

- The engine's row says "Failed" `engine.state.failed`.
- Under the name, the program's own sentence about what happened. It has no fixed wording: write
  it down as it appears.
- The panel offers "Try again" `state.retry`.
- Nothing on the row names a cause the program did not give. The interface has no memory-specific
  state word, so the row says "Failed" and repeats the program's sentence. That is the honest
  shape of this state today, and this row of the walk is where you check that no extra cause was
  invented on the screen.

**Write down.**

- Not produced, and the reason (no speech engines, no model on disk).
- If you did produce it: the state word, the program's own sentence verbatim, and whether the
  screen added any cause of its own.

**Covered by.** Fixtures only, and the reason is the one above: a browser run cannot exhaust a
real model's memory on purpose, so the state is proven from a fixture. The fixture's sentence is
an allocation failure, and the test checks that the row adds nothing to it.
`apps/web/src/components/EngineStatusPanel.test.tsx`: "says an engine failed and adds no cause
of its own to the program's sentence".

## 8. `microphone_lost` — microphone lost

**Produce it.**

This row cannot be produced on this line. It needs a voice conversation that is running when
the microphone stops answering, and this build has no voice loop, so no session can reach that
state.

On a machine with the voice loop, the steps are:

1. Open Talk and start a voice conversation, with a microphone connected.
2. While the conversation is running, unplug the microphone, or disable it in the operating
   system's sound settings.
3. Look at the banner region at the top of the page, wherever you are standing.
4. Follow the banner's link if you want to see the engine's own sentence on Diagnostics.

On this line, record "not produced" and do not tick.

**The screen must say.**

- "An engine stopped this session" `engine.fault.title`, with a link "Open the diagnostics" `engine.fault.action`.
- Under the title: "The microphone stopped answering. Check that it is still connected, then start again." `engine.fault.microphone`.
- On Diagnostics, the engine's row carries the program's own sentence about the fault.

**Write down.**

- Not produced, and the reason (no voice loop in this build).
- If you did produce it: the title, the fault sentence, whether the banner appeared on the screen
  you were on rather than only on Diagnostics, and where the link took you.

**Covered by.** Fixtures only, and the reason is the one above: with no voice loop there is no
running session to interrupt, so the banner is proven from a session fixture.
`apps/web/src/components/EngineFaultBanner.test.tsx`: "interrupts with the microphone fault,
and links to the diagnostics", which renders the banner from a faulted session and checks the
sentence and the link's destination.

## 9. `keyboard` — keyboard

**Produce it.**

1. Open any screen, for example Settings → Diagnostics, and put the pointer down. Use the
   keyboard only for this row.
2. Before clicking anything on the page, press `Tab` once. Focus starts on the page body, so the
   first `Tab` reaches the first focusable thing in the document.
3. Look at what took focus, and at the ring drawn around it.
4. Press `Enter`.
5. Press `Tab` once more, and look at the ring on the next control.

**The screen must say.**

- The first `Tab` stop is the link "Skip to content" `nav.skip`, and it is visibly ringed: an
  outline you can see against the page, not a colour change you have to hunt for.
- `Enter` on that link moves the focus to the page's content, so the next `Tab` continues from
  the content and not from the top of the menu again.
- The second `Tab` stop is a real control, and it draws a ring of its own. A ring that only the
  first element gets is the failure this step is for.

**Write down.**

- Produced or not produced.
- What took the first `Tab`, what the ring looked like, and whether `Enter` landed on the content.
- Whether the ring was visible on your display at your usual brightness and distance. That is a
  judgement about your screen, and only you can make it.

**Covered by.** `tests/a11y.e2e.ts`: "the menu reaches diagnostics, then keyboard focus and 150
percent text hold". It runs in real Chromium, presses `Tab` and `Enter`, and reads the resolved
outline of the focused element rather than the stylesheet. The same contract at component level
is in `apps/web/src/components/AppShell.a11y.test.tsx`: "is the first thing the keyboard reaches,
and it jumps to the main landmark" and "draws a visible focus ring for keyboard users".

## 10. `text_150` — 150% text

**Produce it.**

1. Open Settings → Diagnostics, so the page has headings, long sentences, a list and buttons on
   it.
2. Raise the browser's text size to 150%: use the browser's own text-size or default-font-size
   setting, not the zoom control. `Ctrl`+`+` is zoom and is the wrong control for this row.
3. Read the page. Look at the headings, the sentences, the list of engines and the buttons.
4. Narrow the window to about a third of the screen's width and look again. The same text now
   has far less room.
5. Put the text size back the way it was.

**The screen must say.**

- No new sentence appears for this row. What must hold is that every sentence already on the page
  is still there, whole, and readable. For example: "Skip to content" `nav.skip` at the top,
  "Where each engine stands in this run. The sentence under a name is the program's own." `engine.panel.intro`, and "Try again" `state.retry` on the engines panel.
- Nothing is cut off at the end of a line, nothing runs off the side of the window, and no
  button is clipped by the window's edge.

**Write down.**

- Produced or not produced.
- The text size you set, and how you set it (the browser's setting, not zoom).
- Whether anything overflowed sideways, and whether any control was clipped.
- Whether the text was comfortable to read, or merely unclipped. Those are different results and
  the second one is not a pass.

**Covered by.** `tests/a11y.e2e.ts`: "the menu reaches diagnostics, then keyboard focus and 150
percent text hold", which raises the root font size to 24px in real Chromium, measures that the
body text grows to exactly 1.5 times its size, and then checks for sideways overflow and clipped
controls at a narrow viewport. The type scale itself is pinned by
`apps/web/src/styles/tokens.test.ts`: "is rem, and renders at the px size the old scale had at a
16px root". One limit, said plainly: that browser test raises the root font size from the page,
which is not the same act as changing the browser's text-size setting. It proves the type scale
follows a root change; your step 2 is what checks your browser's own control.

## What is yours to judge

The rows above tell you what to produce and what the screen must say. They cannot tell you
whether any of it is good enough. That is the part of this walk that is yours, and it is the
part no test in this repository can stand in for.

- **That each recovery action actually recovers on your machine.** "Try now" reconnects the page,
  "Try again" re-reads the program's state, "Test again" re-runs the connection test, "Set up a
  provider" leads to the setup, "Open the diagnostics" lands on the page that names the fault.
  Whether the learner's problem is actually gone afterwards is a judgement about your machine,
  your provider and your network, and it is made by you, not by a test.
- **That the ring is visible on your display.** The test measures a non-zero outline in a
  headless browser. Whether you can see it, at your brightness and your distance, is a different
  question.
- **That 150% is comfortable, not merely unclipped.** Nothing overflowing is the floor. Whether
  the page is pleasant to read at 150% is the thing the row is really asking.
- **Whether the tick is earned.** If a row was not produced, or a recovery action did not
  recover, or the ring was invisible, or 150% was unreadable, the tick is not earned yet, and the
  honest record says so.

## When a row cannot be produced

Some rows cannot be produced on this line, and some may not be producible on your machine
either: `loading_models` and `out_of_memory` need the speech engines built in and a model on
disk, `microphone_lost` needs a voice loop this build does not have, and `rate_limited` needs a
live key and a provider that answers with a rate limit. Each of those sections says what to do
and which fixture proves the surface exists.

For any row you could not produce, on this line or on your machine:

1. Write "not produced" in that row's **Write down**, with the reason and what you tried.
2. Write what was on screen instead, verbatim, even if it was nothing.
3. Leave the roadmap's S6-11 box unticked. A row that was not seen is not a row that was judged
   good, and a fixture that proves a surface exists is not a walk that saw it work.

"Not produced" is a recorded fact about this machine and this build, not a hole in the
checklist. What would be dishonest is the other thing: writing a row down as good because a test
somewhere passes.
