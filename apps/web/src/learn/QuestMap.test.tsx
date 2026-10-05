import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { QuestMap, unitHref, type QuestRegion } from "./QuestMap";

const REGIONS: QuestRegion[] = [
  {
    level: "A1",
    nodes: [
      { id: "a1-u01", number: 1, title: "Hello and goodbye", state: "done", skillsPractised: 4 },
      { id: "a1-u02", number: 2, title: "My family", state: "current", skillsPractised: 1 },
      { id: "a1-u03", number: 3, title: "Food I like", state: "open", skillsPractised: 0 },
      { id: "a1-u04", number: 4, title: "My day", state: "locked", skillsPractised: 0 },
    ],
  },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ reduced: true });
});

describe("unitHref", () => {
  it("passes the id in the query string and escapes it", () => {
    expect(unitHref("a1-u01")).toBe("/unit/?id=a1-u01");
    expect(unitHref("a b&c")).toBe("/unit/?id=a%20b%26c");
  });
});

describe("QuestMap", () => {
  it("lists every unit of a region in order", () => {
    renderApp(<QuestMap regions={REGIONS} />);
    const region = screen.getByRole("region", { name: "Quest map" });
    expect(within(region).getByRole("heading", { name: "Region A1" })).toBeInTheDocument();
    const items = within(region).getAllByRole("listitem");
    expect(items).toHaveLength(4);
  });

  it("links open, current and done units, with ids in the query string", () => {
    renderApp(<QuestMap regions={REGIONS} />);
    // next/link adds or drops the trailing slash from the build setting, so accept both.
    const hrefOf = (name: string) => screen.getByRole("link", { name }).getAttribute("href");
    expect(hrefOf("Unit 1: Hello and goodbye")).toMatch(/^\/unit\/?\?id=a1-u01$/);
    expect(hrefOf("Unit 3: Food I like")).toMatch(/^\/unit\/?\?id=a1-u03$/);
    const current = screen.getByRole("link", { name: "Unit 2: My family" });
    expect(current).toHaveAttribute("aria-current", "step");
  });

  it("shows a locked unit as plain text, so keyboard users never reach a dead end", () => {
    renderApp(<QuestMap regions={REGIONS} />);
    expect(screen.queryByRole("link", { name: "Unit 4: My day" })).toBeNull();
    const locked = screen.getByLabelText("Unit 4: My day");
    expect(locked).toHaveAttribute("aria-disabled", "true");
    expect(locked).toHaveTextContent("Locked until the unit before it is done");
  });

  it("says in text how many skills are practised, not only with dots", () => {
    renderApp(<QuestMap regions={REGIONS} />);
    const link = screen.getByRole("link", { name: "Unit 2: My family" });
    expect(link).toHaveTextContent("1 of 4 skills practised");
  });
});
