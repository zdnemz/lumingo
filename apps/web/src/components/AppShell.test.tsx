import { readdirSync } from "node:fs";
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

function openShell() {
  renderApp(
    <AppShell>
      <p>Screen</p>
    </AppShell>,
    fakeApi(),
  );
  return within(screen.getByRole("navigation", { name: label("nav.main") }));
}

function linksOf(nav: ReturnType<typeof openShell>) {
  return nav.getAllByRole("link");
}

describe("the main menu", () => {
  it("lists every screen, with the free-practice screens between the quest map and progress", () => {
    const nav = openShell();
    const links = linksOf(nav);

    expect(links).toHaveLength(NAV.length);
    expect(NAV.map((item) => item.href)).toEqual([
      "/",
      "/unit/",
      "/talk/",
      "/write/",
      "/read/",
      "/progress/",
      "/settings/",
    ]);

    // Every entry leads to its screen and is announced with the label the learner reads.
    NAV.forEach((item, index) => {
      const link = links[index];
      expect(link, `${item.href} has no link`).toBeDefined();
      expect(routeOf(link?.getAttribute("href") ?? null), `${item.href} points elsewhere`).toBe(routeOf(item.href));
      expect(link).toHaveAccessibleName(label(item.label));
    });

    // The three screens of this wave, by name, so a rename cannot pass unnoticed.
    for (const [href, key] of [
      ["/talk/", "nav.talk"],
      ["/write/", "nav.write"],
      ["/read/", "nav.read"],
    ] as const) {
      expect(routeOf(nav.getByRole("link", { name: label(key) }).getAttribute("href"))).toBe(routeOf(href));
    }
  });

  it("marks the screen the learner is on, and keeps a section marked on its sub-pages", () => {
    current.pathname = "/write/";
    const nav = openShell();
    expect(nav.getByRole("link", { name: label("nav.write") })).toHaveAttribute("aria-current", "page");
    expect(nav.getByRole("link", { name: label("nav.talk") })).not.toHaveAttribute("aria-current");
    expect(nav.getByRole("link", { name: label("nav.home") })).not.toHaveAttribute("aria-current");
  });
});

describe("the banners", () => {
  it("renders with no snapshot yet, and shows no engine banner before there is one", () => {
    // `openShell` uses `fakeApi()` and never emits a snapshot: the shell must
    // render, and the engine banner must stay away rather than read a state
    // that does not exist.
    openShell();
    expect(screen.getByRole("main")).toHaveTextContent("Screen");
    expect(screen.queryByText(label("engine.fault.title"))).toBeNull();
    expect(screen.queryByRole("link", { name: label("engine.fault.action") })).toBeNull();
  });
});

// Vitest runs from apps/web, as the other tree walks in this repo assume.
const APP_DIR = join(process.cwd(), "src", "app");

/**
 * Every route that has a page, spelled as the URL the app serves, trailing
 * slash included: `src/app/talk/page.tsx` is `/talk/`, `src/app/page.tsx` is `/`.
 * The walk reads the tree, so a route added in a folder this test never heard
 * of still shows up here.
 */
function pageRoutes(dir = APP_DIR, prefix = ""): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isDirectory()) return pageRoutes(join(dir, entry.name), `${prefix}${entry.name}/`);
    return entry.name === "page.tsx" ? [`/${prefix}`] : [];
  });
}

/**
 * A page belongs to the menu when an entry is that page or sits above it, which
 * is the same reading `isCurrent` uses: `/settings/` keeps Settings marked on
 * `/settings/privacy/`, so a sub-page of a section needs no entry of its own.
 */
function coveredBy(route: string, href: string): boolean {
  const clean = (value: string) => (value.length > 1 ? value.replace(/\/$/, "") : value);
  const path = clean(route);
  const base = clean(href);
  return base === "/" ? path === "/" : path === base || path.startsWith(`${base}/`);
}

/**
 * The two screens that are reached without the menu, and the only ones allowed
 * to be missing from it:
 *
 * - `/onboarding/` is the first-run setup. `FirstRunGate` sends a learner who
 *   has never set up a provider straight there, and the no-provider banner
 *   links to it. A menu entry would only invite someone who is already set up
 *   into a wizard they do not need.
 * - `/styleguide/` is the developer's catalogue of components and tokens. It is
 *   never linked from the app, as the note in `i18n/hardcoded.test.ts` also says.
 *
 * The test checks below that both of these still exist, so the list cannot rot
 * into a hiding place for a screen that simply forgot its entry.
 */
const REACHED_ANOTHER_WAY = ["/onboarding/", "/styleguide/"] as const;

describe("routes and the main menu", () => {
  const routes = pageRoutes();

  it("reads the pages of the app", () => {
    // A walk that finds nothing would make the check below pass for the wrong reason.
    expect(routes).toContain("/");
    for (const href of NAV.map((item) => item.href)) expect(routes).toContain(href);
    for (const route of REACHED_ANOTHER_WAY) expect(routes).toContain(route);
  });

  it("has a menu entry for every page, so a new screen cannot be unreachable", () => {
    const missing = routes.filter(
      (route) => !NAV.some((item) => coveredBy(route, item.href)) && !REACHED_ANOTHER_WAY.some((allowed) => allowed === route),
    );
    expect(missing, "these pages have no entry in the main menu").toEqual([]);
  });

  it("links only to pages that exist, so the menu has no dead ends", () => {
    const dead = NAV.filter((item) => !routes.some((route) => route === item.href)).map((item) => item.href);
    expect(dead, "these menu entries point at no page").toEqual([]);
  });
});
