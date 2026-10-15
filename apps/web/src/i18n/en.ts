// English strings. `id.ts` must define exactly the same keys, and the tests
// check that, and check the wording rules of the product (no "certified",
// no "encrypted", and so on).
//
// The strings live in `parts/<area>.en.ts`, one pair per screen area, so two
// screens can be written at the same time without editing the same file. This
// file only composes them: the `as const` spread keeps the literal key union,
// so `MessageKey` and `id.ts`'s `satisfies Record<MessageKey, string>` still
// fail the build when a key is missing from one language.
import { shellEn } from "./parts/shell.en";
import { unitEn } from "./parts/unit.en";
import { progressEn } from "./parts/progress.en";
import { settingsEn } from "./parts/settings.en";
import { talkEn } from "./parts/talk.en";
import { writeEn } from "./parts/write.en";
import { readEn } from "./parts/read.en";

export const en = {
  ...shellEn,
  ...unitEn,
  ...progressEn,
  ...settingsEn,
  ...talkEn,
  ...writeEn,
  ...readEn,
} as const;

export type MessageKey = keyof typeof en;
