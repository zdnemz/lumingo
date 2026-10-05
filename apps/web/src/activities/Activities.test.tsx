import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { GapFillActivity, gapCount } from "./GapFillActivity";
import { ListenButton } from "./ListenButton";
import { MatchActivity } from "./MatchActivity";
import { McqActivity } from "./McqActivity";
import { ReorderActivity } from "./ReorderActivity";
import { TextAnswer, countWords } from "./TextAnswer";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("McqActivity", () => {
  const options = ["name", "age", "city"];

  it("is a radio group where one answer can be chosen", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    renderApp(<McqActivity stem="What's your ___?" options={options} selected={null} onSelect={onSelect} />);
    expect(await screen.findAllByRole("radio")).toHaveLength(3);
    await user.click(screen.getByRole("radio", { name: "age" }));
    expect(onSelect).toHaveBeenCalledWith(1);
  });

  it("shows a passage before the question when there is one", async () => {
    renderApp(<McqActivity stem="Where is Dewi?" passage="Dewi is at school." options={options} selected={null} onSelect={vi.fn()} />);
    expect(await screen.findByText("Dewi is at school.")).toBeInTheDocument();
  });

  it("marks the right option and the wrong choice after checking, and locks the choices", async () => {
    renderApp(<McqActivity stem="Q" options={options} selected={1} correctIndex={0} locked onSelect={vi.fn()} />);
    const radios = await screen.findAllByRole("radio");
    for (const radio of radios) expect(radio).toBeDisabled();
    const states = Array.from(document.querySelectorAll(".mcq__option")).map((li) => li.getAttribute("data-state"));
    expect(states).toEqual(["right", "wrong", null]);
  });
});

describe("GapFillActivity", () => {
  it("counts gaps and gives each its own labelled box", async () => {
    expect(gapCount("I ___ a student and she ___ a teacher.")).toBe(2);
    expect(gapCount("No gaps here.")).toBe(0);
    renderApp(<GapFillActivity text="I ___ a student and she ___ a teacher." answers={["am", ""]} onChange={vi.fn()} />);
    expect(await screen.findByRole("textbox", { name: "Gap 1" })).toHaveValue("am");
    expect(screen.getByRole("textbox", { name: "Gap 2" })).toHaveValue("");
  });

  it("reports which gap was edited", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    renderApp(<GapFillActivity text="a ___ b ___ c" answers={["", ""]} onChange={onChange} />);
    await user.type(await screen.findByRole("textbox", { name: "Gap 2" }), "x");
    expect(onChange).toHaveBeenCalledWith(1, "x");
  });
});

function ReorderHarness({ tokens }: { tokens: string[] }) {
  const [placed, setPlaced] = useState<number[]>([]);
  return (
    <ReorderActivity
      tokens={tokens}
      placed={placed}
      onPlace={(i) => setPlaced((p) => [...p, i])}
      onRemove={(position) => setPlaced((p) => p.filter((_, k) => k !== position))}
      onReset={() => setPlaced([])}
    />
  );
}

describe("ReorderActivity", () => {
  it("builds a sentence in the order the words are chosen and disables a word once used", async () => {
    const user = userEvent.setup();
    renderApp(<ReorderHarness tokens={["name", "My", "Dewi", "is"]} />);
    await user.click(await screen.findByRole("button", { name: "Add: My" }));
    await user.click(screen.getByRole("button", { name: "Add: name" }));
    expect(screen.getByRole("button", { name: "Add: My" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Remove: My" })).toBeInTheDocument();
    const section = screen.getByRole("region", { name: "Your sentence" });
    expect(within(section).getAllByRole("button").map((b) => b.textContent)).toEqual(["My", "name"]);
  });

  it("removes a word from the sentence and frees it, and starts again", async () => {
    const user = userEvent.setup();
    renderApp(<ReorderHarness tokens={["a", "b"]} />);
    await user.click(await screen.findByRole("button", { name: "Add: a" }));
    await user.click(screen.getByRole("button", { name: "Remove: a" }));
    expect(screen.getByRole("button", { name: "Add: a" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Add: b" }));
    await user.click(screen.getByRole("button", { name: "Start again" }));
    expect(screen.getByText("Tap the words in the right order")).toBeInTheDocument();
  });

  it("copes with the same word twice because tokens are told apart by position", async () => {
    const user = userEvent.setup();
    renderApp(<ReorderHarness tokens={["the", "cat", "the"]} />);
    const adds = await screen.findAllByRole("button", { name: "Add: the" });
    await user.click(adds[0] as HTMLElement);
    expect(screen.getAllByRole("button", { name: "Add: the" })[1]).toBeEnabled();
    expect(screen.getAllByRole("button", { name: "Add: the" })[0]).toBeDisabled();
  });
});

describe("MatchActivity", () => {
  it("lets each item choose its match from a list, including none", async () => {
    const onChoose = vi.fn();
    const user = userEvent.setup();
    renderApp(<MatchActivity left={["hello", "goodbye"]} right={["selamat tinggal", "halo"]} chosen={[null, null]} onChoose={onChoose} />);
    const first = await screen.findByLabelText("Match for: hello");
    await user.selectOptions(first, "halo");
    expect(onChoose).toHaveBeenCalledWith(0, 1);
    await user.selectOptions(first, "");
    expect(onChoose).toHaveBeenLastCalledWith(0, null);
  });

  it("shows the current choice", async () => {
    renderApp(<MatchActivity left={["hello"]} right={["a", "b"]} chosen={[1]} onChoose={vi.fn()} />);
    expect(await screen.findByLabelText("Match for: hello")).toHaveValue("1");
  });
});

describe("TextAnswer", () => {
  it("counts words with any amount of spacing", () => {
    expect(countWords("")).toBe(0);
    expect(countWords("   ")).toBe(0);
    expect(countWords("one  two\nthree")).toBe(3);
  });

  it("shows the target range for a writing task", async () => {
    renderApp(<TextAnswer value="My name is Dewi" onChange={vi.fn()} rows={4} words={{ min: 8, max: 40 }} />);
    expect(await screen.findByText("4 words. Aim for 8 to 40.")).toBeInTheDocument();
    expect(screen.getByLabelText("Your answer")).toHaveAccessibleDescription("4 words. Aim for 8 to 40.");
  });

  it("is a single line for a short answer and reports edits", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    renderApp(<TextAnswer value="" onChange={onChange} />);
    const box = await screen.findByLabelText("Your answer");
    expect(box.tagName).toBe("INPUT");
    await user.type(box, "a");
    expect(onChange).toHaveBeenCalledWith("a");
  });
});

describe("ListenButton", () => {
  it("shows the plays left and stops when none remain", async () => {
    const onListen = vi.fn();
    const user = userEvent.setup();
    const { rerender } = renderApp(<ListenButton onListen={onListen} playsLeft={2} playing={false} />);
    await user.click(await screen.findByRole("button", { name: "Listen" }));
    expect(onListen).toHaveBeenCalledOnce();
    expect(screen.getByText("Plays left: 2")).toBeInTheDocument();
    rerender(<ListenButton onListen={onListen} playsLeft={0} playing={false} />);
    expect(screen.getByRole("button", { name: "Listen" })).toBeDisabled();
  });

  it("offers unlimited plays without a counter, and disables while playing", async () => {
    renderApp(<ListenButton onListen={vi.fn()} playsLeft={null} playing />);
    expect(await screen.findByRole("button", { name: "Playing" })).toBeDisabled();
    expect(screen.queryByText(/Plays left/)).toBeNull();
  });
});
