import { act, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { Mascot } from "./Mascot";

beforeEach(() => resetBrowserState());
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("Mascot", () => {
  it("is hidden from screen readers unless it has a label", async () => {
    setSystem({ reduced: true });
    const { container } = renderApp(<Mascot />);
    await vi.waitFor(() => expect(container.querySelector("svg")).not.toBeNull());
    expect(container.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
  });

  it("exposes its label as an image", async () => {
    setSystem({ reduced: true });
    renderApp(<Mascot label="Lumi" />);
    expect(await screen.findByRole("img", { name: "Lumi" })).toBeInTheDocument();
  });

  it("never blinks or bobs with motion off", async () => {
    setSystem({ reduced: true });
    vi.useFakeTimers();
    const { container } = renderApp(<Mascot />);
    const before = container.innerHTML;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20000);
    });
    expect(container.innerHTML).toBe(before);
  });

  it("blinks now and then with motion on", async () => {
    setSystem({ reduced: false });
    vi.useFakeTimers();
    // The shortest gap between blinks is 2.6 s, and a blink lasts 140 ms.
    vi.spyOn(Math, "random").mockReturnValue(0);
    const { container } = renderApp(<Mascot />);
    const open = container.innerHTML;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2650);
    });
    expect(container.innerHTML).not.toBe(open);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(150);
    });
    expect(container.innerHTML).toBe(open);
  });
});
