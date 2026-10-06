import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/client";
import type { SaveProviderRequest } from "@/generated/SaveProviderRequest";
import { FILE_PROVIDER, fakeApi, renderApp, resetBrowserState, setSystem } from "@/test/utils";
import { KEY_GUIDE_PRESET } from "./keyGuide";
import { ProviderForm } from "./ProviderForm";

const SECRET = "sk-test-SECRET-0123456789abcdef";

beforeEach(() => {
  resetBrowserState();
  setSystem({});
});

async function fillNew(user: ReturnType<typeof userEvent.setup>) {
  await user.type(await screen.findByLabelText("Profile name"), "my-gemini");
  await user.type(screen.getByLabelText("Base URL"), "https://api.example.test/v1");
  await user.type(screen.getByLabelText("Model name"), "tutor-model");
}

describe("ProviderForm", () => {
  it("sends the typed key once and never renders it again after the save", async () => {
    const user = userEvent.setup();
    const saveProvider = vi.fn((request: SaveProviderRequest) => {
      void request;
      return Promise.resolve({ ...FILE_PROVIDER, key_last4: "cdef" });
    });
    const onSaved = vi.fn();
    const { container } = renderApp(
      <ProviderForm makeActiveDefault onSaved={onSaved} />,
      fakeApi(undefined, { saveProvider }),
    );
    await fillNew(user);
    const keyField = screen.getByLabelText("API key");
    expect(keyField).toHaveAttribute("type", "password");
    await user.type(keyField, SECRET);
    await user.click(screen.getByRole("button", { name: "Save profile" }));

    await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
    expect(saveProvider).toHaveBeenCalledWith({
      name: "my-gemini",
      protocol: "openai_chat",
      base_url: "https://api.example.test/v1",
      model: "tutor-model",
      api_key: SECRET,
      clear_key: undefined,
      make_active: true,
    });
    expect((screen.getByLabelText("API key") as HTMLInputElement).value).toBe("");
    expect(document.body.innerHTML).not.toContain(SECRET);
    expect(container.textContent).not.toContain(SECRET);
    expect(JSON.stringify({ ...window.localStorage })).not.toContain(SECRET);
  });

  it("keeps what was typed when the save fails, and says why with a way to retry", async () => {
    const user = userEvent.setup();
    const saveProvider = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(400, "invalid_input", "the model name is empty"))
      .mockResolvedValueOnce(FILE_PROVIDER);
    const onSaved = vi.fn();
    renderApp(<ProviderForm makeActiveDefault={false} onSaved={onSaved} />, fakeApi(undefined, { saveProvider }));
    await fillNew(user);
    await user.type(screen.getByLabelText("API key"), SECRET);
    await user.click(screen.getByRole("button", { name: "Save profile" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("did not accept those values");
    expect(screen.getByRole("alert")).toHaveTextContent("the model name is empty");
    expect((screen.getByLabelText("API key") as HTMLInputElement).value).toBe(SECRET);
    expect(onSaved).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Save profile" }));
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
  });

  it("checks the fields before it asks the program", async () => {
    const user = userEvent.setup();
    const saveProvider = vi.fn();
    renderApp(<ProviderForm makeActiveDefault onSaved={vi.fn()} />, fakeApi(undefined, { saveProvider }));
    await user.click(await screen.findByRole("button", { name: "Save profile" }));
    expect(screen.getByText("Give the profile a name of 1 to 64 characters.")).toBeInTheDocument();
    expect(screen.getByText("Start the address with https://")).toBeInTheDocument();
    expect(screen.getByText("Write the model name.")).toBeInTheDocument();
    expect(saveProvider).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Base URL")).toHaveAttribute("aria-invalid", "true");
  });

  it("sends no key when editing and the field is left empty, so the saved key is kept", async () => {
    const user = userEvent.setup();
    const saveProvider = vi.fn().mockResolvedValue(FILE_PROVIDER);
    renderApp(
      <ProviderForm initial={FILE_PROVIDER} makeActiveDefault onSaved={vi.fn()} />,
      fakeApi(undefined, { saveProvider }),
    );
    expect(await screen.findByLabelText("Profile name")).toHaveAttribute("readonly");
    await user.clear(screen.getByLabelText("Model name"));
    await user.type(screen.getByLabelText("Model name"), "other-model");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(saveProvider).toHaveBeenCalled());
    const request = saveProvider.mock.calls[0]?.[0] as SaveProviderRequest;
    expect(request.api_key).toBeUndefined();
    expect(request.model).toBe("other-model");
  });

  it("asks to remove the saved key only when one exists", async () => {
    const user = userEvent.setup();
    const saveProvider = vi.fn().mockResolvedValue(FILE_PROVIDER);
    renderApp(
      <ProviderForm initial={FILE_PROVIDER} makeActiveDefault onSaved={vi.fn()} />,
      fakeApi(undefined, { saveProvider }),
    );
    await user.click(await screen.findByRole("switch", { name: "Remove the saved key" }));
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(saveProvider).toHaveBeenCalled());
    expect((saveProvider.mock.calls[0]?.[0] as SaveProviderRequest).clear_key).toBe(true);
  });

  it("takes the key guide's values but never a key", async () => {
    const { rerender } = renderApp(<ProviderForm makeActiveDefault onSaved={vi.fn()} />, fakeApi());
    expect(await screen.findByLabelText("Base URL")).toHaveValue("");
    rerender(<ProviderForm makeActiveDefault onSaved={vi.fn()} preset={KEY_GUIDE_PRESET} />);
    expect(screen.getByLabelText("Base URL")).toHaveValue(KEY_GUIDE_PRESET.base_url);
    expect(screen.getByLabelText("Model name")).toHaveValue(KEY_GUIDE_PRESET.model);
    expect(screen.getByLabelText("API key")).toHaveValue("");
  });
});
