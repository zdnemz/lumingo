import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiClient } from "@/api/client";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import type { SaveProviderRequest } from "@/generated/SaveProviderRequest";
import type { Settings } from "@/generated/Settings";
import type { UnitList } from "@/generated/UnitList";
import { KEY_GUIDE } from "@/providers/keyGuide";
import {
  CAPS,
  ENV_PROVIDER,
  FILE_PROVIDER,
  PROBE_OK,
  SNAPSHOT,
  connect,
  fakeApi,
  probeFailure,
  providerList,
  renderScreen,
  resetBrowserState,
  setSystem,
  visibleStrings,
  type FakeApi,
} from "@/test/utils";
import { OnboardingWizard } from "./OnboardingWizard";
import { ONBOARDING_KEY, loadProgress } from "./progress";

const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
vi.mock("next/navigation", () => ({
  useRouter: () => router,
  usePathname: () => "/onboarding/",
}));

const SECRET = "AIza-test-SECRET-key-0123456789";
const SETTINGS: Settings = {
  display_name: "Learner",
  ui_language: "en",
  l1: "id",
  l1_help_mode: "auto",
  adaptive_timing: "auto",
  keep_recordings: false,
};
const UNITS: UnitList = { content_version: "v1", units: [], issues: [] };
const MISSING = ["sessions", "activities", "free_modes", "speech", "models"] as const;

/** A fake program whose profile list follows what the screens save. */
function world(initial: ProviderInfo[], overrides: Partial<ApiClient> = {}) {
  let providers = [...initial];
  const api: FakeApi = fakeApi(undefined, {
    listProviders: () => Promise.resolve(providerList(providers)),
    saveProvider: vi.fn((request: SaveProviderRequest) => {
      const saved: ProviderInfo = {
        ...FILE_PROVIDER,
        name: request.name,
        protocol: request.protocol,
        base_url: request.base_url,
        model: request.model,
        key_last4: request.api_key ? request.api_key.slice(-4) : null,
        has_key: request.api_key !== undefined,
        is_active: true,
      };
      providers = [...providers.filter((p) => p.name !== request.name).map((p) => ({ ...p, is_active: false })), saved];
      return Promise.resolve(saved);
    }),
    testProvider: vi.fn(() => {
      providers = providers.map((p) => (p.is_active ? { ...p, probed_at: "2026-10-06T10:00:00Z", capabilities: CAPS } : p));
      return Promise.resolve(PROBE_OK);
    }),
    getSettings: () => Promise.resolve(SETTINGS),
    updateSettings: vi.fn((settings: Settings) => Promise.resolve(settings)),
    listUnits: () => Promise.resolve(UNITS),
    ...overrides,
  });
  return api;
}

async function open(api: FakeApi, provider: ProviderInfo | null = null) {
  renderScreen(<OnboardingWizard />, api);
  await connect(api, { ...SNAPSHOT, provider, unavailable: [...MISSING] });
}

async function next(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Next" }));
}

function heading(): HTMLElement {
  return screen.getByRole("heading", { level: 1 });
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
  router.push.mockClear();
  router.replace.mockClear();
});

