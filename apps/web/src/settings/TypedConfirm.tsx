"use client";

import { useEffect, useId, useRef, useState } from "react";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { StatusBanner } from "@/ui/StatusBanner";
import { TextField } from "@/ui/TextField";

/** Compares what was typed with the phrase, ignoring case and extra spaces. */
export function matchesPhrase(typed: string, phrase: string): boolean {
  const clean = (value: string) => value.trim().replace(/\s+/g, " ").toLowerCase();
  return clean(typed) !== "" && clean(typed) === clean(phrase);
}

export interface TypedConfirmProps {
  /** What the learner must type. */
  phrase: string;
  /** What is about to happen, said plainly. */
  title: string;
  body: string;
  /** The sentence that asks for the phrase. It contains the phrase. */
  prompt: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * A delete that cannot be undone asks for a typed phrase, because the program
 * itself does not ask again. The button stays off until the phrase matches.
 */
export function TypedConfirm({ phrase, title, body, prompt, busy, onConfirm, onCancel }: TypedConfirmProps) {
  const t = useT();
  const [typed, setTyped] = useState("");
  const field = useRef<HTMLInputElement>(null);
  const groupId = useId();
  const ok = matchesPhrase(typed, phrase);

  // The field takes focus when the question appears, so the keyboard continues where the click was.
  useEffect(() => {
    field.current?.focus();
  }, []);

  return (
    <StatusBanner
      tone="danger"
      title={title}
      actions={
        <>
          <Button variant="danger" disabled={!ok} busy={busy} onClick={onConfirm}>
            {busy ? t("data.confirm.busy") : t("data.confirm.yes")}
          </Button>
          <Button onClick={onCancel}>{t("data.confirm.cancel")}</Button>
        </>
      }
    >
      <div className="confirm" role="group" aria-labelledby={groupId}>
        <p>{body}</p>
        <p id={groupId}>{prompt}</p>
        <TextField
          label={t("data.confirm.label")}
          hint={t("data.confirm.hint")}
          value={typed}
          inputRef={field}
          autoComplete="off"
          spellCheck={false}
          onChange={setTyped}
        />
      </div>
    </StatusBanner>
  );
}
