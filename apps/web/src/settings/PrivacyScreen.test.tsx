import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type ApiClient } from "@/api/client";
import { DIAGNOSTICS, SESSIONS, SNAPSHOT, connect, fakeApi, progressWith, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { PrivacyScreen } from "./PrivacyScreen";
import { saveTextFile } from "./download";

vi.mock("./download", () => ({ saveTextFile: vi.fn() }));

function routes(overrides: Partial<ApiClient> = {}): Partial<ApiClient> {
  return {
    getDiagnostics: () => Promise.resolve(DIAGNOSTICS),
    getProgress: () => Promise.resolve(progressWith(SESSIONS)),
    ...overrides,
  };
}

async function open(overrides: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, routes(overrides));
  renderScreen(<PrivacyScreen />, api);
  await connect(api, { ...SNAPSHOT, unavailable: ["speech", "models"] });
  return api;
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
  vi.mocked(saveTextFile).mockClear();
});

describe("PrivacyScreen: statements and locations", () => {
  it("states what leaves the computer and what does not, in the allowed wording", async () => {
    await open();
    expect(await screen.findByText(/Your voice stays on your device\./)).toBeInTheDocument();
    expect(screen.getByText(/Text is sent to the AI provider you choose\./)).toBeInTheDocument();
    expect(screen.getByText(/Your API key is saved in plain text in a file on this computer/)).toBeInTheDocument();
    expect(screen.getByText(/never leave this computer/)).toBeInTheDocument();
  });

  it("shows the data locations from the diagnostics, and says when there is no log folder", async () => {
    await open();
    expect(await screen.findByText(DIAGNOSTICS.data_dir)).toBeInTheDocument();
    expect(screen.getByText(DIAGNOSTICS.curriculum_dir)).toBeInTheDocument();
    expect(screen.getByText("127.0.0.1:8765")).toBeInTheDocument();
    expect(screen.getByText(/The program writes its log to its console window only/)).toBeInTheDocument();
  });

  it("shows a log folder when there is one", async () => {
    await open({ getDiagnostics: () => Promise.resolve({ ...DIAGNOSTICS, log_folder: "C:/Lumingo/logs" }) });
    expect(await screen.findByText("C:/Lumingo/logs")).toBeInTheDocument();
  });

  it("shows an error with a retry when the diagnostics cannot be read, and keeps the statements", async () => {
    const getDiagnostics = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValueOnce(DIAGNOSTICS);
    await open({ getDiagnostics });
    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    expect(screen.getByText(/Your voice stays on your device\./)).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText(DIAGNOSTICS.data_dir)).toBeInTheDocument();
  });

  it("says the payload inspector is not available instead of showing a control", async () => {
    await open();
    const notes = await screen.findAllByRole("note");
    expect(notes.some((note) => /payload inspector is not available/.test(note.textContent ?? ""))).toBe(true);
  });
});

describe("PrivacyScreen: export", () => {
  it("hands the file to the browser and says so", async () => {
    const user = userEvent.setup();
    const exportData = vi.fn().mockResolvedValue({ filename: "lumingo-export-2026-10-06.json", text: '{"data":{}}' });
    await open({ exportData });
    await user.click(await screen.findByRole("button", { name: "Download my data" }));
    await waitFor(() => expect(saveTextFile).toHaveBeenCalledWith("lumingo-export-2026-10-06.json", '{"data":{}}'));
    expect(await screen.findByRole("status")).toHaveTextContent("lumingo-export-2026-10-06.json");
    expect(screen.getByText(/never contains an API key/)).toBeInTheDocument();
  });

  it("shows the error and offers another try when the export fails", async () => {
    const user = userEvent.setup();
    const exportData = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(500, "storage", "the database could not complete the request"))
      .mockResolvedValueOnce({ filename: "lumingo-export.json", text: "{}" });
    await open({ exportData });
    await user.click(await screen.findByRole("button", { name: "Download my data" }));
    expect(await screen.findByText(/could not read or write its data/)).toBeInTheDocument();
    expect(saveTextFile).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(saveTextFile).toHaveBeenCalledTimes(1));
  });
});

