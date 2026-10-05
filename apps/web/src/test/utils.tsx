import { render } from "@testing-library/react";
import type { ReactElement, ReactNode } from "react";
import { vi } from "vitest";
import type { ApiClient, EventStreamHandlers } from "@/api/client";
import { Providers } from "@/components/Providers";
import type { ServerEvent } from "@/generated/ServerEvent";
import type { StateSnapshot } from "@/generated/StateSnapshot";

/** jsdom has no matchMedia. This installs one whose answers the test controls. */
export function setSystem(options: { reduced?: boolean; dark?: boolean; language?: string }): void {
  const { reduced = false, dark = true, language = "en-US" } = options;
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: query.includes("reduced-motion") ? reduced : dark,
      media: query,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }),
  });
  Object.defineProperty(window.navigator, "language", { configurable: true, value: language });
}

export function resetBrowserState(): void {
  window.localStorage.clear();
  const root = document.documentElement;
  delete root.dataset.motion;
  delete root.dataset.theme;
  delete root.dataset.crt;
  root.lang = "";
}

export const SNAPSHOT: StateSnapshot = {
  server_version: "9.9.9",
  dev_mode: false,
  uptime_ms: 5000,
  settings: {
    display_name: "Learner",
    ui_language: "en",
    l1: "id",
    l1_help_mode: "auto",
    adaptive_timing: "auto",
    keep_recordings: false,
  },
  provider: null,
  hardware: {
    ram_total_bytes: 16_000_000_000,
    logical_cores: 8,
    minimum_ram_bytes: 8_000_000_000,
    minimum_logical_cores: 4,
    meets_minimum: true,
  },
  unavailable: [],
};

export interface FakeApi extends ApiClient {
  emit: (event: ServerEvent) => void;
  open: () => void;
  drop: () => void;
  opened: () => number;
}

/** A typed client whose stream the test drives by hand. */
export function fakeApi(getState: () => Promise<StateSnapshot> = () => Promise.resolve(SNAPSHOT)): FakeApi {
  let handlers: EventStreamHandlers | null = null;
  let opened = 0;
  return {
    getState: vi.fn(getState),
    openEvents: vi.fn((next: EventStreamHandlers) => {
      handlers = next;
      opened += 1;
      return { close: () => undefined };
    }),
    emit: (event) => handlers?.onEvent(event),
    open: () => handlers?.onOpen(),
    drop: () => handlers?.onClose(),
    opened: () => opened,
  };
}

/** Renders inside the app providers. The wrapper stays in place on `rerender`. */
export function renderApp(ui: ReactElement, client?: ApiClient) {
  return render(ui, {
    wrapper: ({ children }: { children: ReactNode }) => <Providers client={client}>{children}</Providers>,
  });
}
