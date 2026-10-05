import { en, type MessageKey } from "./en";
import { id } from "./id";
import type { Language } from "@/state/preferences";

export type { MessageKey };

export const dictionaries: Record<Language, Record<MessageKey, string>> = { en, id };

export type TranslateParams = Record<string, string | number>;

/** Looks a string up and fills `{name}` slots. Unknown slots stay visible so a typo is easy to spot. */
export function translate(language: Language, key: MessageKey, params?: TranslateParams): string {
  const template = dictionaries[language][key];
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = params[name];
    return value === undefined ? whole : String(value);
  });
}
