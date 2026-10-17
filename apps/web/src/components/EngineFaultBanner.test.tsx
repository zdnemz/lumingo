import { screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { ActiveSessionView } from "@/generated/ActiveSessionView";
import type { EngineFaultView } from "@/generated/EngineFaultView";
import { defaultPreferences, savePreferences } from "@/state/preferences";
import { SNAPSHOT, connect, fakeApi, renderApp, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { EngineFaultBanner, EngineFaultNotice } from "./EngineFaultBanner";

/**
 * The sentence each fault shows, spelled out. The record type makes a fifth
 * server variant a compile error here too, and the wording is pinned so a
 * variant wired to the wrong key cannot pass.
 */
const FAULT_SENTENCE: Record<EngineFaultView, string> = {
  microphone: "The microphone stopped answering. Check that it is still connected, then start again.",
  speech_recognition: "Speech recognition stopped. Check the microphone, then start again.",
  speech_synthesis: "The voice stopped. Start again to hear the reply.",
  playback: "The speaker stopped answering. Check that it is still connected, then start again.",
};

/** Every variant, so the loop below cannot quietly skip one. */
const FAULTS = Object.entries(FAULT_SENTENCE) as Array<[EngineFaultView, string]>;

const TITLE = "An engine stopped this session";
const ACTION = "Open the diagnostics";
const DIAGNOSTICS = /^\/settings\/diagnostics\/?$/;

/** One running session, every field spelled out so a test overrides only what it cares about. */
function sessionView(patch: Partial<ActiveSessionView> = {}): ActiveSessionView {
  return {
    id: 7,
    kind: "conversation",
    unit_id: null,
    channel: "voice",
    life: "active",
    turn_state: "listening",
    fault: null,
    turns_completed: 2,
    turn: 3,
    recent: [],
    pending_reply: null,
    speaking: false,
    activity_id: null,
    ...patch,
  };
}

/** A session the engine stopped, with the fault that stopped it. */
function faulted(fault: EngineFaultView): ActiveSessionView {
  return sessionView({ life: "engine_error", turn_state: null, fault });
}

beforeEach(() => {
  resetBrowserState();
  setSystem({});
  // The banner's sentences come from the stored preferences, so pin the language
  // to English, and turn motion off so no blink timer runs here.
  savePreferences(window, { ...defaultPreferences("en"), motion: "off" });
});

describe("EngineFaultBanner", () => {
  it("says nothing when there is no session", () => {
    const { container } = renderApp(<EngineFaultBanner session={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("says nothing while the session is running normally", () => {
    const { container } = renderApp(<EngineFaultBanner session={sessionView()} />);
    expect(container).toBeEmptyDOMElement();
  });

  for (const [fault, sentence] of FAULTS) {
    it(`interrupts with the ${fault} fault, and links to the diagnostics`, () => {
      renderApp(<EngineFaultBanner session={faulted(fault)} />);

      const banner = screen.getByRole("alert");
      expect(banner).toHaveTextContent(TITLE);
      expect(banner).toHaveTextContent(sentence);

      const link = screen.getByRole("link", { name: ACTION });
      expect(link).toHaveAttribute("href", expect.stringMatching(DIAGNOSTICS));
    });
  }

  it("says nothing once the session has ended, even if a fault is left behind", () => {
    const { container } = renderApp(<EngineFaultBanner session={sessionView({ life: "ended", turn_state: null, fault: "microphone" })} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("leaves the provider-unavailable state to the connection banner", () => {
    const { container } = renderApp(<EngineFaultBanner session={sessionView({ life: "provider_unavailable", turn_state: null })} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("says nothing when the engine error carries no fault to name", () => {
    const { container } = renderApp(<EngineFaultBanner session={sessionView({ life: "engine_error", turn_state: null })} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("EngineFaultNotice", () => {
  it("says nothing before the first snapshot, so the shell can render without one", () => {
    renderScreen(<EngineFaultNotice />, fakeApi());
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("link", { name: ACTION })).toBeNull();
  });

  it("shows the banner when a snapshot says an engine stopped the session", async () => {
    const api = fakeApi();
    renderScreen(<EngineFaultNotice />, api);
    await connect(api, { ...SNAPSHOT, active_session: faulted("speech_synthesis") });
    expect(await screen.findByRole("alert")).toHaveTextContent(FAULT_SENTENCE.speech_synthesis);
  });

  it("shows nothing when the snapshot has no session running", async () => {
    const api = fakeApi();
    renderScreen(<EngineFaultNotice />, api);
    await connect(api, { ...SNAPSHOT, active_session: null });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
