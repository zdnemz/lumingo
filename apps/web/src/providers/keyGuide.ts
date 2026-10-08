import type { ProviderProtocol } from "@/generated/ProviderProtocol";

/**
 * The provider the key guide walks through: one with a free tier that speaks
 * the OpenAI-compatible protocol.
 *
 * `statedOn` is the date the data-use sentences in the dictionaries were
 * written. They paraphrase the provider's published terms and the program
 * cannot check them, so the owner re-reads the terms and moves this date
 * whenever the sentences are confirmed or changed. The screen prints the date.
 */
export const KEY_GUIDE = {
  id: "google-ai-studio",
  /** Shown as a link. Following it is the learner's own navigation, not a request made by this page. */
  url: "https://aistudio.google.com/",
  statedOn: "2026-10-06",
  profile: {
    name: "gemini-free",
    protocol: "openai_chat" satisfies ProviderProtocol,
    base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
    model: "gemini-flash-latest",
  },
} as const;

/** Values the key guide can put into the provider form. Never a key. */
export interface ProviderPreset {
  name: string;
  protocol: ProviderProtocol;
  base_url: string;
  model: string;
}

export const KEY_GUIDE_PRESET: ProviderPreset = KEY_GUIDE.profile;
