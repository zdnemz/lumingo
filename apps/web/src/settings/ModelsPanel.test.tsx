import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type ApiClient } from "@/api/client";
import type { ModelList } from "@/generated/ModelList";
import type { ModelView } from "@/generated/ModelView";
import type { ServerEvent } from "@/generated/ServerEvent";
import { SNAPSHOT, connect, fakeApi, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { ModelsPanel } from "./ModelsPanel";

/** One manifest entry as the server sends it, with every field spelled out. */
function model(patch: Partial<ModelView> & { id: string }): ModelView {
  return {
    role: "stt",
    engine: "sherpa-onnx",
    version: "candidate",
    size_bytes: 0,
    licence: { license: "MIT", license_url: "https://example.test/MIT", text: null, needs_review: false },
    downloadable: { state: "no", reason: "model stt-whisper-base-en lists no files" },
    installed: null,
    downloading: false,
    notes: null,
    ...patch,
  };
}

function list(...models: ModelView[]): ModelList {
  return { models };
}

function routes(overrides: Partial<ApiClient> = {}): Partial<ApiClient> {
  return { listModels: () => Promise.resolve(list()), ...overrides };
}

async function open(overrides: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, routes(overrides));
  renderScreen(<ModelsPanel />, api);
  await connect(api, SNAPSHOT);
  return api;
}

