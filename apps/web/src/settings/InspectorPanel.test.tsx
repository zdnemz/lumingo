import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiClient } from "@/api/client";
import type { InspectorReport } from "@/generated/InspectorReport";
import type { PayloadEntryView } from "@/generated/PayloadEntryView";
import { SNAPSHOT, connect, fakeApi, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { InspectorPanel } from "./InspectorPanel";

const LIMITS = { capacity: 50, max_body_bytes: 16 * 1024 };

function entry(patch: Partial<PayloadEntryView> & { id: number }): PayloadEntryView {
  return {
    started_unix_ms: 1_700_000_000_000,
    endpoint: "api.example.test/v1/chat/completions",
    streaming: false,
    request_body: '{"model":"tutor-model"}',
    request_truncated: false,
    status: 200,
    response_body: '{"choices":[]}',
    response_truncated: false,
    outcome: "ok",
    elapsed_ms: 420,
    ...patch,
  };
}

function report(patch: Partial<InspectorReport> = {}): InspectorReport {
  return { ...LIMITS, dropped: 0, entries: [], ...patch };
}

function routes(overrides: Partial<ApiClient> = {}): Partial<ApiClient> {
  return { getInspector: () => Promise.resolve(report()), ...overrides };
}

async function open(overrides: Partial<ApiClient> = {}) {
  const api = fakeApi(undefined, routes(overrides));
  renderScreen(<InspectorPanel />, api);
  await connect(api, SNAPSHOT);
  return api;
}

beforeEach(() => {
  resetBrowserState();
  setSystem({ language: "en-US" });
});

describe("InspectorPanel", () => {
  it("states what it shows, the limits the program keeps and the empty state", async () => {
    await open();
    expect(await screen.findByText(/The last requests the program sent to the AI provider/)).toBeInTheDocument();
    expect(screen.getByText(/The key is removed when an entry is captured and no header is kept\./)).toBeInTheDocument();
    expect(screen.getByText("Keeps the newest 50 entries, and cuts each request or response body at 16384 bytes.")).toBeInTheDocument();
    expect(screen.getByText("Older entries are removed when the list is full. Dropped so far: 0.")).toBeInTheDocument();
    expect(screen.getByText("Nothing has been sent to a provider yet. The first request the tutor makes appears here.")).toBeInTheDocument();
    expect(screen.queryByRole("listitem")).toBeNull();
  });

  it("draws each entry oldest first, with its endpoint, status, outcome, time and both bodies as text", async () => {
    await open({
      getInspector: () =>
        Promise.resolve(
          report({
            entries: [
              entry({ id: 1 }),
              entry({
                id: 2,
                status: null,
                outcome: "failed",
                elapsed_ms: null,
                response_body: null,
                request_body: '{"prompt":"<b>hi</b>"}',
              }),
            ],
          }),
        ),
    });
    const rows = await screen.findAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("Request 1: api.example.test/v1/chat/completions, 200 ok, 420 ms");
    expect(rows[1]).toHaveTextContent("Request 2: api.example.test/v1/chat/completions, failed, —");
    // A body is shown as text, never run as markup.
    expect(rows[1]).toHaveTextContent('{"prompt":"<b>hi</b>"}');
    expect(rows[1]?.querySelector("b")).toBeNull();
    // A request that never came back has no response section.
    expect(rows[1]).not.toHaveTextContent("Response body");
  });

  it("marks a body the program cut at the stored limit, once for each body", async () => {
    await open({
      getInspector: () =>
        Promise.resolve(report({ dropped: 3, entries: [entry({ id: 9, request_truncated: true, response_truncated: true })] })),
    });
    const row = (await screen.findAllByRole("listitem"))[0] as HTMLElement;
    const marks = [...row.querySelectorAll("*")].filter((element) => element.textContent?.trim() === "The body was cut at the stored limit.");
    expect(marks).toHaveLength(2);
    expect(screen.getByText("Older entries are removed when the list is full. Dropped so far: 3.")).toBeInTheDocument();
  });

  it("says so when the inspector cannot be read, and reads it again on request", async () => {
    const getInspector = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(report({ entries: [entry({ id: 4 })] }));
    await open({ getInspector });
    expect(await screen.findByText("The inspector could not be read.")).toBeInTheDocument();
    expect(screen.getByText("The program is not answering")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText(/Request 4: /)).toBeInTheDocument();
  });

  it("shows loading while the report comes", async () => {
    await open({ getInspector: () => new Promise(() => undefined) });
    expect((await screen.findAllByRole("status")).some((status) => status.textContent === "Loading...")).toBe(true);
  });
});
