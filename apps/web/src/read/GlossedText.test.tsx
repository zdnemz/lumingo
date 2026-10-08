import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { GlossedText, splitSentences } from "./GlossedText";

const TEXT = "Dewi lives in a small town. She goes to the market every Sunday. It is busy!";
const GLOSSARY = [
  { word: "market", gloss: "pasar" },
  { word: "busy", gloss: "ramai" },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("splitSentences", () => {
  it("keeps every character and ends sentences at . ! ?", () => {
    const parts = splitSentences(TEXT);
    expect(parts).toHaveLength(3);
    expect(parts.join("")).toBe(TEXT);
  });

  it("keeps text with no final mark as one last sentence", () => {
    expect(splitSentences("One. Two without end")).toEqual(["One. ", "Two without end"]);
    expect(splitSentences("")).toEqual([]);
  });
});

describe("GlossedText", () => {
  it("makes glossary words buttons and leaves other words as plain text", async () => {
    renderApp(<GlossedText text={TEXT} glossary={GLOSSARY} openWord={null} onOpen={vi.fn()} />);
    expect(await screen.findByRole("button", { name: "Word with a meaning: market" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Word with a meaning: busy" })).toBeInTheDocument();
    expect(screen.getAllByRole("button")).toHaveLength(2);
    expect(screen.getByLabelText("Reading text")).toHaveTextContent(TEXT);
  });

  it("opens the meaning for the chosen word and closes it again", async () => {
    const onOpen = vi.fn();
    const user = userEvent.setup();
    const { rerender } = renderApp(<GlossedText text={TEXT} glossary={GLOSSARY} openWord={null} onOpen={onOpen} />);
    await user.click(await screen.findByRole("button", { name: /market/ }));
    expect(onOpen).toHaveBeenCalledWith("market");
    rerender(<GlossedText text={TEXT} glossary={GLOSSARY} openWord="market" onOpen={onOpen} />);
    expect(screen.getByText("market means pasar")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close the meaning" }));
    expect(onOpen).toHaveBeenLastCalledWith(null);
  });

  it("matches glossary words without regard to case", async () => {
    renderApp(<GlossedText text="Busy streets." glossary={GLOSSARY} openWord={null} onOpen={vi.fn()} />);
    expect(await screen.findByRole("button", { name: "Word with a meaning: Busy" })).toBeInTheDocument();
  });

  it("marks the sentence being read aloud", async () => {
    renderApp(<GlossedText text={TEXT} glossary={GLOSSARY} openWord={null} onOpen={vi.fn()} currentSentence={1} />);
    await screen.findByLabelText("Reading text");
    const current = document.querySelectorAll("[data-current]");
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveTextContent("She goes to the market every Sunday.");
  });
});
