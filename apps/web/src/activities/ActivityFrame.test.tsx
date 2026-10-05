import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { ActivityFrame, type ActivityResult } from "./ActivityFrame";

function frame(result?: ActivityResult, extra: Partial<Parameters<typeof ActivityFrame>[0]> = {}) {
  return (
    <ActivityFrame skill="reading" instructions="Read and choose." canSubmit onSubmit={vi.fn()} result={result} {...extra}>
      <p>body</p>
    </ActivityFrame>
  );
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("ActivityFrame", () => {
  it("shows the instruction and the activity and a Check button", async () => {
    renderApp(frame());
    expect(await screen.findByText("Read and choose.")).toBeInTheDocument();
    expect(screen.getByText("body")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check" })).toBeEnabled();
  });

  it("keeps Check off until the answer is complete enough", async () => {
    renderApp(frame(undefined, { canSubmit: false }));
    expect(await screen.findByRole("button", { name: "Check" })).toBeDisabled();
  });

  it("sends the answer once when Check is pressed and ignores clicks while busy", async () => {
    const onSubmit = vi.fn();
    const user = userEvent.setup();
    const { rerender } = renderApp(frame(undefined, { onSubmit }));
    await user.click(await screen.findByRole("button", { name: "Check" }));
    expect(onSubmit).toHaveBeenCalledOnce();
    rerender(frame(undefined, { onSubmit, busy: true }));
    await user.click(screen.getByRole("button", { name: "Checking" }));
    expect(onSubmit).toHaveBeenCalledOnce();
  });

  it("shows Indonesian help only when it is given, marked with its language", async () => {
    const { rerender } = renderApp(frame());
    await screen.findByText("Read and choose.");
    expect(screen.queryByText("In Indonesian")).toBeNull();
    rerender(frame(undefined, { help: "Baca lalu pilih." }));
    const help = screen.getByText("Baca lalu pilih.");
    expect(help).toHaveAttribute("lang", "id");
  });

  it("says Correct with an explanation and sparks, in words", async () => {
    renderApp(frame({ score: 1, explanation: "She asks about your name.", sparks: 5 }));
    expect(await screen.findByText("Correct!")).toBeInTheDocument();
    expect(screen.getByText("Why: She asks about your name.")).toBeInTheDocument();
    expect(screen.getByText("+5 sparks")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });

  it("shows the accepted answer after a wrong or partial answer, but not after a right one", async () => {
    const { rerender } = renderApp(frame({ score: 0, correctAnswer: "am" }));
    expect(await screen.findByText("Not quite")).toBeInTheDocument();
    expect(screen.getByText("Answer: am")).toBeInTheDocument();
    rerender(frame({ score: 0.5, correctAnswer: "beautiful" }));
    expect(screen.getByText("Almost there")).toBeInTheDocument();
    rerender(frame({ score: 1, correctAnswer: "am" }));
    expect(screen.queryByText("Answer: am")).toBeNull();
  });

  it("says a productive answer is saved when it is waiting for the provider", async () => {
    renderApp(frame({ score: null }));
    expect(await screen.findByText(/Saved\. Lumi will mark this/)).toBeInTheDocument();
  });

  it("offers Next after a result, and announces feedback politely", async () => {
    const onNext = vi.fn();
    const user = userEvent.setup();
    renderApp(frame({ score: 1 }, { onNext }));
    await user.click(await screen.findByRole("button", { name: "Next" }));
    expect(onNext).toHaveBeenCalledOnce();
    expect(screen.getByRole("status")).toHaveAttribute("aria-live", "polite");
  });
});
