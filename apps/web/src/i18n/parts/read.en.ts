// English strings for the read screens. Part of the composed dictionary in `en.ts`.
export const readEn = {
  "read.text": "Reading text",
  "read.word": "Word with a meaning: {word}",
  "read.gloss": "{word} means {gloss}",
  "read.gloss.close": "Close the meaning",

  "read.title": "Graded reading",
  "read.intro": "Pick a level and type what you want to read about. The program asks your AI provider for a text with questions. When no text can be written, the reading sets of your units are offered instead.",
  "read.setup.title": "Choose a level and a topic",
  "read.level": "Level",
  "read.level.fixed": "This text is at level {level}. To read at another level, open the reading page again.",
  "read.topic": "What to read about",
  "read.topic.placeholder": "A day at the market",
  "read.topic.hint": "A few words. The text is written for the level above.",
  "read.start": "Get a text",
  "read.starting": "Asking for a text",
  "read.again": "Another text",

  "read.fallback.title": "Texts from your units",
  "read.reason.provider_unavailable": "The AI provider could not be reached, so no new text was written. These texts are already written and need no provider.",
  "read.reason.unusable": "Two texts from the provider did not pass the checks, so neither was shown. These texts are already written and need no provider.",
  "read.empty.title": "Nothing to read at this level yet",
  "read.empty.body": "No unit at this level has a reading set, and no text was written. Nothing was invented to fill the gap. Try another level, or set up an AI provider.",
  "read.set.from": "From unit {unit}",

  "read.generated.label": "Generated text",
  "read.generated.note": "A model wrote this text, so a detail may be wrong.",
  "read.generated.regenerated": "The first text failed a check, so this is the second one.",
  "read.generated.vocabulary.skipped": "The vocabulary check was skipped: no word list is installed.",

  "read.score": "{correct} of {total} correct",
  "read.score.note": "Practice only: this score does not count toward a level estimate.",
} as const;
