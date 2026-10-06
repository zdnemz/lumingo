import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/client";
import { ENV_PROVIDER, FILE_PROVIDER, SNAPSHOT, connect, fakeApi, renderApp, renderScreen, resetBrowserState, setSystem } from "@/test/utils";
import { ConnectionBanner } from "./ConnectionBanner";
import { ErrorBanner } from "./ErrorBanner";
import { UnavailableNote } from "./UnavailableNote";

beforeEach(() => {
  resetBrowserState();
  setSystem({});
});
afterEach(() => {
  vi.useRealTimers();
});

describe("ErrorBanner", () => {
  it("says the program is not answering when no answer came, and offers another try", async () => {
    const onRetry = vi.fn();
    renderApp(<ErrorBanner error={new TypeError("network")} onRetry={onRetry} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("The program is not answering");
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalled();
  });

  it("says the program refused the page, and offers a reload", async () => {
    renderApp(<ErrorBanner error={new ApiError(403, "origin_not_allowed")} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("refused this page");
    expect(screen.getByRole("button", { name: "Reload this page" })).toBeInTheDocument();
  });

  it("turns every coded answer into a sentence and keeps the program's text on a details line", async () => {
    renderApp(<ErrorBanner error={new ApiError(409, "read_only", "the environment profile is read-only")} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("That item is read-only");
    expect(screen.getByText("Details from the program: the environment profile is read-only")).toBeInTheDocument();
  });

  it("does not fail on a code it does not know", async () => {
    renderApp(<ErrorBanner error={new ApiError(500, "something_new")} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("does not understand");
  });
});

describe("UnavailableNote", () => {
  it("says a part is not available when the snapshot lists it", async () => {
    const api = fakeApi(undefined, {});
    renderScreen(<UnavailableNote feature="speech" />, api);
    await connect(api, { ...SNAPSHOT, unavailable: ["speech"] });
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent("Not available in this build");
    expect(note).toHaveTextContent("Speech: microphone, speakers and voices: not part of this version");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("does not claim anything before the first snapshot", async () => {
    renderScreen(<UnavailableNote feature="models" />, fakeApi());
    expect(await screen.findByText("Loading...")).toBeInTheDocument();
    expect(screen.queryByRole("note")).toBeNull();
  });

  it("is honest when the build has the part but this screen has no control for it", async () => {
    const api = fakeApi();
    renderScreen(<UnavailableNote feature="models" />, api);
    await connect(api, { ...SNAPSHOT, unavailable: [] });
    expect(await screen.findByRole("note")).toHaveTextContent("has no control for it yet");
  });

  it("always says the inspector is not available, because the program has no route for it", async () => {
    renderScreen(<UnavailableNote feature="inspector" />, fakeApi());
    expect(await screen.findByRole("note")).toHaveTextContent("payload inspector is not available");
  });
});

describe("ConnectionBanner", () => {
  it("shows nothing while the program is live and a provider is set up", async () => {
    const api = fakeApi();
    renderScreen(<ConnectionBanner />, api);
    await connect(api, { ...SNAPSHOT, provider: FILE_PROVIDER });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("says the connection is lost and reconnects when asked", async () => {
    const api = fakeApi();
    renderScreen(<ConnectionBanner />, api);
    await connect(api, { ...SNAPSHOT, provider: ENV_PROVIDER });
    act(() => api.drop());
    expect(await screen.findByRole("alert")).toHaveTextContent("Connection to the program lost");
    await userEvent.setup().click(screen.getByRole("button", { name: "Try now" }));
    await vi.waitFor(() => expect(api.getState).toHaveBeenCalledTimes(2));
  });

  it("says plainly when the program refuses the page", async () => {
    const api = fakeApi(() => Promise.reject(new ApiError(403, "host_not_allowed")));
    renderScreen(<ConnectionBanner />, api);
    expect(await screen.findByRole("alert")).toHaveTextContent("refused this page");
    expect(screen.getByRole("button", { name: "Reload" })).toBeInTheDocument();
  });

  it("warns when no provider is set up, with a link to the setup guide", async () => {
    const api = fakeApi();
    renderScreen(<ConnectionBanner />, api);
    await connect(api, { ...SNAPSHOT, provider: null });
    expect(await screen.findByRole("alert")).toHaveTextContent("No AI provider is set up");
    expect(screen.getByRole("link", { name: "Set up a provider" })).toHaveAttribute("href", expect.stringMatching(/^\/onboarding\/?$/));
  });

  it("follows a provider change from the event stream", async () => {
    const api = fakeApi();
    renderScreen(<ConnectionBanner />, api);
    await connect(api, { ...SNAPSHOT, provider: null });
    expect(await screen.findByText("No AI provider is set up")).toBeInTheDocument();
    act(() => api.emit({ type: "ProviderStatus", seq: 1, provider: FILE_PROVIDER }));
    expect(screen.queryByText("No AI provider is set up")).toBeNull();
  });
});
