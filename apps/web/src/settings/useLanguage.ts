"use client";

import { useCallback } from "react";
import { useApi } from "@/api/ApiProvider";
import { usePreferences } from "@/state/PreferencesProvider";
import type { Language } from "@/state/preferences";
import { useAction } from "@/state/useResource";

/**
 * Changes the interface language in two places: this browser, at once, so the
 * screen changes with no wait, and the program's settings, so the choice is
 * kept with the rest of the learner's data. If the program cannot save it, the
 * browser keeps the new language and `error` says so.
 */
export function useLanguage() {
  const api = useApi();
  const { prefs, setPrefs } = usePreferences();
  const action = useAction();
  const { run, clear } = action;

  const setLanguage = useCallback(
    async (language: Language) => {
      setPrefs({ language });
      await run(async () => {
        // PUT /api/settings replaces every setting, so the rest is read first.
        const current = await api.getSettings();
        if (current.ui_language !== language) {
          await api.updateSettings({ ...current, ui_language: language });
        }
      });
    },
    [api, run, setPrefs],
  );

  return { language: prefs.language, setLanguage, error: action.error, busy: action.busy, clear };
}
