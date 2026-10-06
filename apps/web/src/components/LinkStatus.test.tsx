import { act, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ServerStateProvider } from "@/api/ServerState";
import { ApiError } from "@/api/client";
import { SNAPSHOT, fakeApi, renderApp, resetBrowserState, setSystem, type FakeApi } from "@/test/utils";
import { LinkStatus } from "./LinkStatus";

function renderLink(api: FakeApi) {
  return renderApp(
    <ServerStateProvider>
      <LinkStatus />
    </ServerStateProvider>,
    api,
  );
}

beforeEach(() => {
  resetBrowserState();
  setSystem({});
});
afterEach(() => {
  vi.useRealTimers();
});

describe("LinkStatus", () => {
  it("shows connecting, then live with the snapshot and the events in order", async () => {
    const api = fakeApi();
    renderLink(api);
    expect(await screen.findByText("Connecting")).toBeInTheDocument();

    await vi.waitFor(() => expect(api.opened()).toBe(1));
    act(() => {
      api.open();
      api.emit({ type: "Snapshot", seq: 4, state: SNAPSHOT });
      api.emit({ type: "Heartbeat", seq: 5, uptime_ms: 7000 });
      api.emit({ type: "Heartbeat", seq: 6, uptime_ms: 8000 });
    });

    expect(await screen.findByText("Live")).toBeInTheDocument();
    expect(screen.getByText("9.9.9")).toBeInTheDocument();
    expect(screen.getByText("8 s")).toBeInTheDocument();
    const items = screen.getAllByRole("listitem").map((li) => li.textContent);
    expect(items).toEqual(["#6 Heartbeat", "#5 Heartbeat", "#4 Snapshot"]);
  });

  it("says plainly when the server refuses the page", async () => {
    const api = fakeApi(() => Promise.reject(new ApiError(403, "origin_not_allowed")));
    renderLink(api);
    expect(await screen.findByText(/refused this page/)).toBeInTheDocument();
    expect(api.opened()).toBe(0);
  });

  it("shows the lost state and reconnects with a fresh snapshot", async () => {
    const api = fakeApi();
    renderLink(api);
    await vi.waitFor(() => expect(api.opened()).toBe(1));
    act(() => {
      api.open();
      api.emit({ type: "Snapshot", seq: 0, state: SNAPSHOT });
    });
    expect(await screen.findByText("Live")).toBeInTheDocument();

    vi.useFakeTimers();
    act(() => api.drop());
    expect(screen.getByText(/Connection lost/)).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1100);
    });
    expect(api.opened()).toBe(2);

    act(() => {
      api.open();
      api.emit({ type: "Snapshot", seq: 40, state: { ...SNAPSHOT, server_version: "9.9.10" } });
    });
    expect(screen.getByText("Live")).toBeInTheDocument();
    expect(screen.getByText("9.9.10")).toBeInTheDocument();
  });

  it("retries when the server cannot be reached at all", async () => {
    let calls = 0;
    const api = fakeApi(() => {
      calls += 1;
      return calls === 1 ? Promise.reject(new TypeError("network")) : Promise.resolve(SNAPSHOT);
    });
    vi.useFakeTimers();
    renderLink(api);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(screen.getByText(/Connection lost/)).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1100);
    });
    expect(api.opened()).toBe(1);
  });
});
