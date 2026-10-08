import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { PhonemeChips, type WordResult } from "./PhonemeChips";

const WORDS: WordResult[] = [
  {
    text: "think",
    phonemes: [
      { symbol: "θ", tone: "off", heard: "s" },
      { symbol: "ɪ", tone: "good" },
      { symbol: "ŋ", tone: "close" },
      { symbol: "k", tone: "good" },
    ],
  },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("PhonemeChips", () => {
  it("says the feedback is experimental", () => {
    renderApp(<PhonemeChips words={WORDS} />);
    expect(screen.getByText("Pronunciation feedback is experimental")).toBeInTheDocument();
  });

  it("gives every sound a text label next to the colour", () => {
    renderApp(<PhonemeChips words={WORDS} />);
    const chips = within(screen.getByRole("list", { name: "Word: think" })).getAllByRole("listitem");
    expect(chips).toHaveLength(4);
    expect(chips[0]).toHaveTextContent("Needs another try");
    expect(chips[1]).toHaveTextContent("Sounds right");
    expect(chips[2]).toHaveTextContent("Close");
  });

  it("names the sound that was heard instead", () => {
    renderApp(<PhonemeChips words={WORDS} />);
    expect(screen.getByText("Heard: s", { exact: false })).toBeInTheDocument();
  });

  it("draws IPA symbols in the phonetic font class", () => {
    renderApp(<PhonemeChips words={WORDS} />);
    expect(screen.getByText("θ")).toHaveClass("phonetic");
  });
});
