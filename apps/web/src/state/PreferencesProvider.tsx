"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { translate, type MessageKey, type TranslateParams } from "@/i18n";
import {
  PREFS_KEY,
  applyToDocument,
  defaultPreferences,
  loadPreferences,
  readSystemHints,
  resolveDisplay,
  savePreferences,
  type Preferences,
  type ResolvedDisplay,
  type SystemHints,
} from "./preferences";

interface PreferencesContextValue {
  prefs: Preferences;
  display: ResolvedDisplay;
  /** False until the page has hydrated and the stored preferences have been read. */
  ready: boolean;
  setPrefs: (patch: Partial<Preferences>) => void;
}

const PreferencesContext = createContext<PreferencesContextValue | null>(null);

/** What the server render and the first client render both see. */
const SERVER_PREFS = defaultPreferences();
const SERVER_HINTS: SystemHints = { prefersReducedMotion: true, prefersDark: true, browserLanguage: "en" };

/** Stored preferences, as an external store so React reads them without a setState-in-effect. */
function createPrefsStore() {
  let current: Preferences | null = null;
  const listeners = new Set<() => void>();
  return {
    getSnapshot(): Preferences {
      current ??= loadPreferences(window);
      return current;
    },
    set(patch: Partial<Preferences>) {
      current = { ...this.getSnapshot(), ...patch };
      savePreferences(window, current);
      for (const listener of listeners) listener();
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      // Another tab changed the setting.
      const onStorage = (event: StorageEvent) => {
        if (event.key === PREFS_KEY) {
          current = null;
          listener();
        }
      };
      window.addEventListener("storage", onStorage);
      return () => {
        listeners.delete(listener);
        window.removeEventListener("storage", onStorage);
      };
    },
  };
}

/** The system's motion and colour preferences, kept referentially stable between reads. */
function createHintsStore() {
  let current: SystemHints | null = null;
  return {
    getSnapshot(): SystemHints {
      const next = readSystemHints(window);
      if (
        current !== null &&
        current.prefersReducedMotion === next.prefersReducedMotion &&
        current.prefersDark === next.prefersDark &&
        current.browserLanguage === next.browserLanguage
      ) {
        return current;
      }
      current = next;
      return next;
    },
    subscribe(listener: () => void) {
      if (typeof window.matchMedia !== "function") return () => undefined;
      const queries = [
        window.matchMedia("(prefers-reduced-motion: reduce)"),
        window.matchMedia("(prefers-color-scheme: dark)"),
      ];
      for (const query of queries) query.addEventListener("change", listener);
      return () => {
        for (const query of queries) query.removeEventListener("change", listener);
      };
    },
  };
}

const noopSubscribe = () => () => undefined;

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [prefsStore] = useState(createPrefsStore);
  const [hintsStore] = useState(createHintsStore);

  const prefs = useSyncExternalStore(prefsStore.subscribe, prefsStore.getSnapshot, () => SERVER_PREFS);
  const hints = useSyncExternalStore(hintsStore.subscribe, hintsStore.getSnapshot, () => SERVER_HINTS);
  // False while hydrating, true afterwards: the standard way to ask "are we in the browser yet?"
  const ready = useSyncExternalStore(
    noopSubscribe,
    () => true,
    () => false,
  );

  const display = useMemo(() => resolveDisplay(prefs, hints), [prefs, hints]);

  useEffect(() => {
    if (ready) applyToDocument(display, document.documentElement);
  }, [display, ready]);

  const setPrefs = useCallback((patch: Partial<Preferences>) => prefsStore.set(patch), [prefsStore]);

  const value = useMemo(() => ({ prefs, display, ready, setPrefs }), [prefs, display, ready, setPrefs]);
  return <PreferencesContext.Provider value={value}>{children}</PreferencesContext.Provider>;
}

export function usePreferences(): PreferencesContextValue {
  const value = useContext(PreferencesContext);
  if (value === null) {
    throw new Error("usePreferences must be used inside PreferencesProvider");
  }
  return value;
}

/** True when animations may run. Code that animates in JavaScript checks this. */
export function useMotionEnabled(): boolean {
  return usePreferences().display.motion === "on";
}

export type Translate = (key: MessageKey, params?: TranslateParams) => string;

export function useT(): Translate {
  const { display } = usePreferences();
  const language = display.language;
  return useCallback((key, params) => translate(language, key, params), [language]);
}
