"use client";

import { useId, useState, type KeyboardEvent } from "react";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";

export interface ChatInputProps {
  /** Called with the trimmed text. A blank message is never sent. */
  onSend: (text: string) => void;
  /** True while a message is on its way, so a double click cannot send twice. */
  busy?: boolean;
  disabled?: boolean;
}

/** One text box and a Send button. Enter sends; Shift and Enter starts a new line. */
export function ChatInput({ onSend, busy, disabled }: ChatInputProps) {
  const t = useT();
  const id = useId();
  const hintId = useId();
  const [text, setText] = useState("");

  const send = () => {
    const trimmed = text.trim();
    if (trimmed === "" || busy || disabled) return;
    onSend(trimmed);
    setText("");
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      send();
    }
  };

  return (
    <form
      className="chat-input"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <label className="px-label" htmlFor={id}>
        {t("chat.input.label")}
      </label>
      <textarea
        id={id}
        className="px-field"
        rows={2}
        value={text}
        placeholder={t("chat.input.placeholder")}
        aria-describedby={hintId}
        disabled={disabled}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
      />
      <p id={hintId} className="px-hint">
        {t("chat.input.hint")}
      </p>
      <Button variant="primary" type="submit" busy={busy} disabled={disabled || text.trim() === ""}>
        {busy ? t("chat.sending") : t("chat.send")}
      </Button>
    </form>
  );
}
