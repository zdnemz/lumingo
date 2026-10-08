import { screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { SkillBars, type SkillEstimate } from "./SkillBars";

const ESTIMATES: SkillEstimate[] = [
  { skill: "speaking", status: "estimate", level: "A2", confidence: "medium", scoredTasks: 14, sessions: 5 },
  { skill: "writing", status: "insufficient", scoredTasks: 1, sessions: 1, moreTasksNeeded: 2 },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("SkillBars", () => {
  it("writes an estimate in the form the assessment rules allow", () => {
    renderApp(<SkillBars estimates={ESTIMATES} />);
    expect(
      screen.getByText("Speaking: A2 (estimate, medium confidence), based on 14 scored tasks in 5 sessions"),
    ).toBeInTheDocument();
  });

  it("shows no level at all when the evidence is not enough", () => {
    renderApp(<SkillBars estimates={ESTIMATES} />);
    const sentence = screen.getByText(/Not enough evidence yet for writing/);
    expect(sentence).toHaveTextContent("Not enough evidence yet for writing. 2 more writing tasks will help.");
    expect(sentence.textContent).not.toMatch(/\b[ABC][12]\b/);
    const bar = screen.getByRole("progressbar", { name: "Writing" });
    expect(bar).toHaveAttribute("aria-valuenow", "0");
  });

  it("puts the Estimate label on every row", () => {
    renderApp(<SkillBars estimates={ESTIMATES} />);
    expect(screen.getAllByText("Estimate")).toHaveLength(2);
  });

  it("maps the level to the bar so a higher level is a fuller bar", () => {
    renderApp(<SkillBars estimates={ESTIMATES} />);
    const bar = screen.getByRole("progressbar", { name: "Speaking" });
    expect(Number(bar.getAttribute("aria-valuenow"))).toBeGreaterThan(0);
  });

  it("speaks Indonesian when the language is Indonesian", () => {
    setSystem({ reduced: true, language: "id-ID" });
    renderApp(<SkillBars estimates={ESTIMATES} />);
    expect(
      screen.getByText("Berbicara: A2 (perkiraan, keyakinan sedang), berdasarkan 14 tugas yang dinilai dalam 5 sesi"),
    ).toBeInTheDocument();
  });
});
