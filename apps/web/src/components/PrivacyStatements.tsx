"use client";

import type { MessageKey } from "@/i18n";
import { useT } from "@/state/PreferencesProvider";

/** Plain statements of what leaves the computer, from PRD 12.1 to 12.3. Used on the setup and the privacy screens. */
const STATEMENTS: readonly MessageKey[] = [
  "privacy.voice",
  "privacy.text",
  "privacy.provider",
  "privacy.key",
  "privacy.keysent",
  "privacy.never",
  "privacy.audio",
  "privacy.local",
];

export function PrivacyStatements({ only }: { only?: readonly MessageKey[] }) {
  const t = useT();
  return (
    <ul className="statements">
      {(only ?? STATEMENTS).map((key) => (
        <li key={key}>{t(key)}</li>
      ))}
    </ul>
  );
}
