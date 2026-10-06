import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/client";
import type { ProviderList } from "@/generated/ProviderList";
import { ENV_PROVIDER, FILE_PROVIDER, PROBE_OK, fakeApi, probeFailure, providerList, renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { ProviderManager } from "./ProviderManager";

beforeEach(() => {
  resetBrowserState();
  setSystem({});
});

const OTHER = { ...FILE_PROVIDER, id: 3, name: "second", is_active: false, key_last4: "1111" };

function card(name: string): HTMLElement {
  return screen.getByRole("article", { name });
}

describe("ProviderManager", () => {
  it("shows loading, then the profiles with the key only as has_key and last four", async () => {
    let resolve: (list: ProviderList) => void = () => undefined;
    const listProviders = vi.fn(() => new Promise<ProviderList>((done) => (resolve = done)));
    const { container } = renderApp(<ProviderManager />, fakeApi(undefined, { listProviders }));
    expect(screen.getByRole("status")).toHaveTextContent("Loading");
    resolve(providerList([ENV_PROVIDER, FILE_PROVIDER, { ...OTHER, has_key: false, key_last4: null }]));

    expect(await screen.findByRole("article", { name: "my-gemini" })).toBeInTheDocument();
    expect(card("my-gemini")).toHaveTextContent("Key saved. It ends with ••••4321.");
    expect(card("second")).toHaveTextContent("No key saved.");
    expect(card("env")).toHaveTextContent("From your .env file, read-only");
    expect(container.textContent).not.toMatch(/sk-|api_key/i);
  });

  it("shows the empty state", async () => {
    renderApp(<ProviderManager />, fakeApi(undefined, { listProviders: () => Promise.resolve(providerList([])) }));
    expect(await screen.findByText("No provider profiles yet.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add a profile" })).toBeInTheDocument();
  });

  it("shows the error state with a retry when the program does not answer", async () => {
    const listProviders = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("network"))
      .mockResolvedValueOnce(providerList([FILE_PROVIDER]));
    renderApp(<ProviderManager />, fakeApi(undefined, { listProviders }));
    expect(await screen.findByText("The program is not answering")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("article", { name: "my-gemini" })).toBeInTheDocument();
  });

  it("offers no edit or delete for the read-only env profile, and says why", async () => {
    renderApp(<ProviderManager />, fakeApi(undefined, { listProviders: () => Promise.resolve(providerList([ENV_PROVIDER])) }));
    const env = await screen.findByRole("article", { name: "env" });
    expect(within(env).queryByRole("button", { name: "Edit" })).toBeNull();
    expect(within(env).queryByRole("button", { name: "Delete" })).toBeNull();
    expect(env).toHaveTextContent("To change it, edit that file");
    expect(within(env).getByRole("button", { name: "Test the connection" })).toBeInTheDocument();
  });

  it("activates another profile and reloads the list", async () => {
    const user = userEvent.setup();
    const listProviders = vi
      .fn()
      .mockResolvedValueOnce(providerList([FILE_PROVIDER, OTHER]))
      .mockResolvedValue(providerList([{ ...FILE_PROVIDER, is_active: false }, { ...OTHER, is_active: true }]));
    const activateProvider = vi.fn().mockResolvedValue({ ...OTHER, is_active: true });
    renderApp(<ProviderManager />, fakeApi(undefined, { listProviders, activateProvider }));
    await user.click(await within(await screen.findByRole("article", { name: "second" })).findByRole("button", { name: "Use this one" }));
    expect(activateProvider).toHaveBeenCalledWith(3);
    await waitFor(() => expect(within(card("second")).getByText("In use")).toBeInTheDocument());
  });

  it("asks before deleting, and deletes only after the learner agrees", async () => {
    const user = userEvent.setup();
    const deleteProvider = vi.fn().mockResolvedValue(providerList([OTHER]));
    renderApp(
      <ProviderManager />,
      fakeApi(undefined, { listProviders: () => Promise.resolve(providerList([FILE_PROVIDER, OTHER])), deleteProvider }),
    );
    const mine = await screen.findByRole("article", { name: "my-gemini" });
    await user.click(within(mine).getByRole("button", { name: "Delete" }));
    expect(deleteProvider).not.toHaveBeenCalled();
    expect(screen.getByText("Delete the profile my-gemini?")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Keep it" }));
    expect(screen.queryByText("Delete the profile my-gemini?")).toBeNull();

    await user.click(within(mine).getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Yes, delete it" }));
    await waitFor(() => expect(deleteProvider).toHaveBeenCalledWith(2));
    await waitFor(() => expect(screen.queryByRole("article", { name: "my-gemini" })).toBeNull());
    expect(screen.getByRole("article", { name: "second" })).toBeInTheDocument();
  });

  it("explains a refused delete with the program's sentence", async () => {
    const user = userEvent.setup();
    const deleteProvider = vi.fn().mockRejectedValue(new ApiError(409, "conflict", "providers.toml cannot be read"));
    renderApp(
      <ProviderManager />,
      fakeApi(undefined, { listProviders: () => Promise.resolve(providerList([FILE_PROVIDER])), deleteProvider }),
    );
    await user.click(await screen.findByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Yes, delete it" }));
    expect(await screen.findByText(/providers.toml cannot be read/)).toBeInTheDocument();
    expect(screen.getByRole("article", { name: "my-gemini" })).toBeInTheDocument();
  });

  it("tests a profile and shows the result; a wrong key opens the form with focus on the key", async () => {
    const user = userEvent.setup();
    const testProvider = vi.fn().mockResolvedValueOnce(probeFailure("auth")).mockResolvedValueOnce(PROBE_OK);
    renderApp(
      <ProviderManager />,
      fakeApi(undefined, { listProviders: () => Promise.resolve(providerList([FILE_PROVIDER])), testProvider }),
    );
    await user.click(await screen.findByRole("button", { name: "Test the connection" }));
    expect(await screen.findByText("The provider did not accept the key")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Enter the key again" }));
    const key = await screen.findByLabelText("API key");
    expect(key).toHaveFocus();
    expect(screen.getByLabelText("Profile name")).toHaveAttribute("readonly");
  });

  it("shows a test that succeeds, and a program that is busy", async () => {
    const user = userEvent.setup();
    const testProvider = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(409, "busy", "another request of this kind is still running"))
      .mockResolvedValueOnce(PROBE_OK);
    renderApp(
      <ProviderManager />,
      fakeApi(undefined, { listProviders: () => Promise.resolve(providerList([FILE_PROVIDER])), testProvider }),
    );
    await user.click(await screen.findByRole("button", { name: "Test the connection" }));
    expect(await screen.findByText(/Another request of this kind is still running/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test the connection" }));
    expect(await screen.findByText("The connection works")).toBeInTheDocument();
  });

  it("shows problems with the provider files as a warning", async () => {
    renderApp(
      <ProviderManager />,
      fakeApi(undefined, {
        listProviders: () => Promise.resolve(providerList([], ["providers file: the file is not valid TOML"])),
      }),
    );
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("A provider file has a problem");
    expect(alert).toHaveTextContent("providers file: the file is not valid TOML");
  });

  it("adds a profile through the form and reloads the list", async () => {
    const user = userEvent.setup();
    const listProviders = vi
      .fn()
      .mockResolvedValueOnce(providerList([]))
      .mockResolvedValue(providerList([FILE_PROVIDER]));
    const saveProvider = vi.fn().mockResolvedValue(FILE_PROVIDER);
    renderApp(<ProviderManager />, fakeApi(undefined, { listProviders, saveProvider }));
    await user.click(await screen.findByRole("button", { name: "Add a profile" }));
    await user.type(screen.getByLabelText("Profile name"), "my-gemini");
    await user.type(screen.getByLabelText("Base URL"), "https://api.example.test/v1");
    await user.type(screen.getByLabelText("Model name"), "tutor-model");
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    expect(await screen.findByRole("article", { name: "my-gemini" })).toBeInTheDocument();
  });
});
