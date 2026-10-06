import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProbeFailureKind } from "@/generated/ProbeFailureKind";
import { CAPS, PROBE_OK, probeFailure, renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { ProbeResult } from "./ProbeResult";

beforeEach(() => {
  resetBrowserState();
  setSystem({});
});

const KINDS: ProbeFailureKind[] = [
  "cancelled",
  "timeout",
  "network",
  "auth",
  "rate_limited",
  "provider_error",
  "unexpected_reply",
  "configuration",
];

describe("ProbeResult", () => {
  it("says plainly what the test found", async () => {
    renderApp(<ProbeResult report={PROBE_OK} onRetest={vi.fn()} />);
    expect(await screen.findByText("The connection works")).toBeInTheDocument();
    const facts = screen.getByRole("status").querySelector("dl") as HTMLElement;
    const text = facts.textContent ?? "";
    expect(text).toContain("Key accepted" + "Yes");
    expect(text).toContain("Streaming replies" + "Yes");
    expect(text).toContain("Time to the first word" + "420 ms");
    expect(text).toContain("55 tokens per second");
    expect(text).toContain("Level 1: native JSON schema, the strongest");
    expect(text).toContain("15 requests per minute");
    expect(text).toContain("2.3 s");
  });

  it("says when nothing was measured or reported", async () => {
    const caps = { ...CAPS, ttft_ms: null, tokens_per_second: null, structured_level: null, rate_limit_rpm: null };
    renderApp(<ProbeResult report={{ ...PROBE_OK, capabilities: caps }} onRetest={vi.fn()} />);
    expect(await screen.findByText("No level worked")).toBeInTheDocument();
    expect(screen.getByText("The provider reported none")).toBeInTheDocument();
    expect(screen.getAllByText("Not measured")).toHaveLength(2);
  });

  it("warns when the key works but replies do not stream", async () => {
    const caps = { ...CAPS, stream_ok: false };
    renderApp(<ProbeResult report={{ ...PROBE_OK, capabilities: caps }} onRetest={vi.fn()} onEdit={vi.fn()} />);
    expect(await screen.findByText("The key works, but replies did not stream")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit the profile" })).toBeInTheDocument();
  });

  it.each(KINDS)("a %s failure is an alert with a recovery action and the program's details line", async (kind) => {
    const onRetest = vi.fn();
    renderApp(<ProbeResult report={probeFailure(kind)} onRetest={onRetest} onEnterKey={vi.fn()} onEdit={vi.fn()} />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(`test failure: ${kind}`);
    const buttons = screen.getAllByRole("button");
    expect(buttons.length).toBeGreaterThan(0);
    await userEvent.setup().click(screen.getByRole("button", { name: "Test again" }));
    expect(onRetest).toHaveBeenCalledTimes(1);
  });

  it("a wrong key leads back to the key field", async () => {
    const onEnterKey = vi.fn();
    renderApp(<ProbeResult report={probeFailure("auth")} onRetest={vi.fn()} onEnterKey={onEnterKey} />);
    expect(await screen.findByText("The provider did not accept the key")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Enter the key again" }));
    expect(onEnterKey).toHaveBeenCalled();
  });

  it("an unreachable provider offers the address as well as another try", async () => {
    renderApp(<ProbeResult report={probeFailure("network")} onRetest={vi.fn()} onEdit={vi.fn()} />);
    expect(await screen.findByText("The provider could not be reached")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit the profile" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Enter the key again" })).toBeNull();
  });
});
