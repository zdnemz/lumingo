import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiagnosticsReport } from "@/generated/DiagnosticsReport";
import { DIAGNOSTICS, fakeApi, renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { DiagnosticsScreen } from "./DiagnosticsScreen";

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

function open(report: () => Promise<DiagnosticsReport>) {
  return renderApp(<DiagnosticsScreen />, fakeApi(undefined, { getDiagnostics: report }));
}

describe("DiagnosticsScreen", () => {
  it("shows loading, then the facts of the program, the computer, the provider and the speed", async () => {
    let resolve: (value: DiagnosticsReport) => void = () => undefined;
    open(() => new Promise((done) => (resolve = done)));
    expect(screen.getByRole("status")).toHaveTextContent("Loading");
    resolve(DIAGNOSTICS);

    expect(await screen.findByText("1 h 2 min")).toBeInTheDocument();
    expect(screen.getByText("127.0.0.1:8765")).toBeInTheDocument();
    expect(screen.getByText("Normal")).toBeInTheDocument();
    // Hardware
    expect(screen.getByText("16 GB")).toBeInTheDocument();
    // Provider and its structured-output level
    expect(screen.getByText("my-gemini")).toBeInTheDocument();
    expect(screen.getByText("Level 1: native JSON schema, the strongest")).toBeInTheDocument();
    // Lessons
    expect(screen.getByText("Unit files found")).toBeInTheDocument();
    expect(screen.getByText("abc123")).toBeInTheDocument();
    // Latency table: a measured step and a step with no measurements
    const table = screen.getByRole("table");
    const rows = within(table).getAllByRole("row");
    expect(rows[1]).toHaveTextContent("stt_ms4410 ms690 ms");
    expect(rows[2]).toHaveTextContent("e2e_ms0n/an/a");
    // Provider calls
    expect(screen.getByText("median 500 ms, 95th percentile 900 ms")).toBeInTheDocument();
  });

  it("shows the empty states instead of invented numbers", async () => {
    open(() =>
      Promise.resolve({
        ...DIAGNOSTICS,
        provider: null,
        latency: [],
        llm: { calls: 0, failures: 0, ttft: { count: 0, p50_ms: null, p95_ms: null }, total: { count: 0, p50_ms: null, p95_ms: null } },
        server_address: null,
        hardware: { ...DIAGNOSTICS.hardware, ram_total_bytes: null, logical_cores: null, meets_minimum: null },
      }),
    );
    expect(await screen.findByText("No provider is set up.")).toBeInTheDocument();
    expect(screen.getByText(/No measurements yet/)).toBeInTheDocument();
    expect(screen.getByText("No provider calls recorded yet.")).toBeInTheDocument();
    expect(screen.getByText("Not reported")).toBeInTheDocument();
    expect(screen.getAllByText("Not reported by the system")).toHaveLength(2);
    expect(screen.getByText(/cannot tell whether this computer meets the minimum/)).toBeInTheDocument();
  });

  it("says when the active provider has not passed a test", async () => {
    open(() => Promise.resolve({ ...DIAGNOSTICS, provider: { ...DIAGNOSTICS.provider!, capabilities: null } }));
    expect(await screen.findByText("The active provider has not passed a connection test yet.")).toBeInTheDocument();
  });

  it("says a computer below the minimum is below it", async () => {
    open(() => Promise.resolve({ ...DIAGNOSTICS, hardware: { ...DIAGNOSTICS.hardware, ram_total_bytes: 4_000_000_000, meets_minimum: false } }));
    expect(await screen.findByText(/below the minimum of 8 GB memory and 4 logical processors/)).toBeInTheDocument();
  });

  it("shows an error with a retry, and refresh loads again", async () => {
    const user = userEvent.setup();
    const getDiagnostics = vi.fn().mockRejectedValueOnce(new TypeError("network")).mockResolvedValue(DIAGNOSTICS);
    open(getDiagnostics);
    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("abc123")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await vi.waitFor(() => expect(getDiagnostics).toHaveBeenCalledTimes(3));
  });
});
