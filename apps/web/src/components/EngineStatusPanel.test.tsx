import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EngineId } from "@/generated/EngineId";
import type { EngineView } from "@/generated/EngineView";
import { SNAPSHOT, connect, fakeApi, renderApp, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { EngineStatusPanel, EngineStatusSection } from "./EngineStatusPanel";

const DETAIL = "The model file is not in the models folder.";

/** One engine as the program reports it, with every field spelled out. */
function engine(patch: Partial<EngineView> & { id: EngineId }): EngineView {
  return { state: "ready", detail: DETAIL, model: null, ...patch };
}

/** Renders the pure panel around one engine and hands back its row. */
function rowOf(patch: Partial<EngineView> & { id: EngineId }): HTMLElement {
  renderApp(<EngineStatusPanel engines={[engine(patch)]} onRetry={vi.fn()} />);
  return screen.getByRole("listitem");
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

describe("EngineStatusPanel: one fixture per state", () => {
  it("says an engine is not available, and repeats the program's own reason a file is missing", () => {
    const detail = "The microphone model file is not in the models folder.";
    const row = rowOf({ id: "audio_input", state: "unavailable", detail });
    expect(within(row).getByText("Microphone")).toBeInTheDocument();
    expect(row).toHaveTextContent("Not available");
    expect(within(row).getByText(detail)).toBeInTheDocument();
    expect(within(row).queryByText("Ready")).toBeNull();
  });

  it("says an engine is set up but not loaded", () => {
    const row = rowOf({ id: "tts", state: "configured", detail: "The voice is set up and loads on first use." });
    expect(within(row).getByText("Text to speech")).toBeInTheDocument();
    expect(row).toHaveTextContent("Set up, not loaded");
    expect(within(row).getByText("The voice is set up and loads on first use.")).toBeInTheDocument();
  });

  it("says an engine is loading its model", () => {
    const row = rowOf({ id: "vad", state: "loading", detail: "The voice activity model is loading." });
    expect(within(row).getByText("Voice activity detection")).toBeInTheDocument();
    expect(row).toHaveTextContent("Loading the model");
    expect(within(row).getByText("The voice activity model is loading.")).toBeInTheDocument();
  });

  it("says an engine is ready and shows the model the engine itself reported", () => {
    const row = rowOf({
      id: "stt",
      state: "ready",
      detail: "Speech recognition is loaded.",
      model: { id: "sherpa-onnx-whisper-base-en", version: "2.4.1", model_checksum: null },
    });
    expect(within(row).getByText("Speech to text")).toBeInTheDocument();
    expect(row).toHaveTextContent("Ready");
    expect(within(row).getByText("sherpa-onnx-whisper-base-en")).toBeInTheDocument();
    expect(within(row).getByText("Version 2.4.1")).toBeInTheDocument();
  });

  it("says an engine failed and adds no cause of its own to the program's sentence", () => {
    // The "out of memory" case. EngineState has no memory variant, so the row
    // must not name a cause: it says the state and repeats the server's words.
    const detail = "The engine asked for 2.1 GB of memory and the program could not give it.";
    const row = rowOf({ id: "stt", state: "failed", detail });
    expect(row).toHaveTextContent("Failed");
    expect(within(row).getByText(detail)).toBeInTheDocument();
    // With the program's sentence removed, only the name and the state are
    // left, so nothing was added to what the server said.
    expect(row.textContent?.replace(detail, "")).toBe("Speech to textFailed");
  });

  it("lists every engine in the order the program sent them", () => {
    const order: readonly EngineId[] = ["audio_output", "stt", "pron", "audio_input", "vad", "tts"];
    const names = ["Speaker", "Speech to text", "Pronunciation scoring", "Microphone", "Voice activity detection", "Text to speech"];
    renderApp(<EngineStatusPanel engines={order.map((id) => engine({ id }))} onRetry={vi.fn()} />);
    const rows = screen.getAllByRole("listitem").map((row) => row.textContent);
    expect(rows).toEqual(names.map((name) => `${name}Ready${DETAIL}`));
  });

  it("says the program reported no engines at all", () => {
    renderApp(<EngineStatusPanel engines={[]} onRetry={vi.fn()} />);
    expect(screen.getByText("This program reported no engines at all.")).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("asks for another try", async () => {
    const onRetry = vi.fn();
    renderApp(<EngineStatusPanel engines={[engine({ id: "vad", state: "failed", detail: "The model stopped." })]} onRetry={onRetry} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("shows the program's sentence as text, never as HTML", () => {
    const detail = "The file <b>hi</b> could not be read.";
    const row = rowOf({ id: "pron", state: "failed", detail });
    expect(row.textContent).toContain("<b>hi</b>");
    expect(row.querySelector("b")).toBeNull();
  });
});

describe("EngineStatusSection: the container", () => {
  it("says it is loading before the first snapshot, then shows the engines of the snapshot", async () => {
    const api = fakeApi();
    renderScreen(<EngineStatusSection />, api);
    expect(screen.getByText("Loading...")).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();

    await connect(api, { ...SNAPSHOT, engines: [engine({ id: "stt", state: "ready", detail: "Speech recognition is loaded." })] });
    expect(await screen.findByText("Speech to text")).toBeInTheDocument();
    expect(screen.getByText("Speech recognition is loaded.")).toBeInTheDocument();
    expect(screen.queryByText("Loading...")).toBeNull();
  });

  it("says the program reported no engines at all when the snapshot lists none", async () => {
    const api = fakeApi();
    renderScreen(<EngineStatusSection />, api);
    await connect(api, { ...SNAPSHOT, engines: [] });
    expect(await screen.findByText("This program reported no engines at all.")).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
  });
});
