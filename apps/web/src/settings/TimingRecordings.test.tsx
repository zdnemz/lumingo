import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type ApiClient } from "@/api/client";
import type { Settings } from "@/generated/Settings";
import { SNAPSHOT, connect, fakeApi, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { RecordingsSettings } from "./RecordingsSettings";
import { TimingSettings } from "./TimingSettings";

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
    getSettings: () => Promise.resolve(SETTINGS),
    updateSettings: vi.fn((settings: Settings) => Promise.resolve(settings)),
    ...overrides,
  };
}

async function open(ui: React.ReactElement, overrides: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, routes(overrides));
  renderScreen(ui, api);
  await connect(api, SNAPSHOT);
  return api;
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

describe("TimingSettings", () => {
  it("shows the three timing choices with the stored one picked", async () => {
    await open(<TimingSettings />);
    const group = await screen.findByRole("group", { name: "Pronunciation timing" });
    expect(within(group).getByRole("radio", { name: "Auto", checked: true })).toBeInTheDocument();
    expect(within(group).getByRole("radio", { name: "Always wait", checked: false })).toBeInTheDocument();
    expect(within(group).getByRole("radio", { name: "Never wait", checked: false })).toBeInTheDocument();
  });

  it("says plainly that the choice changes nothing in this build", async () => {
    await open(<TimingSettings />);
    expect(
      await screen.findByText("Speech is not part of this build yet, so the choice changes nothing for now. It is saved and will be used once speech is here."),
    ).toBeInTheDocument();
  });

  it("saves a new choice with the whole current set of settings", async () => {
    const user = userEvent.setup();
    const api = await open(<TimingSettings />);
    await user.click(await screen.findByRole("radio", { name: "Never wait" }));
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, adaptive_timing: "never_wait" }));
    expect(await screen.findByText("Saved.")).toBeInTheDocument();
  });

  it("reads the settings again before it writes, so another panel's change is not undone", async () => {
    const user = userEvent.setup();
    const getSettings = vi.fn().mockResolvedValueOnce(SETTINGS).mockResolvedValue({ ...SETTINGS, keep_recordings: true });
    const api = await open(<TimingSettings />, { getSettings });
    await user.click(await screen.findByRole("radio", { name: "Always wait" }));
    await waitFor(() =>
      expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, keep_recordings: true, adaptive_timing: "always_wait" }),
    );
  });

  it("shows the failure and keeps the stored choice when the program cannot save", async () => {
    const user = userEvent.setup();
    await open(<TimingSettings />, {
      updateSettings: vi.fn().mockRejectedValue(new ApiError(500, "storage", "the database could not complete the request")),
    });
    await user.click(await screen.findByRole("radio", { name: "Always wait" }));
    expect(await screen.findByText(/could not read or write its data/)).toBeInTheDocument();
    // The stored answer did not change, so the control still shows what is saved.
    expect(screen.getByRole("radio", { name: "Auto", checked: true })).toBeInTheDocument();
  });

  it("shows an error with a retry when the settings cannot be read", async () => {
    const getSettings = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValue(SETTINGS);
    await open(<TimingSettings />, { getSettings });
    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("group", { name: "Pronunciation timing" })).toBeInTheDocument();
  });

  it("shows loading while the settings come", async () => {
    await open(<TimingSettings />, { getSettings: () => new Promise(() => undefined) });
    expect((await screen.findAllByRole("status")).some((status) => status.textContent === "Loading...")).toBe(true);
  });
});

describe("RecordingsSettings", () => {
  it("puts the recordings switch next to the raw-audio sentence, off, with the stored folder said", async () => {
    await open(<RecordingsSettings />);
    const toggle = await screen.findByRole("switch", { name: "Recordings" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("Raw audio is not kept unless you switch on recordings in Settings.")).toBeInTheDocument();
    expect(screen.getByText("Stored recordings stay in the data folder on this computer.")).toBeInTheDocument();
  });

  it("says plainly that nothing is recorded in this build", async () => {
    await open(<RecordingsSettings />);
    expect(
      await screen.findByText(
        "Keep recent recordings of your voice so you can replay them. Speech is not part of this build yet, so nothing is recorded for now.",
      ),
    ).toBeInTheDocument();
  });

  it("saves the switch with the whole current set of settings", async () => {
    const user = userEvent.setup();
    const api = await open(<RecordingsSettings />);
    await user.click(await screen.findByRole("switch", { name: "Recordings" }));
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, keep_recordings: true }));
    expect(await screen.findByText("Saved.")).toBeInTheDocument();
  });

  it("reads the settings again before it writes, so another panel's change is not undone", async () => {
    const user = userEvent.setup();
    const getSettings = vi.fn().mockResolvedValueOnce(SETTINGS).mockResolvedValue({ ...SETTINGS, adaptive_timing: "never_wait" });
    const api = await open(<RecordingsSettings />, { getSettings });
    await user.click(await screen.findByRole("switch", { name: "Recordings" }));
    await waitFor(() =>
      expect(api.updateSettings).toHaveBeenCalledWith({ ...SETTINGS, adaptive_timing: "never_wait", keep_recordings: true }),
    );
  });

  it("shows the failure and keeps the stored choice when the program cannot save", async () => {
    const user = userEvent.setup();
    await open(<RecordingsSettings />, {
      updateSettings: vi.fn().mockRejectedValue(new ApiError(500, "storage", "the database could not complete the request")),
    });
    await user.click(await screen.findByRole("switch", { name: "Recordings" }));
    expect(await screen.findByText(/could not read or write its data/)).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Recordings" })).toHaveAttribute("aria-checked", "false");
  });

  it("shows an error with a retry when the settings cannot be read", async () => {
    const getSettings = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValue(SETTINGS);
    await open(<RecordingsSettings />, { getSettings });
    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("switch", { name: "Recordings" })).toBeInTheDocument();
  });

  it("shows loading while the settings come", async () => {
    await open(<RecordingsSettings />, { getSettings: () => new Promise(() => undefined) });
    expect((await screen.findAllByRole("status")).some((status) => status.textContent === "Loading...")).toBe(true);
  });
});
