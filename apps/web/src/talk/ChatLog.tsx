"use client";

import { useEffect, useRef } from "react";
import { Mascot } from "@/sprites/Mascot";
import { useT } from "@/state/PreferencesProvider";
import { DialogBox } from "@/ui/DialogBox";

export interface ChatMessage {
  id: string;
  role: "tutor" | "learner";
  /** Plain text. It is always rendered as text, never as HTML. */
  text: string;
  /** The tutor's reply is still arriving. */
  streaming?: boolean;
  /** The learner fixed what the recogniser heard. */
  edited?: boolean;
}

export interface ChatLogProps {
  messages: readonly ChatMessage[];
  /** The tutor has not started its reply yet. */
  thinking?: boolean;
}

/**
 * The conversation. Tutor messages appear in Lumi's dialogue box and the
 * learner's in a plain panel. A message is announced to screen readers once it
 * is complete, not word by word.
 */
export function ChatLog({ messages, thinking }: ChatLogProps) {
  const t = useT();
  const end = useRef<HTMLDivElement>(null);
  const last = messages.at(-1);

  useEffect(() => {
    end.current?.scrollIntoView?.({ block: "end" });
  }, [messages.length, last?.text.length]);

  const announcement =
    last && last.role === "tutor" && !last.streaming ? `${t("chat.new-message")}: ${last.text}` : "";

  return (
    <div className="chat" role="log" aria-label={t("chat.log")} aria-busy={last?.streaming || thinking || undefined}>
      <ol className="chat__list">
        {messages.map((message) => (
          <li key={message.id} className={`chat__item chat__item--${message.role}`}>
            {message.role === "tutor" ? (
              <div className="chat__tutor">
                <Mascot mood={message.streaming ? "talk" : "idle"} scale={3} />
                <DialogBox name={t("chat.tutor")}>
                  <span>{message.text}</span>
                  {message.streaming ? <span className="chat__caret" data-anim="blink" aria-hidden="true" /> : null}
                </DialogBox>
              </div>
            ) : (
              <div className="chat__learner px-panel px-panel--inset">
                <p className="chat__who">
                  {t("chat.you")}
                  {message.edited ? <span className="chat__edited"> ({t("chat.edited")})</span> : null}
                </p>
                <p>{message.text}</p>
              </div>
            )}
          </li>
        ))}
        {thinking ? (
          <li className="chat__item chat__item--tutor">
            <div className="chat__tutor">
              <Mascot mood="think" scale={3} />
              <p className="chat__thinking">{t("chat.thinking")}</p>
            </div>
          </li>
        ) : null}
      </ol>
      <p className="sr-only" aria-live="polite">
        {announcement}
      </p>
      <div ref={end} />
    </div>
  );
}