function progress(patch: Partial<Extract<ServerEvent, { type: "DownloadProgress" }>> & { seq: number }): ServerEvent {
  return {
    type: "DownloadProgress",
    model_id: "stt-whisper-base-en",
    file: "model.onnx",
    file_index: 0,
    file_count: 2,
    bytes_done: 1024,
    bytes_total: 4096,
    state: "running",
    message: null,
    ...patch,
  };
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

describe("ModelsPanel", () => {
  it("says the list is empty when the manifest has no entries", async () => {
    await open();
    expect(await screen.findByText("The model list is empty.")).toBeInTheDocument();
  });

  it("draws one row per entry with its role, engine, version, size and licence link", async () => {
    await open({
      listModels: () =>
        Promise.resolve(
          list(
            model({ id: "vad-silero", role: "vad", engine: "sherpa-onnx", version: "candidate", size_bytes: 0 }),
            model({
              id: "tts-kokoro",
              role: "tts",
              engine: "sherpa-onnx",
              version: "1.0",
              size_bytes: 310_000_000,
              licence: { license: "Apache-2.0", license_url: "https://example.test/apache", text: null, needs_review: false },
            }),
          ),
        ),
    });
    const rows = await screen.findAllByRole("listitem");
    expect(rows).toHaveLength(2);

    const vad = rows[0] as HTMLElement;
    expect(within(vad).getByRole("heading", { name: "vad-silero" })).toBeInTheDocument();
    expect(vad).toHaveTextContent("Voice activity detection");
    expect(vad).toHaveTextContent("sherpa-onnx");
    expect(vad).toHaveTextContent("Version candidate");
    // A size of zero is "None": the manifest has not measured it yet.
    expect(vad).toHaveTextContent("Size None");
    expect(vad).toHaveTextContent("Licence: MIT");
    expect(within(vad).getByRole("link", { name: "Read the licence" })).toHaveAttribute("href", "https://example.test/MIT");

    const tts = rows[1] as HTMLElement;
    expect(tts).toHaveTextContent("Text to speech");
    expect(tts).toHaveTextContent("Version 1.0");
    expect(tts).toHaveTextContent("Size 310 megabytes");
  });

  it("shows the installed record when the program has one, and says so when it has none", async () => {
    await open({
      listModels: () =>
        Promise.resolve(
          list(
            model({
              id: "vad-silero",
              installed: { version: "1.0", files: 3, size_bytes: 2_500_000, combined_sha256: "a".repeat(64) },
            }),
            model({ id: "stt-whisper-base-en" }),
          ),
        ),
    });
    const rows = await screen.findAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("Installed: version 1.0, 3 files, 2.5 megabytes");
    expect(rows[1]).toHaveTextContent("Not installed on this computer yet.");
  });

  it("gives the server's own reason and no download control when the entry cannot be downloaded", async () => {
    await open({
      listModels: () =>
        Promise.resolve(list(model({ id: "vad-silero", downloadable: { state: "no", reason: "model vad-silero lists no files" } }))),
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    expect(row).toHaveTextContent("Why it cannot be downloaded: model vad-silero lists no files");
    expect(within(row).queryByRole("button", { name: "Download this model" })).toBeNull();
    expect(within(row).queryByRole("checkbox")).toBeNull();
  });

  it("asks for the licence to be accepted before the download control works", async () => {
    const user = userEvent.setup();
    const downloadModel = vi.fn().mockResolvedValue({ model_id: "vad-silero", files: 2 });
    await open({
      listModels: () =>
        Promise.resolve(
          list(
            model({
              id: "vad-silero",
              downloadable: { state: "yes" },
              licence: { license: "MIT", license_url: "https://example.test/MIT", text: "MIT text", needs_review: false },
            }),
          ),
        ),
      downloadModel,
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    const control = within(row).getByRole("button", { name: "Download this model" });
    expect(control).toBeDisabled();

    await user.click(within(row).getByRole("checkbox", { name: "I have read and accept this licence" }));
    expect(control).toBeEnabled();
    await user.click(control);
    // The request names the licence the learner was shown, which is what the
    // server checks before it fetches anything.
    expect(downloadModel).toHaveBeenCalledWith("vad-silero", {
      accept_licence: true,
      license: "MIT",
      license_url: "https://example.test/MIT",
    });
  });

  it("explains the server's refusal of a download whose licence was not accepted", async () => {
    const user = userEvent.setup();
    const downloadModel = vi
      .fn()
      .mockRejectedValue(new ApiError(409, "licence_not_accepted", "the licence of this model must be accepted before it is downloaded"));
    await open({
      listModels: () => Promise.resolve(list(model({ id: "vad-silero", downloadable: { state: "yes" } }))),
      downloadModel,
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    await user.click(within(row).getByRole("checkbox", { name: "I have read and accept this licence" }));
    await user.click(within(row).getByRole("button", { name: "Download this model" }));
    expect(await screen.findByText("The licence of this model must be accepted before it is downloaded.")).toBeInTheDocument();
  });

  it("shows the progress from the stream events and cancels through the route", async () => {
    const user = userEvent.setup();
    const cancelModel = vi.fn().mockResolvedValue({ ok: true });
    const api = await open({
      listModels: () => Promise.resolve(list(model({ id: "stt-whisper-base-en", downloadable: { state: "yes" } }))),
      downloadModel: vi.fn().mockResolvedValue({ model_id: "stt-whisper-base-en", files: 2 }),
      cancelModel,
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    await user.click(within(row).getByRole("checkbox", { name: "I have read and accept this licence" }));
    await user.click(within(row).getByRole("button", { name: "Download this model" }));

    // Progress arrives on the event stream, not in the answer to the download.
    const started = await within(row).findByRole("button", { name: "Cancel the download" });
    expect(within(row).queryByRole("button", { name: "Download this model" })).toBeNull();
    api.emit(progress({ seq: 1, file_index: 1, file_count: 3, bytes_done: 2048, bytes_total: 4096 }));
    expect(await within(row).findByRole("status")).toHaveTextContent("Downloading model.onnx, file 2 of 3: 2048 of 4096 bytes");

    await user.click(started);
    expect(cancelModel).toHaveBeenCalledWith("stt-whisper-base-en");
  });

  it("keeps the control busy from the moment the download starts, before any event", async () => {
    const user = userEvent.setup();
    await open({
      listModels: () => Promise.resolve(list(model({ id: "vad-silero", downloadable: { state: "yes" } }))),
      downloadModel: vi.fn().mockResolvedValue({ model_id: "vad-silero", files: 1 }),
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    await user.click(within(row).getByRole("checkbox", { name: "I have read and accept this licence" }));
    await user.click(within(row).getByRole("button", { name: "Download this model" }));
    // The server answered "started"; the row waits for the events from here.
    expect(await within(row).findByRole("button", { name: "Cancel the download" })).toBeInTheDocument();
  });

  it("reads the list again when a download ends, so the installed record is the stored one", async () => {
    const user = userEvent.setup();
    const installed: ModelView = model({
      id: "vad-silero",
      downloadable: { state: "yes" },
      installed: { version: "1.0", files: 2, size_bytes: 1_000_000, combined_sha256: "b".repeat(64) },
    });
    const listModels = vi
      .fn()
      .mockResolvedValueOnce(list(model({ id: "vad-silero", downloadable: { state: "yes" } })))
      .mockResolvedValue(list(installed));
    const api = await open({
      listModels,
      downloadModel: vi.fn().mockResolvedValue({ model_id: "vad-silero", files: 2 }),
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    await user.click(within(row).getByRole("checkbox", { name: "I have read and accept this licence" }));
    await user.click(within(row).getByRole("button", { name: "Download this model" }));
    api.emit(progress({ seq: 1, model_id: "vad-silero", state: "done" }));
    expect(await screen.findByText("Installed: version 1.0, 2 files, 1 megabyte")).toBeInTheDocument();
    expect(listModels).toHaveBeenCalledTimes(2);
  });

  it("shows the failure sentence the stream carried and offers the control again", async () => {
    const user = userEvent.setup();
    const api = await open({
      listModels: () => Promise.resolve(list(model({ id: "vad-silero", downloadable: { state: "yes" } }))),
      downloadModel: vi.fn().mockResolvedValue({ model_id: "vad-silero", files: 1 }),
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    await user.click(within(row).getByRole("checkbox", { name: "I have read and accept this licence" }));
    await user.click(within(row).getByRole("button", { name: "Download this model" }));
    api.emit(progress({ seq: 2, model_id: "vad-silero", state: "failed", message: "the transfer was interrupted; start the download again to resume" }));
    expect(await within(row).findByRole("status")).toHaveTextContent("the transfer was interrupted; start the download again to resume");
    // The download ended, so the licence has to be accepted again.
    const control = await within(row).findByRole("button", { name: "Download this model" });
    expect(control).toBeDisabled();
  });

  it("says so when the list cannot be read, and reads it again on request", async () => {
    const listModels = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(list(model({ id: "vad-silero" })));
    await open({ listModels });
    expect(await screen.findByText("The model list could not be loaded.")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("heading", { name: "vad-silero" })).toBeInTheDocument();
  });

  it("shows loading while the list comes", async () => {
    await open({ listModels: () => new Promise(() => undefined) });
    expect((await screen.findAllByRole("status")).some((status) => status.textContent === "Loading...")).toBe(true);
  });
});
