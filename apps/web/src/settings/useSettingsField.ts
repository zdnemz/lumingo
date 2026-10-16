"use client";

import { useCallback, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import type { Settings } from "@/generated/Settings";
import { useAction, useResource, type ResourceState } from "@/state/useResource";

export interface SettingsField {
  state: ResourceState<Settings>;
  /** Loads again, for the retry of an error banner. */
  reload: () => void;
  busy: boolean;
  error: unknown;
  saved: boolean;
  /** Hides the saved note, for a control that is being changed again. */
  clearSaved: () => void;
  /** Saves one setting with the whole set. Resolves true when the program stored it. */
  save: (patch: Partial<Settings>) => Promise<boolean>;
}

/**
 * One settings control that writes through `PUT /api/settings`. That route
 * replaces every setting, so the whole current set is read from the program
 * just before the write: another panel on this screen may have saved something
 * since this one loaded, and a stale copy would quietly undo it. What the
 * program stored is what is shown afterwards.
 */
export function useSettingsField(): SettingsField {
  const api = useApi();
  const load = useCallback((signal: AbortSignal) => api.getSettings(signal), [api]);
  const { state, reload, replace } = useResource<Settings>(load);
  const action = useAction();
  const [saved, setSaved] = useState(false);

  const save = useCallback(
    async (patch: Partial<Settings>) => {
      setSaved(false);
      const stored = await action.run(async () => {
        const current = await api.getSettings();
        return api.updateSettings({ ...current, ...patch });
      });
      if (!stored) return false;
      replace(stored);
      setSaved(true);
      return true;
    },
    [action, api, replace],
  );

  const clearSaved = useCallback(() => setSaved(false), []);

  return { state, reload, busy: action.busy, error: action.error, saved, clearSaved, save };
}
