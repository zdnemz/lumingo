import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type ApiClient } from "@/api/client";
import type { Settings } from "@/generated/Settings";
import { FILE_PROVIDER, SNAPSHOT, connect, fakeApi, providerList, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { GeneralSettings } from "./GeneralSettings";
import { matchesPhrase } from "./TypedConfirm";

const SETTINGS: Settings = {
  display_name: "Sari",
  ui_language: "en",
  l1: "id",
  l1_help_mode: "auto",
  adaptive_timing: "auto",
  keep_recordings: false,
};

function routes(overrides: Partial<ApiClient> = {}): Partial<ApiClient> {
  return {
    listProviders: () => Promise.resolve(providerList([FILE_PROVIDER])),
    getSettings: () => Promise.resolve(SETTINGS),
    updateSettings: vi.fn((settings: Settings) => Promise.resolve(settings)),
    ...overrides,
  };
}

async function open(overrides: Partial<ApiClient> = {}, unavailable: typeof SNAPSHOT.unavailable = ["speech", "models"]) {
  const api = fakeApi(undefined, routes(overrides));
  renderScreen(<GeneralSettings />, api);
  await connect(api, { ...SNAPSHOT, unavailable });
  return api;
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

describe("GeneralSettings", () => {
  it("shows the providers, the plain-text key statement, and the display and learning settings", async () => {
    await open();
    expect(await screen.findByRole("article", { name: "my-gemini" })).toBeInTheDocument();
    expect(screen.getByText(/Your API key is saved in plain text in a file on this computer/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open the setup guide again" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Theme" })).toBeInTheDocument();
    expect(await screen.findByLabelText("What the tutor calls you")).toHaveValue("Sari");
  });

  it("says audio devices, the model manager and pronunciation timing are not available, with no fake control", async () => {
    await open();
    const notes = await screen.findAllByRole("note");
    expect(notes).toHaveLength(3);
    for (const note of notes) expect(note).toHaveTextContent("Not available in this build");
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.queryByRole("switch", { name: /recording/i })).toBeNull();
  });

  it("changes the language at once and saves it with the program", async () => {
    const user = userEvent.setup();
    const api = await open();
    await user.click(await screen.findByRole("radio", { name: "Bahasa Indonesia" }));
    expect(await screen.findByRole("heading", { name: "Penyedia AI" })).toBeInTheDocument();
    expect(document.documentElement.lang).toBe("id");
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, ui_language: "id" }));
  });

  it("keeps the new language but says so when the program cannot save it", async () => {
    const user = userEvent.setup();
    await open({ updateSettings: vi.fn().mockRejectedValue(new ApiError(500, "storage", "the database could not complete the request")) });
    await user.click(await screen.findByRole("radio", { name: "Bahasa Indonesia" }));
    expect(await screen.findByText(/Pilihan sudah dipakai di sini, tetapi program tidak bisa menyimpannya/)).toBeInTheDocument();
    expect(document.documentElement.lang).toBe("id");
  });

  it("saves a new name with the whole current set of settings", async () => {
    const user = userEvent.setup();
    const api = await open();
    const field = await screen.findByLabelText("What the tutor calls you");
    await user.clear(field);
    await user.type(field, "Dewi");
    await user.click(screen.getByRole("button", { name: "Save name" }));
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, display_name: "Dewi" }));
    expect(await screen.findByText("Saved.")).toBeInTheDocument();
  });

  it("refuses an empty name before it asks the program", async () => {
    const user = userEvent.setup();
    const api = await open();
    const field = await screen.findByLabelText("What the tutor calls you");
    await user.clear(field);
    await user.click(screen.getByRole("button", { name: "Save name" }));
    expect(screen.getByText("Write a name of 1 to 64 characters.")).toBeInTheDocument();
    expect(api.updateSettings).not.toHaveBeenCalled();
  });

  it("saves Indonesian help as soon as it is chosen", async () => {
    const user = userEvent.setup();
    const api = await open();
    const group = await screen.findByRole("group", { name: "Help in Indonesian" });
    await user.click(within(group).getByRole("radio", { name: "Off" }));
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, l1_help_mode: "off" }));
  });

  it("shows the settings error with a retry", async () => {
    const getSettings = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValue(SETTINGS);
    await open({ getSettings });
    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByLabelText("What the tutor calls you")).toHaveValue("Sari");
  });

  it("shows loading while the settings come", async () => {
    await open({ getSettings: () => new Promise(() => undefined) });
    expect((await screen.findAllByRole("status")).some((status) => status.textContent === "Loading...")).toBe(true);
  });
});

describe("matchesPhrase", () => {
  it("ignores case and extra spaces but never matches empty or partial text", () => {
    expect(matchesPhrase("  Delete   everything ", "delete everything")).toBe(true);
    expect(matchesPhrase("delete", "delete everything")).toBe(false);
    expect(matchesPhrase("", "")).toBe(false);
    expect(matchesPhrase("7", "7")).toBe(true);
    expect(matchesPhrase("70", "7")).toBe(false);
  });
});
