import { screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { Hud } from "./Hud";

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("Hud", () => {
  it("announces streak, sparks and rank in words", () => {
    renderApp(<Hud streak={5} sparks={240} rank={2} rankProgress={40} restTokens={1} />);
    expect(screen.getByText("Day streak: 5")).toBeInTheDocument();
    expect(screen.getByText("Sparks: 240")).toBeInTheDocument();
    expect(screen.getByText("Glow rank 2, Flicker")).toBeInTheDocument();
    expect(screen.getByText("Rest days: 1")).toBeInTheDocument();
  });

  it("hides the rest-day line when there are none, and never shows a negative streak state", () => {
    renderApp(<Hud streak={0} sparks={0} rank={1} rankProgress={0} restTokens={0} />);
    expect(screen.queryByText(/Rest days/)).toBeNull();
    expect(screen.getByText("Day streak: 0")).toBeInTheDocument();
  });

  it("keeps rewards apart from level estimates", () => {
    renderApp(<Hud streak={5} sparks={240} rank={2} rankProgress={40} restTokens={0} />);
    expect(screen.getByTitle("Sparks and streaks are only for fun. They are not a level estimate.")).toBeInTheDocument();
  });

  it("clamps a rank outside 1 to 6 to a known name", () => {
    renderApp(<Hud streak={1} sparks={1} rank={99} rankProgress={0} restTokens={0} />);
    expect(screen.getByText("Glow rank 99, Beacon")).toBeInTheDocument();
  });
});
