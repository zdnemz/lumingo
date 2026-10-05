import { act, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { Typewriter } from "./Typewriter";

beforeEach(() => resetBrowserState());
afterEach(() => vi.useRealTimers());

function visibleText(): string {
  return document.querySelector('[aria-hidden="true"]')?.textContent ?? "";
}

describe("Typewriter", () => {
  it("keeps the full text for screen readers whatever the motion setting", async () => {
    setSystem({ reduced: false });
    renderApp(<Typewriter text="Hello there" />);
    expect(await screen.findByText("Hello there", { selector: ".sr-only" })).toBeInTheDocument();
  });

  it("reveals the text step by step when motion is on", async () => {
    setSystem({ reduced: false });
    vi.useFakeTimers();
    renderApp(<Typewriter text="Hello" speed={10} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(visibleText()).toBe("He");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(visibleText()).toBe("Hello");
  });

  it("shows everything at once and calls onDone when motion is off", async () => {
    setSystem({ reduced: true });
    const done = vi.fn();
    renderApp(<Typewriter text="Hello" onDone={done} />);
    await screen.findByText("Hello", { selector: ".sr-only" });
    expect(visibleText()).toBe("Hello");
    expect(done).toHaveBeenCalled();
  });
});