describe("PrivacyScreen: delete one session", () => {
  it("shows the empty state", async () => {
    await open({ getProgress: () => Promise.resolve(progressWith([])) });
    expect(await screen.findByText("There are no sessions yet.")).toBeInTheDocument();
  });

  it("shows loading and an error with a retry", async () => {
    const getProgress = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValueOnce(progressWith(SESSIONS));
    await open({ getProgress });
    const retry = await screen.findAllByRole("button", { name: "Try again" });
    await userEvent.setup().click(retry[0] as HTMLElement);
    expect(await screen.findByText(/Session 7: conversation/)).toBeInTheDocument();
  });

  it("deletes only after the session number is typed", async () => {
    const user = userEvent.setup();
    const getProgress = vi
      .fn()
      .mockResolvedValueOnce(progressWith(SESSIONS))
      .mockResolvedValue(progressWith(SESSIONS.slice(1)));
    const deleteSession = vi.fn().mockResolvedValue({ audio_files_removed: 2, compacted: true });
    await open({ getProgress, deleteSession });

    expect(await screen.findByText(/Session 7: conversation/)).toBeInTheDocument();
    expect(screen.getByText(/unit a1-u01/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Delete session 7" }));
    expect(deleteSession).not.toHaveBeenCalled();

    const confirm = screen.getByRole("group", { name: /Type the session number 7/ });
    const confirmButton = screen.getAllByRole("button", { name: "Delete" }).find((button) => confirm.closest("[role=alert]")?.contains(button)) as HTMLElement;
    expect(confirmButton).toBeDisabled();
    const field = within(confirm).getByLabelText("Confirmation");
    expect(field).toHaveFocus();

    await user.type(field, "8");
    expect(confirmButton).toBeDisabled();
    await user.clear(field);
    await user.type(field, "7");
    expect(confirmButton).toBeEnabled();
    await user.click(confirmButton);

    await waitFor(() => expect(deleteSession).toHaveBeenCalledWith(7));
    expect(await screen.findByText("Session 7 was deleted.")).toBeInTheDocument();
    expect(screen.getByText("Recordings removed: 2.")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText(/Session 7: conversation/)).toBeNull());
    expect(screen.getByText(/Session 8: writing/)).toBeInTheDocument();
  });

  it("does nothing when the question is cancelled", async () => {
    const user = userEvent.setup();
    const deleteSession = vi.fn();
    await open({ deleteSession });
    await user.click(await screen.findByRole("button", { name: "Delete session 8" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByLabelText("Confirmation")).toBeNull();
    expect(deleteSession).not.toHaveBeenCalled();
  });

  it("explains a refusal, such as a session that is still running", async () => {
    const user = userEvent.setup();
    const deleteSession = vi.fn().mockRejectedValue(new ApiError(409, "conflict", "the session is still running"));
    await open({ deleteSession });
    await user.click(await screen.findByRole("button", { name: "Delete session 7" }));
    await user.type(screen.getByLabelText("Confirmation"), "7");
    await user.click(screen.getAllByRole("button", { name: "Delete" }).find((button) => !(button as HTMLButtonElement).disabled) as HTMLElement);
    expect(await screen.findByText(/the session is still running/)).toBeInTheDocument();
    expect(screen.getByText(/Session 7: conversation/)).toBeInTheDocument();
  });

  it("warns when the text is gone but the database file could not be rewritten", async () => {
    const user = userEvent.setup();
    const deleteSession = vi.fn().mockResolvedValue({ audio_files_removed: 0, compacted: false });
    await open({ deleteSession });
    await user.click(await screen.findByRole("button", { name: "Delete session 7" }));
    await user.type(screen.getByLabelText("Confirmation"), "7");
    await user.click(screen.getAllByRole("button", { name: "Delete" }).find((button) => !(button as HTMLButtonElement).disabled) as HTMLElement);
    expect(await screen.findByText(/may still sit inside the database file/)).toBeInTheDocument();
  });
});

describe("PrivacyScreen: delete everything", () => {
  async function ask(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByRole("button", { name: "Delete everything" }));
  }

  it("states what goes and what stays, and does not delete at the first click", async () => {
    const user = userEvent.setup();
    const deleteAllData = vi.fn();
    await open({ deleteAllData });
    await ask(user);
    expect(screen.getAllByText(/Provider profiles, settings and the lesson index stay/).length).toBeGreaterThan(0);
    expect(deleteAllData).not.toHaveBeenCalled();
  });

  it("deletes only after the phrase is typed, ignoring case and extra spaces", async () => {
    const user = userEvent.setup();
    const deleteAllData = vi.fn().mockResolvedValue({ audio_files_removed: 3, compacted: true });
    await open({ deleteAllData });
    await ask(user);
    const field = screen.getByLabelText("Confirmation");
    const confirmButton = screen.getAllByRole("button", { name: "Delete" })[0] as HTMLElement;
    expect(confirmButton).toBeDisabled();
    await user.type(field, "delete every");
    expect(confirmButton).toBeDisabled();
    await user.clear(field);
    await user.type(field, "  Delete   EVERYTHING ");
    expect(confirmButton).toBeEnabled();
    await user.click(confirmButton);
    await waitFor(() => expect(deleteAllData).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Everything was deleted.")).toBeInTheDocument();
    expect(screen.getByText("Recordings removed: 3.")).toBeInTheDocument();
  });

  it("asks for the phrase in the learner's language", async () => {
    const user = userEvent.setup();
    setSystem({ language: "id-ID" });
    const deleteAllData = vi.fn().mockResolvedValue({ audio_files_removed: 0, compacted: true });
    await open({ deleteAllData });
    await user.click(await screen.findByRole("button", { name: "Hapus semuanya" }));
    expect(screen.getByText("Ketik hapus semuanya untuk mengonfirmasi.")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Konfirmasi"), "delete everything");
    expect(screen.getAllByRole("button", { name: "Hapus" })[0]).toBeDisabled();
    await user.clear(screen.getByLabelText("Konfirmasi"));
    await user.type(screen.getByLabelText("Konfirmasi"), "hapus semuanya");
    await user.click(screen.getAllByRole("button", { name: "Hapus" })[0] as HTMLElement);
    await waitFor(() => expect(deleteAllData).toHaveBeenCalled());
  });

  it("can be cancelled, and shows a failure with a way to try again", async () => {
    const user = userEvent.setup();
    const deleteAllData = vi.fn().mockRejectedValue(new ApiError(409, "conflict", "a session is running"));
    await open({ deleteAllData });
    await ask(user);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: "Delete everything" })).toBeInTheDocument();

    await ask(user);
    await user.type(screen.getByLabelText("Confirmation"), "delete everything");
    await user.click(screen.getAllByRole("button", { name: "Delete" })[0] as HTMLElement);
    expect(await screen.findByText(/a session is running/)).toBeInTheDocument();
    expect(screen.getByLabelText("Confirmation")).toBeInTheDocument();
  });
});
