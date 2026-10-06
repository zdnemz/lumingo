import { cleanup, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiClient } from "@/api/client";
import { dictionaries } from "@/i18n";
import { PREFS_KEY, type Language } from "@/state/preferences";
import {
  DIAGNOSTICS,
  ENV_PROVIDER,
  FILE_PROVIDER,
  PROBE_OK,
  SESSIONS,
  SNAPSHOT,
  connect,
  fakeApi,
  probeFailure,
  progressWith,
  providerList,
  renderScreen,
  resetBrowserState,
  setSystem,
  visibleStrings,
} from "@/test/utils";
import { DiagnosticsScreen } from "./DiagnosticsScreen";
import { GeneralSettings } from "./GeneralSettings";
import { PrivacyScreen } from "./PrivacyScreen";

vi.mock("./download", () => ({ saveTextFile: vi.fn() }));

const routes = (): Partial<ApiClient> => ({
  listProviders: () => Promise.resolve(providerList([ENV_PROVIDER, { ...FILE_PROVIDER, is_active: true }])),
  getSettings: () =>
    Promise.resolve({
      display_name: "Sari",
      ui_language: "en",
      l1: "id",
      l1_help_mode: "auto",
      adaptive_timing: "auto",
      keep_recordings: false,
    }),
  testProvider: vi.fn().mockResolvedValueOnce(PROBE_OK).mockResolvedValue(probeFailure("auth")),
  getDiagnostics: () => Promise.resolve(DIAGNOSTICS),
  getProgress: () => Promise.resolve(progressWith(SESSIONS)),
});

/** A value that is identical in both dictionaries is a name or a symbol, not a leak. */
function onlyIn(language: Language): Set<string> {
  const other: Language = language === "en" ? "id" : "en";
  const theirs = new Set<string>(Object.values(dictionaries[other]));
  return new Set(Object.values<string>(dictionaries[language]).filter((value) => /\p{L}{3}/u.test(value) && !theirs.has(value) && !value.includes("{")));
}

async function show(language: Language, ui: ReactElement, interact: () => Promise<void>): Promise<string[]> {
  window.localStorage.setItem(PREFS_KEY, JSON.stringify({ language }));
  const api = fakeApi(undefined, routes());
  renderScreen(ui, api);
  await connect(api, { ...SNAPSHOT, unavailable: ["speech", "models"] });
  await interact();
  const strings = visibleStrings(document.body);
  cleanup();
  return strings;
}

const LABELS = {
  en: { test: "Test the connection", add: "Add a profile", del: "Delete everything", delSession: "Delete session 7" },
  id: { test: "Uji sambungan", add: "Tambah profil", del: "Hapus semuanya", delSession: "Hapus sesi 7" },
} as const;

const SCREENS: { name: string; ui: () => ReactElement; interact: (language: Language) => Promise<void> }[] = [
  {
    name: "general settings, with a test result, a failed test and the add form",
    ui: () => <GeneralSettings />,
    interact: async (language) => {
      const user = userEvent.setup();
      const tests = await screen.findAllByRole("button", { name: LABELS[language].test });
      await user.click(tests[0] as HTMLElement);
      await screen.findAllByRole("status");
      await user.click((await screen.findAllByRole("button", { name: LABELS[language].test }))[1] as HTMLElement);
      await screen.findAllByRole("alert");
      await user.click(screen.getByRole("button", { name: LABELS[language].add }));
    },
  },
  {
    name: "privacy and data, with both delete questions open",
    ui: () => <PrivacyScreen />,
    interact: async (language) => {
      const user = userEvent.setup();
      await user.click(await screen.findByRole("button", { name: LABELS[language].delSession }));
      await user.click(screen.getByRole("button", { name: LABELS[language].del }));
    },
  },
  { name: "diagnostics", ui: () => <DiagnosticsScreen />, interact: async () => void (await screen.findByText("abc123")) },
];

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

describe("language switch on the settings, privacy and diagnostics screens", () => {
  it("has English-only and Indonesian-only strings to check against", () => {
    expect(onlyIn("en").size).toBeGreaterThan(300);
    expect(onlyIn("id").size).toBeGreaterThan(300);
  });

  it.each(SCREENS)("$name: no English text in Indonesian and no Indonesian text in English", async ({ ui, interact }) => {
    const english = await show("en", ui(), () => interact("en"));
    const indonesian = await show("id", ui(), () => interact("id"));
    expect(english.length).toBeGreaterThan(10);
    expect(indonesian.length).toBeGreaterThan(10);

    const englishOnly = onlyIn("en");
    const indonesianOnly = onlyIn("id");
    for (const text of indonesian) expect(englishOnly.has(text), `Indonesian screen shows English: "${text}"`).toBe(false);
    for (const text of english) expect(indonesianOnly.has(text), `English screen shows Indonesian: "${text}"`).toBe(false);
    // The two renderings differ in most of their text, so one is not a copy of the other.
    const same = english.filter((text) => indonesian.includes(text));
    expect(same.length).toBeLessThan(english.length / 2);
  });
});
