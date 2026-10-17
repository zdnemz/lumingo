import { readFileSync } from "node:fs";
import { join } from "node:path";
import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { translate, type MessageKey } from "@/i18n";
import { defaultPreferences, savePreferences } from "@/state/preferences";
import { fakeApi, renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { AppShell, NAV } from "./AppShell";

/** The path the shell believes it is on, so one render can stand on any screen. */
const current = vi.hoisted(() => ({ pathname: "/" }));
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
  usePathname: () => current.pathname,
}));

beforeEach(() => {
  resetBrowserState();
  setSystem({});
  current.pathname = "/";
  // The labels a learner reads come from the stored preferences, so pin the
  // language to English, and turn motion off so no blink timer runs here.
  savePreferences(window, { ...defaultPreferences("en"), motion: "off" });
});

/** The English label of a menu entry, as the shell renders it. */
function label(key: MessageKey): string {
  return translate("en", key);
}

/**
 * The route a link points at, without the trailing slash. Next keeps `/` and
 * writes every other href without the final slash, while `NAV` spells the
 * routes the way the app serves them, so the two are compared on this form.
 */
function routeOf(href: string | null): string {
  return href !== null && href.length > 1 ? href.replace(/\/$/, "") : (href ?? "");
}

/** Renders the shell and returns its menu landmark. */
function openShell(): HTMLElement {
  renderApp(
    <AppShell>
      <p>Screen</p>
    </AppShell>,
    fakeApi(),
  );
  return screen.getByRole("navigation", { name: label("nav.main") });
}

/**
 * Everything a Tab press can land on, in document order. `main` carries
 * `tabindex="-1"` so the skip link can focus it without putting it in the tab
 * order; that is the one negative index this walk drops.
 */
const FOCUSABLE = ["a[href]", "button", "input", "select", "textarea", "summary", "[tabindex]"].join(",");

function focusables(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((element) => {
    if (element.hasAttribute("disabled")) return false;
    if (element.hasAttribute("hidden") || element.getAttribute("aria-hidden") === "true") return false;
    const index = element.getAttribute("tabindex");
    return index === null || Number(index) >= 0;
  });
}

/**
 * Read from disk rather than from the DOM: jsdom never loads the stylesheets,
 * so a computed style here would only ever report the browser default and the
 * assertion would pass no matter what `base.css` said. Vitest runs from
 * apps/web, as the tree walk in `AppShell.test.tsx` also assumes.
 */
const BASE_CSS = readFileSync(join(process.cwd(), "src", "styles", "base.css"), "utf8");

/** Every declaration of the one block whose selector list holds `selector` on its own. */
function declarationsOf(selector: string): Map<string, string> {
  const css = BASE_CSS.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const [, selectorList = "", body = ""] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (!selectorList.split(",").some((part) => part.trim() === selector)) continue;
    const found = new Map<string, string>();
    for (const chunk of body.split(";")) {
      const declaration = /^\s*([\w-]+)\s*:\s*(.+?)\s*$/.exec(chunk);
      if (declaration?.[1] !== undefined && declaration[2] !== undefined) found.set(declaration[1], declaration[2]);
    }
    return found;
  }
  throw new Error(`base.css has no rule for ${selector}`);
}

describe("the skip link", () => {
  it("is the first thing the keyboard reaches, and it jumps to the main landmark", () => {
    openShell();
    const skip = screen.getByRole("link", { name: label("nav.skip") });
    expect(skip).toHaveAttribute("href", "#main");
    // A skip link whose target is missing scrolls nowhere and focuses nothing.
    expect(document.getElementById("main"), "nothing on the page answers the skip link").not.toBeNull();

    const order = focusables();
    // Without other focusable nodes "first" would be true of anything at all.
    expect(order.length, "the walk found nothing else focusable, so 'first' proves nothing").toBeGreaterThan(1);
    expect(order[0], `the first focusable node is ${order[0]?.outerHTML ?? "none"}`).toBe(skip);
  });
});

describe("the main landmark", () => {
  it("is the element the skip link names, and it takes focus without joining the tab order", () => {
    openShell();
    const main = screen.getByRole("main");
    expect(document.querySelector("main#main"), "the skip link's #main is not the main landmark").toBe(main);
    expect(main).toHaveAttribute("tabindex", "-1");

    // jsdom reports `tabIndex === -1` for a `<main>` with no attribute at all,
    // so the property that matters is the one the skip link relies on: asking
    // for focus actually lands there. A bare `<main>` refuses it.
    main.focus();
    expect(main, "main refuses focus, so the skip link scrolls nowhere and strands the keyboard").toHaveFocus();

    expect(focusables(), "main is in the tab order, so every screen starts one Tab too late").not.toContain(main);
  });
});

describe("the styles the shell leans on", () => {
  it("draws a visible focus ring for keyboard users", () => {
    const outline = declarationsOf(":focus-visible").get("outline") ?? "";
    expect(outline, ":focus-visible declares no outline at all").not.toBe("");
    expect(outline, ":focus-visible removes the ring").not.toMatch(/^none\b/);
    expect(outline, ":focus-visible draws a zero-width ring").not.toMatch(/^0(?:px|rem|em)?$/);
    expect(outline, ":focus-visible draws a transparent ring").not.toMatch(/transparent/);
    expect(outline, "an outline without a style keyword paints nothing").toMatch(
      /\b(solid|dashed|dotted|double|groove|ridge|inset|outset)\b/,
    );
  });

  it("hides sr-only text from the eye while leaving it to a screen reader", () => {
    const rule = declarationsOf(".sr-only");
    expect(rule.get("position"), "sr-only text that stays in the flow takes up room").toBe("absolute");
    expect(rule.get("width"), "sr-only text must be shrunk to nothing on screen").toBe("1px");
    expect(rule.get("height")).toBe("1px");
    expect(rule.get("overflow"), "a box that does not clip can spill its text back into view").toBe("hidden");
    expect(rule.get("clip-path") ?? rule.get("clip") ?? "", "with no clip the text is still painted").toMatch(/inset\(|rect\(/);
  });
});

describe("the main menu", () => {
  it("is a labelled landmark whose every entry is announced", () => {
    const nav = openShell();
    expect(nav, "the menu has no accessible name, so it is announced as an unnamed navigation").toHaveAttribute(
      "aria-label",
      label("nav.main"),
    );

    const links = within(nav).getAllByRole("link");
    expect(links, "the menu has no entries to check").toHaveLength(NAV.length);
    for (const link of links) {
      // A link named only by its sprite is announced as nothing at all.
      expect(link, `the entry to ${link.getAttribute("href")} has no accessible name`).toHaveAccessibleName();
    }
  });
});

/** The entry the menu must mark, and the route it must be marked on. */
const CURRENT: readonly { route: string; href: string }[] = [
  { route: "/", href: "/" },
  { route: "/write/", href: "/write/" },
  // A sub-page keeps its section marked, so Settings stays current on privacy.
  { route: "/settings/privacy/", href: "/settings/" },
];

describe("the current page", () => {
  for (const { route, href } of CURRENT) {
    it(`marks only the entry for ${href} on ${route}`, () => {
      current.pathname = route;
      const nav = openShell();
      const marked = within(nav)
        .getAllByRole("link")
        .filter((link) => link.getAttribute("aria-current") === "page");

      expect(marked, `on ${route} the menu marks ${marked.length} entries`).toHaveLength(1);
      expect(routeOf(marked[0]?.getAttribute("href") ?? null), `on ${route} the marked entry is not ${href}`).toBe(routeOf(href));
    });
  }
});
