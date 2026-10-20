import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { translate, type MessageKey, type TranslateParams } from "@/i18n";
import { fakeApi, renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { AboutScreen, notices, type ComponentKind } from "./AboutScreen";

/**
 * The kinds the data file holds, in the order the page shows them. The test
 * reads the file, so it grows with it: a component added to the file has to
 * appear on the page, and one removed has to go.
 */
const KINDS: readonly ComponentKind[] = ["crate", "package", "font", "model", "release"];

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

/** The English string of a key, as the screen shows it. */
function say(key: MessageKey, params?: TranslateParams): string {
  return translate("en", key, params);
}

function render() {
  renderApp(<AboutScreen />, fakeApi());
}

/** The components of one kind, as the data file holds them. */
function componentsOf(kind: ComponentKind) {
  return notices.components.filter((component) => component.kind === kind);
}

/**
 * The group of one kind: the heading the screen gives it and the rows under
 * it. The heading is the third level, so it is told apart from the panel
 * titles above it — the fonts group and the fonts panel share a name.
 */
function groupOf(kind: ComponentKind): HTMLElement {
  const heading = screen.getByRole("heading", { level: 3, name: say(`about.kind.${kind}`) });
  const section = heading.closest("section");
  if (section === null) throw new Error(`the group of ${kind} has no section`);
  return section;
}

describe("AboutScreen", () => {
  it("shows every component of the data file, grouped by kind, each group with its count", () => {
    render();
    for (const kind of KINDS) {
      const items = componentsOf(kind);
      if (items.length === 0) continue;

      const group = groupOf(kind);
      // The count is the number the file holds, not a number written down here.
      expect(within(group).getByText(say("about.components.count", { count: items.length }))).toBeInTheDocument();
      // One row per component, and the row of the column headings.
      expect(within(group).getAllByRole("row")).toHaveLength(items.length + 1);

      const text = group.textContent ?? "";
      for (const item of items) {
        expect(text, `${item.name} is not shown`).toContain(item.name);
        expect(text, `the licence of ${item.name} is not shown`).toContain(item.licence);
        if (item.attribution) expect(text, `the attribution of ${item.name} is not shown`).toContain(item.attribution);
      }
    }
  });

  it("states the licences of the project itself", () => {
    render();
    expect(screen.getByRole("heading", { level: 2, name: say("about.own.title") })).toBeInTheDocument();
    expect(screen.getByText(say("about.own.code"))).toBeInTheDocument();
    expect(screen.getByText(say("about.own.content"))).toBeInTheDocument();
    expect(screen.getAllByText(notices.project.code).length).toBeGreaterThan(0);
    expect(screen.getAllByText(notices.project.content).length).toBeGreaterThan(0);
  });

  it("names the bundled fonts and who wrote them", () => {
    render();
    // The panel title is the second level: the fonts group under the
    // components carries the same name, so it is told apart by its level.
    expect(screen.getByRole("heading", { level: 2, name: say("about.fonts.title") })).toBeInTheDocument();
    expect(screen.getByText(say("about.fonts.note"))).toBeInTheDocument();
    expect(screen.getByText(say("about.components.attribution"))).toBeInTheDocument();
  });

  it("lists the speech models and says plainly that none of them can be downloaded yet", () => {
    render();
    expect(screen.getByRole("heading", { level: 2, name: say("about.models.title") })).toBeInTheDocument();
    const downloadable = notices.models.filter((model) => model.downloadable);
    if (notices.models.length === 0) {
      expect(screen.getByText(say("about.models.empty"))).toBeInTheDocument();
    } else if (downloadable.length === 0) {
      expect(screen.getByText(say("about.models.pending"))).toBeInTheDocument();
      expect(screen.queryByText(say("about.models.downloadable"))).toBeNull();
    } else {
      expect(screen.getAllByText(say("about.models.downloadable"))).toHaveLength(downloadable.length);
    }
    for (const model of notices.models) {
      expect(screen.getByText(model.id)).toBeInTheDocument();
      expect(screen.getAllByText(model.licence).length).toBeGreaterThan(0);
    }
  });

  it("names the files in the release and offers a way back", () => {
    render();
    expect(screen.getByRole("heading", { level: 2, name: say("about.release.title") })).toBeInTheDocument();
    // Next writes the href without the trailing slash, the way the app serves
    // the route with one.
    const back = screen.getByRole("link", { name: say("about.back") });
    expect(back.getAttribute("href")?.replace(/\/$/, "")).toBe("/settings");
  });

  it("names no file the release zip does not hold", () => {
    render();
    // The zip holds five flat members. The page may not send the reader to
    // the register or to the models folder, because neither is in it.
    const text = document.body.textContent ?? "";
    expect(text).not.toContain("docs/");
    expect(text).not.toContain("models/");
    expect(text).not.toContain("LICENSE_REGISTER");
  });

  it("says so when the file holds no third-party components", () => {
    render();
    const thirdParty = notices.components.filter((component) => component.kind !== "release");
    if (thirdParty.length === 0) {
      expect(screen.getByText(say("about.components.empty"))).toBeInTheDocument();
    } else {
      expect(screen.queryByText(say("about.components.empty"))).toBeNull();
    }
  });
});
