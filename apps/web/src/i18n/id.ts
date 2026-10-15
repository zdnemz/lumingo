import type { MessageKey } from "./en";
import { shellId } from "./parts/shell.id";
import { unitId } from "./parts/unit.id";
import { progressId } from "./parts/progress.id";
import { settingsId } from "./parts/settings.id";
import { talkId } from "./parts/talk.id";
import { writeId } from "./parts/write.id";
import { readId } from "./parts/read.id";

// Indonesian strings. Same keys as `en.ts`; the type below makes a missing key a compile error.
export const id = {
  ...shellId,
  ...unitId,
  ...progressId,
  ...settingsId,
  ...talkId,
  ...writeId,
  ...readId,
} as const satisfies Record<MessageKey, string>;
