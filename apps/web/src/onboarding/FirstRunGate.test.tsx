import { beforeEach, describe, expect, it, vi } from "vitest";
import { ENV_PROVIDER, SNAPSHOT, connect, fakeApi, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { FirstRunGate } from "./FirstRunGate";
import { ONBOARDING_KEY } from "./progress";

const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
vi.mock("next/navigation", () => ({ useRouter: () => router, usePathname: () => "/" }));

beforeEach(() => {
  resetBrowserState();
  setSystem({});
  router.replace.mockClear();
});

async function open(provider: typeof SNAPSHOT.provider) {
  const api = fakeApi();
  renderScreen(<FirstRunGate />, api);
  await connect(api, { ...SNAPSHOT, provider });
}

describe("FirstRunGate", () => {
  it("sends a first-time learner with no provider to the setup", async () => {
    await open(null);
    await vi.waitFor(() => expect(router.replace).toHaveBeenCalledWith("/onboarding/"));
  });

  it("does not move someone who has a provider, for example from .env", async () => {
    await open(ENV_PROVIDER);
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(router.replace).not.toHaveBeenCalled();
  });

  it("does not move someone who has opened the setup before", async () => {
    window.localStorage.setItem(ONBOARDING_KEY, JSON.stringify({ step: "language", done: false }));
    await open(null);
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(router.replace).not.toHaveBeenCalled();
  });

  it("waits for the first snapshot before it decides", async () => {
    renderScreen(<FirstRunGate />, fakeApi());
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(router.replace).not.toHaveBeenCalled();
  });
});