describe("OnboardingWizard", () => {
  it("shows loading, then the welcome", async () => {
    renderScreen(<OnboardingWizard />, fakeApi(undefined, { listProviders: () => new Promise(() => undefined) }));
    expect(await screen.findByRole("status")).toHaveTextContent("Loading");
  });

  it("walks from the welcome to ready with a key typed in the UI", async () => {
    const user = userEvent.setup();
    const api = world([]);
    await open(api);

    expect(await screen.findByRole("heading", { name: "Welcome to Lumingo" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    await next(user);
    expect(heading()).toHaveTextContent("Choose a language");
    await next(user);

    // The plain statements of what leaves the computer, including the plain-text key.
    expect(heading()).toHaveTextContent("What leaves your computer");
    expect(screen.getByText(/Your voice stays on your device\./)).toBeInTheDocument();
    expect(screen.getByText(/Text is sent to the AI provider you choose\./)).toBeInTheDocument();
    expect(screen.getByText(/Your API key is saved in plain text in a file on this computer/)).toBeInTheDocument();
    await next(user);

    // The key guide, with the provider's data-use statement, clearly dated.
    expect(heading()).toHaveTextContent("Get a key");
    expect(screen.getByText(/may be used to improve its products/)).toBeInTheDocument();
    expect(screen.getByText(new RegExp(`Written on ${KEY_GUIDE.statedOn}`))).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open Google AI Studio" })).toHaveAttribute("rel", "noopener noreferrer");
    await user.click(screen.getByRole("button", { name: "Fill in these values" }));
    await next(user);

    // No profile yet: the step cannot be left.
    expect(heading()).toHaveTextContent("Add an AI provider");
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    expect(screen.getByLabelText("Base URL")).toHaveValue(KEY_GUIDE.profile.base_url);
    await user.type(screen.getByLabelText("API key"), SECRET);
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    expect(await screen.findByText("Profile gemini-free saved.")).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain(SECRET);
    expect(api.saveProvider).toHaveBeenCalledWith(expect.objectContaining({ api_key: SECRET, make_active: true }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
    await next(user);

    // The connection test, said plainly; the step opens only after it passed.
    expect(heading()).toHaveTextContent("Test the connection");
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Run the test" }));
    expect(await screen.findByText("The connection works")).toBeInTheDocument();
    expect(screen.getByText("Level 1: native JSON schema, the strongest")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
    await next(user);

    // Hardware, and honest states for what the build does not have.
    expect(heading()).toHaveTextContent("Your computer");
    expect(screen.getByText("16 GB")).toBeInTheDocument();
    expect(screen.getByText(/meets the minimum of 8 GB memory and 4 logical processors/)).toBeInTheDocument();
    expect(screen.getAllByRole("note").length).toBeGreaterThanOrEqual(3);
    expect(screen.getAllByRole("note")[0]).toHaveTextContent("Not available in this build");
    await next(user);

    expect(heading()).toHaveTextContent("You are ready");
    expect(await screen.findByText("No unit files found yet.")).toBeInTheDocument();
    expect(screen.getByText("gemini-free, connection tested")).toBeInTheDocument();
    for (const name of ["Learning sessions", "Practice activities", "Speech model downloads"]) {
      expect(screen.getByText(name)).toBeInTheDocument();
    }
    await user.click(screen.getByRole("button", { name: "Start learning" }));
    expect(router.push).toHaveBeenCalledWith("/");
    expect(loadProgress(window)).toEqual({ step: "ready", done: true });
  });

  it("returns to the stored step after a reload", async () => {
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "test", done: false }));
    await open(world([FILE_PROVIDER]), FILE_PROVIDER);
    expect(await screen.findByRole("heading", { name: "Test the connection" })).toBeInTheDocument();
  });

  it("will not resume past the provider step when no profile exists any more", async () => {
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "ready", done: false }));
    await open(world([]));
    expect(await screen.findByRole("heading", { name: "Add an AI provider" })).toBeInTheDocument();
  });

  it("remembers each step as the learner moves", async () => {
    const user = userEvent.setup();
    await open(world([]));
    await screen.findByRole("heading", { name: "Welcome to Lumingo" });
    await next(user);
    await next(user);
    await waitFor(() => expect(loadProgress(window)).toEqual({ step: "privacy", done: false }));
  });

  it("moves focus to the new step's heading", async () => {
    const user = userEvent.setup();
    await open(world([]));
    await screen.findByRole("heading", { name: "Welcome to Lumingo" });
    await next(user);
    await waitFor(() => expect(heading()).toHaveFocus());
  });

  it("shows the read-only env profile and lets the learner carry on with it", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "provider", done: false }));
    const api = world([{ ...ENV_PROVIDER, is_active: true }]);
    await open(api, ENV_PROVIDER);
    expect(await screen.findByText("A provider was found in your environment")).toBeInTheDocument();
    const card = screen.getByRole("article", { name: "env" });
    expect(card).toHaveTextContent("From your .env file, read-only");
    expect(card).toHaveTextContent("Key saved. It ends with ••••9876.");
    expect(within(card).queryByRole("button")).toBeNull();
    // No form until the learner asks for one.
    expect(screen.queryByLabelText("API key")).toBeNull();
    expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "Add another profile" }));
    expect(screen.getByLabelText("API key")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByLabelText("API key")).toBeNull();
    await next(user);
    expect(heading()).toHaveTextContent("Test the connection");
  });

  it("a wrong key sends the learner back to the key field of the profile", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "test", done: false }));
    const api = world([FILE_PROVIDER], { testProvider: vi.fn().mockResolvedValue(probeFailure("auth")) });
    await open(api, FILE_PROVIDER);
    await user.click(await screen.findByRole("button", { name: "Run the test" }));
    expect(await screen.findByText("The provider did not accept the key")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Enter the key again" }));
    expect(heading()).toHaveTextContent("Add an AI provider");
    await waitFor(() => expect(screen.getByLabelText("API key")).toHaveFocus());
  });

  it("a wrong key in the env profile points to the .env file", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "test", done: false }));
    const env = { ...ENV_PROVIDER, is_active: true };
    const api = world([env], { testProvider: vi.fn().mockResolvedValue(probeFailure("auth")) });
    await open(api, env);
    await user.click(await screen.findByRole("button", { name: "Run the test" }));
    expect(await screen.findByText(/comes from your \.env file\. Change it there/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Enter the key again" })).toBeNull();
  });

  it("an unreachable provider can be tried again, and the test can be skipped", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "test", done: false }));
    const api = world([FILE_PROVIDER], { testProvider: vi.fn().mockResolvedValue(probeFailure("network")) });
    await open(api, FILE_PROVIDER);
    await user.click(await screen.findByRole("button", { name: "Run the test" }));
    expect(await screen.findByText("The provider could not be reached")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Test again", hidden: false })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Skip the test for now" }));
    expect(heading()).toHaveTextContent("Your computer");
  });

  it("knows a test passed earlier, from the program's own record", async () => {
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "test", done: false }));
    const tested = { ...FILE_PROVIDER, probed_at: "2026-10-06T10:00:00Z", capabilities: CAPS };
    await open(world([tested]), tested);
    expect(await screen.findByText("This profile passed a connection test.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  });

  it("shows an error with a retry when the program does not answer", async () => {
    const listProviders = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(providerList([]));
    const api = world([], { listProviders });
    renderScreen(<OnboardingWizard />, api);
    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("heading", { name: "Welcome to Lumingo" })).toBeInTheDocument();
  });

  it("switching the language changes every visible string of every step and saves the choice", async () => {
    const user = userEvent.setup();
    const tested = { ...FILE_PROVIDER, probed_at: "2026-10-06T10:00:00Z", capabilities: CAPS };
    const api = world([tested]);
    await open(api, tested);
    await screen.findByRole("heading", { name: "Welcome to Lumingo" });
    await user.click(screen.getByRole("button", { name: "Next" }));
    await user.click(await screen.findByRole("radio", { name: "Bahasa Indonesia" }));
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, ui_language: "id" }));
    expect(heading()).toHaveTextContent("Pilih bahasa");

    // Technical values and language names are the same in both languages by nature.
    const same = new Set(["Lumingo", "Lumi", "Bahasa Indonesia", "English", "Anthropic messages", "my-gemini", "tutor-model", "Minimum", "Median"]);
    const { en } = await import("@/i18n/en");
    const english = new Set<string>(Object.values(en).filter((value) => /[A-Za-z]{4}/.test(value) && !same.has(value)));

    const seen: string[] = [];
    for (;;) {
      seen.push(heading().textContent ?? "");
      for (const text of visibleStrings(document.body)) {
        expect(english.has(text), `step "${heading().textContent}": "${text}" is still English`).toBe(false);
      }
      const forward = screen.queryByRole("button", { name: "Lanjut" });
      if (forward === null) break;
      await user.click(forward);
    }
    // Language, privacy, key guide, provider, test, hardware, ready.
    expect(seen).toHaveLength(7);
    expect(seen[6]).toBe("Kamu siap");
  });
});
