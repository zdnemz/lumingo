import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { MicOrb } from "./MicOrb";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("MicOrb", () => {
  it("says what is happening in words for every state", async () => {
    const cases: [Parameters<typeof MicOrb>[0]["state"], string][] = [
      ["idle", "Ready. Say something."],
      ["listening", "Listening"],
      ["transcribing", "Writing down what you said"],
      ["thinking", "Lumi is thinking"],
      ["speaking", "Lumi is speaking"],
      ["paused", "Paused"],
      ["error", "The microphone has a problem"],
    ];
    for (const [state, text] of cases) {
      const { unmount } = renderApp(<MicOrb state={state} level={50} />);
      expect(await screen.findByRole("status")).toHaveTextContent(text);
      unmount();
    }
  });

  it("shows the microphone level only while listening", async () => {
    const { rerender } = renderApp(<MicOrb state="listening" level={64} />);
    expect((await screen.findByRole("progressbar", { name: "Microphone level" })).getAttribute("aria-valuenow")).toBe("64");
    rerender(<MicOrb state="thinking" level={64} />);
    expect(screen.getByRole("progressbar", { name: "Microphone level" }).getAttribute("aria-valuenow")).toBe("0");
  });

  it("offers Stop while Lumi speaks and sends the learner's stop", async () => {
    const stop = vi.fn();
    const user = userEvent.setup();
    renderApp(<MicOrb state="speaking" level={0} onStopSpeaking={stop} />);
    await user.click(await screen.findByRole("button", { name: "Stop Lumi talking" }));
    expect(stop).toHaveBeenCalledOnce();
  });

  it("offers Resume while paused and Pause otherwise, but no pause on an error", async () => {
    const resume = vi.fn();
    const pause = vi.fn();
    const user = userEvent.setup();
    const { unmount } = renderApp(<MicOrb state="paused" level={0} onResume={resume} onPause={pause} />);
    await user.click(await screen.findByRole("button", { name: "Resume" }));
    expect(resume).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    unmount();

    const second = renderApp(<MicOrb state="listening" level={0} onPause={pause} />);
    await user.click(await screen.findByRole("button", { name: "Pause" }));
    expect(pause).toHaveBeenCalledOnce();
    second.unmount();

    renderApp(<MicOrb state="error" level={0} onPause={pause} />);
    await screen.findByRole("status");
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
  });
});
