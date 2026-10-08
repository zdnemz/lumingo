import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { AnnotatedText, type Segment } from "./AnnotatedText";
import { RevisionSummary } from "./RevisionSummary";

const SEGMENTS: Segment[] = [
  { text: "Yesterday " },
  { text: "I go", error: { id: "e1", label: "Verb tense", correction: "I went", explanation: "Use the past form for finished time." } },
  { text: " to the market with " },
  { text: "my friends is", error: { id: "e2", label: "Agreement", correction: "my friends are", explanation: "Plural subjects take are." } },
  { text: "." },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("AnnotatedText", () => {
  it("shows the whole draft and makes each mistake a labelled button", async () => {
    renderApp(<AnnotatedText segments={SEGMENTS} selectedId={null} onSelect={vi.fn()} />);
    const text = await screen.findByLabelText("Marked text");
    expect(text).toHaveTextContent("Yesterday I go to the market with my friends is.");
    expect(screen.getByRole("button", { name: "Mistake 1 of 2: Verb tense: I go" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Mistake 2 of 2: Agreement: my friends is" })).toBeInTheDocument();
  });

  it("opens the correction and the reason for the chosen mark", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    const { rerender } = renderApp(<AnnotatedText segments={SEGMENTS} selectedId={null} onSelect={onSelect} />);
    await user.click(await screen.findByRole("button", { name: /Mistake 1 of 2/ }));
    expect(onSelect).toHaveBeenCalledWith("e1");

    rerender(<AnnotatedText segments={SEGMENTS} selectedId="e1" onSelect={onSelect} />);
    expect(screen.getByText("Better: I went")).toBeInTheDocument();
    expect(screen.getByText(/Use the past form for finished time/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Mistake 1 of 2/ })).toHaveAttribute("aria-expanded", "true");
  });

  it("closes the explanation from its own button and by choosing the same mark again", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    renderApp(<AnnotatedText segments={SEGMENTS} selectedId="e2" onSelect={onSelect} />);
    await user.click(await screen.findByRole("button", { name: "Close the explanation" }));
    expect(onSelect).toHaveBeenLastCalledWith(null);
    await user.click(screen.getByRole("button", { name: /Mistake 2 of 2/ }));
    expect(onSelect).toHaveBeenLastCalledWith(null);
  });

  it("renders text as text, never as HTML", async () => {
    const hostile: Segment[] = [{ text: "<script>alert(1)</script>", error: { id: "x", label: "<b>L</b>", correction: "<i>c</i>", explanation: "<u>e</u>" } }];
    renderApp(<AnnotatedText segments={hostile} selectedId="x" onSelect={vi.fn()} />);
    await screen.findByLabelText("Marked text");
    expect(document.querySelector("script")?.textContent ?? "").not.toContain("alert");
    expect(document.querySelector("b, i, u")).toBeNull();
  });
});

describe("RevisionSummary", () => {
  it("gives each count its own label so colour is not the only cue", async () => {
    renderApp(<RevisionSummary fixed={3} remaining={1} added={2} />);
    expect(await screen.findByText("Fixed: 3")).toBeInTheDocument();
    expect(screen.getByText("Still there: 1")).toBeInTheDocument();
    expect(screen.getByText("New: 2")).toBeInTheDocument();
  });
});
