"use client";

import { LANGUAGES, MOTION_SETTINGS, THEME_SETTINGS } from "@/state/preferences";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { Switch } from "@/ui/Switch";

export function DisplaySettings() {
  const t = useT();
  const { prefs, setPrefs } = usePreferences();
  return (
    <Panel title={t("settings.display")} raised>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <Segmented
          legend={t("settings.motion")}
          hint={t("settings.motion.help")}
          value={prefs.motion}
          onChange={(motion) => setPrefs({ motion })}
          options={MOTION_SETTINGS.map((value) => ({ value, label: t(`settings.motion.${value}`) }))}
        />
        <Segmented
          legend={t("settings.theme")}
          value={prefs.theme}
          onChange={(theme) => setPrefs({ theme })}
          options={THEME_SETTINGS.map((value) => ({ value, label: t(`settings.theme.${value}`) }))}
        />
        <Segmented
          legend={t("settings.language")}
          value={prefs.language}
          onChange={(language) => setPrefs({ language })}
          options={LANGUAGES.map((value) => ({ value, label: t(`settings.language.${value}`) }))}
        />
        <Switch
          label={t("settings.crt")}
          hint={t("settings.crt.help")}
          checked={prefs.crt}
          onChange={(crt) => setPrefs({ crt })}
          onText={t("settings.state.on")}
          offText={t("settings.state.off")}
        />
      </div>
    </Panel>
  );
}
