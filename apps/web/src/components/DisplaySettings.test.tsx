import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { PREFS_KEY } from "@/state/preferences";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { DisplaySettings } from "./DisplaySettings";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: false, dark: true, language: "en-US" });
});

describe("DisplaySettings", () => {
  it("follows the system by default and applies it to the page", async () => {
    renderApp(<DisplaySettings />);
    // Motion and theme both default to "Follow system".
    expect(await screen.findAllByRole("radio", { name: "Follow system", checked: true })).toHaveLength(2);
    expect(document.documentElement.dataset.motion).toBe("on");
    expect(document.documentElement.dataset.theme).toBe("night");
  });

  it("starts still when the system asks for reduced motion", async () => {
    setSystem({ reduced: true });
    renderApp(<DisplaySettings />);
    await screen.findByText("Motion");
    expect(document.documentElement.dataset.motion).toBe("off");
  });

  it("turns motion off, applies it at once, and remembers it", async () => {
    const user = userEvent.setup();
    renderApp(<DisplaySettings />);
    const group = await screen.findByRole("group", { name: "Motion" });
    await user.click(group.querySelector('input[value="off"]') as HTMLElement);
    expect(document.documentElement.dataset.motion).toBe("off");
    expect(JSON.parse(window.localStorage.getItem(PREFS_KEY) ?? "{}").motion).toBe("off");
  });

  it("lets motion be forced on even when the system asks for less", async () => {
    setSystem({ reduced: true });
    const user = userEvent.setup();
    renderApp(<DisplaySettings />);
    const group = await screen.findByRole("group", { name: "Motion" });
    await user.click(group.querySelector('input[value="on"]') as HTMLElement);
    expect(document.documentElement.dataset.motion).toBe("on");
  });

  it("switches the language and the page language attribute", async () => {
    const user = userEvent.setup();
    renderApp(<DisplaySettings />);
    await user.click(await screen.findByRole("radio", { name: "Bahasa Indonesia" }));
    expect(document.documentElement.lang).toBe("id");
    expect(await screen.findByText("Gerakan")).toBeInTheDocument();
  });

  it("guesses Indonesian for an Indonesian browser", async () => {
    setSystem({ language: "id-ID" });
    renderApp(<DisplaySettings />);
    expect(await screen.findByText("Gerakan")).toBeInTheDocument();
  });

  it("toggles the scanline overlay as a labelled switch", async () => {
    const user = userEvent.setup();
    renderApp(<DisplaySettings />);
    const toggle = await screen.findByRole("switch", { name: /Scanline overlay/ });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(toggle).toHaveTextContent("OFF");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(toggle).toHaveTextContent("ON");
    expect(document.documentElement.dataset.crt).toBe("on");
  });

  it("keeps working when storage is blocked", async () => {
    const user = userEvent.setup();
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = () => {
      throw new Error("blocked");
    };
    try {
      renderApp(<DisplaySettings />);
      await user.click(await screen.findByRole("radio", { name: "Day" }));
      expect(document.documentElement.dataset.theme).toBe("day");
    } finally {
      Storage.prototype.setItem = original;
    }
  });
});
