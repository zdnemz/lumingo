import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { ChatInput } from "./ChatInput";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("ChatInput", () => {
  it("sends the trimmed text on Enter and clears the box", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderApp(<ChatInput onSend={onSend} />);
    const box = await screen.findByLabelText("Your message");
    await user.type(box, "  Hello there  {Enter}");
    expect(onSend).toHaveBeenCalledWith("Hello there");
    expect(box).toHaveValue("");
  });

  it("makes a new line on Shift and Enter instead of sending", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderApp(<ChatInput onSend={onSend} />);
    const box = await screen.findByLabelText("Your message");
    await user.type(box, "one{Shift>}{Enter}{/Shift}two");
    expect(onSend).not.toHaveBeenCalled();
    expect(box).toHaveValue("one\ntwo");
  });

  it("never sends a blank message, and the Send button stays disabled", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderApp(<ChatInput onSend={onSend} />);
    const box = await screen.findByLabelText("Your message");
    await user.type(box, "   {Enter}");
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  });

  it("sends with the button", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderApp(<ChatInput onSend={onSend} />);
    await user.type(await screen.findByLabelText("Your message"), "Hi");
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(onSend).toHaveBeenCalledWith("Hi");
  });

  it("does not send twice while busy", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderApp(<ChatInput onSend={onSend} busy />);
    await user.type(await screen.findByLabelText("Your message"), "Hi{Enter}");
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Sending" })).toHaveAttribute("aria-busy", "true");
  });

  it("describes how Enter works for the person typing", async () => {
    renderApp(<ChatInput onSend={vi.fn()} />);
    const box = await screen.findByLabelText("Your message");
    expect(box).toHaveAccessibleDescription("Enter sends. Shift and Enter makes a new line.");
  });
});
