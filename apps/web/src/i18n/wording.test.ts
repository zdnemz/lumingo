import { describe, expect, it } from "vitest";
import { dictionaries } from "./index";

/**
 * The wording rules for anything that talks about a level or an estimate:
 * `docs/ASSESSMENT_SPEC.md` section 12 ("what the UI may show") and `PRD.md`
 * section 12.4 ("allowed and banned wording"). Every string of both
 * dictionaries is scanned, so a banned phrase cannot arrive in a string that
 * a screen test never renders.
 */

interface Banned {
  /** Where the rule comes from, so a failure explains itself. */
  rule: string;
  en: RegExp;
  id: RegExp;
}

const BANNED: readonly Banned[] = [
  { rule: 'ASSESSMENT_SPEC 12: "Your level is A2" after placement', en: /\byour (current |working )?level is\b/i, id: /\b(level|tingkat)(mu| kamu| anda)( saat ini)? adalah\b/i },
  { rule: 'ASSESSMENT_SPEC 12: a percentage "CEFR score"', en: /\bcefr (score|points?|percent)/i, id: /\b(skor|nilai|poin) cefr\b/i },
  { rule: "ASSESSMENT_SPEC 12: certification", en: /certif/i, id: /sertifi/i },
  { rule: "ASSESSMENT_SPEC 12 and PRD 12.4: official CEFR level", en: /\bofficial/i, id: /\bresmi\b/i },
  { rule: "ASSESSMENT_SPEC 12: exam equivalence", en: /equivalen|\bexams?\b|\bielts\b|\btoefl\b|\btoeic\b/i, id: /setara|ekuivalen|sepadan|\bujian\b|\bielts\b|\btoefl\b|\btoeic\b/i },
  { rule: "PRD 12.4: fully offline, fully local AI, no cloud", en: /fully (offline|local)|\bno cloud\b|100 ?% (offline|local)/i, id: /sepenuhnya (offline|lokal|luring)|tanpa cloud|tanpa awan/i },
  { rule: 'PRD 12.4: "private" without the sentence next to it', en: /\bprivate\b|\bprivacy-first\b/i, id: /\bpribadi\b|\brahasia\b/i },
  { rule: "PRD 12.4: accurate pronunciation scoring", en: /accurate pronunciation|pronunciation (is )?accurate|precise pronunciation/i, id: /pelafalan (yang )?(akurat|tepat sekali)|penilaian pelafalan (yang )?akurat/i },
  { rule: "PRD 12.4: securely stored, encrypted, encrypted at rest", en: /\bsecure(ly)?\b|encrypt/i, id: /\baman\b|terenkripsi|dienkripsi|enkripsi/i },
  { rule: "PRD 12.4: website, online app, hosted", en: /\bwebsite\b|\bweb site\b|online app|\bhosted\b|\bhosting\b|\bonline\b/i, id: /situs web|\bwebsite\b|aplikasi online|di-?host|\bhosting\b|\bdaring\b/i },
];

type Table = Record<string, string>;
const tables: readonly (readonly ["en" | "id", Table])[] = [
  ["en", dictionaries.en],
  ["id", dictionaries.id],
];

describe("banned wording (ASSESSMENT_SPEC section 12 and PRD section 12.4)", () => {
  for (const banned of BANNED) {
    it(banned.rule, () => {
      const hits: string[] = [];
      for (const [language, table] of tables) {
        const pattern = language === "en" ? banned.en : banned.id;
        for (const [key, value] of Object.entries(table)) {
          if (pattern.test(value)) hits.push(`${language}:${key} = ${value}`);
        }
      }
      expect(hits).toEqual([]);
    });
  }

  it("catches each banned phrase when it is written", () => {
    const samples: Array<[string, "en" | "id"]> = [
      ["Your level is A2", "en"],
      ["Tingkatmu adalah A2", "id"],
      ["A CEFR score of 70", "en"],
      ["skor CEFR 70", "id"],
      ["This is a certified result", "en"],
      ["Sudah bersertifikat", "id"],
      ["Official CEFR level", "en"],
      ["Level resmi", "id"],
      ["Equivalent to IELTS 6", "en"],
      ["Setara dengan TOEFL", "id"],
      ["Works fully offline", "en"],
      ["Sepenuhnya offline", "id"],
      ["Your data is private", "en"],
      ["Datamu pribadi", "id"],
      ["Accurate pronunciation scoring", "en"],
      ["Your key is securely stored", "en"],
      ["Kunci terenkripsi", "id"],
      ["Visit the website", "en"],
      ["Aplikasi online", "id"],
    ];
    for (const [text, language] of samples) {
      const hit = BANNED.some((banned) => (language === "en" ? banned.en : banned.id).test(text));
      expect(hit, text).toBe(true);
    }
  });
});

/** A phrase the specs allow, and the key that must carry it. */
interface Allowed {
  source: string;
  key: keyof typeof dictionaries.en;
  /** What the English text must contain. */
  en: string;
  /** What the Indonesian text must contain. */
  id: string;
}

const ALLOWED: readonly Allowed[] = [
  {
    source: 'ASSESSMENT_SPEC 12: "Speaking: A2 (estimate, medium confidence), based on 14 scored tasks in 5 sessions"',
    key: "skill.estimate",
    en: "{skill}: {level} (estimate, {confidence} confidence), based on {tasks} scored tasks in {sessions} sessions",
    id: "{skill}: {level} (perkiraan, keyakinan {confidence}), berdasarkan {tasks} tugas yang dinilai dalam {sessions} sesi",
  },
  {
    source: "ASSESSMENT_SPEC 12: the same sentence when the number of sessions is not known",
    key: "skill.estimate.basis",
    en: "{skill}: {level} (estimate, {confidence} confidence), {basis}",
    id: "{skill}: {level} (perkiraan, keyakinan {confidence}), {basis}",
  },
  { source: "ASSESSMENT_SPEC 12: scored tasks, singular", key: "skill.basis.one", en: "based on 1 scored task", id: "berdasarkan 1 tugas yang dinilai" },
  { source: "ASSESSMENT_SPEC 12: scored tasks", key: "skill.basis.many", en: "based on {tasks} scored tasks", id: "berdasarkan {tasks} tugas yang dinilai" },
  {
    source: 'ASSESSMENT_SPEC 12: "Not enough evidence yet for writing. Two more writing tasks will help."',
    key: "skill.insufficient",
    en: "Not enough evidence yet for {skill}. {more} more {skill} tasks will help.",
    id: "Belum cukup bukti untuk {skill}. {more} tugas {skill} lagi akan membantu.",
  },
  {
    source: "ASSESSMENT_SPEC 12: the same when the number of tasks still needed is not known",
    key: "skill.insufficient.general",
    en: "Not enough evidence yet for {skill}.",
    id: "Belum cukup bukti untuk {skill}.",
  },
  {
    source: 'ASSESSMENT_SPEC 12: "Suggested starting level: A2" after placement',
    key: "skill.placement",
    en: "Suggested starting level: {level}",
    id: "Level awal yang disarankan: {level}",
  },
  {
    source: 'ASSESSMENT_SPEC 12: "Pronunciation feedback is experimental"',
    key: "pron.experimental",
    en: "Pronunciation feedback is experimental",
    id: "Umpan balik pelafalan masih eksperimental",
  },
  {
    source: 'PRD 12.4: "Estimated level, based on your recorded work."',
    key: "progress.estimated.note",
    en: "Estimated level, based on your recorded work.",
    id: "Perkiraan level, berdasarkan hasil kerja yang tercatat.",
  },
  { source: "ASSESSMENT_SPEC 9.3: confidence word low", key: "skill.confidence.low", en: "low", id: "rendah" },
  { source: "ASSESSMENT_SPEC 9.3: confidence word medium", key: "skill.confidence.medium", en: "medium", id: "sedang" },
  { source: "ASSESSMENT_SPEC 9.3: confidence word high", key: "skill.confidence.high", en: "high", id: "tinggi" },
  { source: "ASSESSMENT_SPEC 12: a list of the attempts and quotes behind an estimate", key: "skill.evidence", en: "See the work behind this estimate", id: "Lihat hasil kerja di balik perkiraan ini" },
  { source: 'PRD 12.4: "Your voice stays on your device."', key: "privacy.voice", en: "Your voice stays on your device.", id: "Suaramu tetap di perangkatmu." },
  { source: 'PRD 12.4: "Text is sent to the AI provider you choose."', key: "privacy.text", en: "Text is sent to the AI provider you choose.", id: "Teks dikirim ke penyedia AI yang kamu pilih." },
  {
    source: 'PRD 12.4: "Your API key is saved in plain text in a file on this computer."',
    key: "privacy.key",
    en: "Your API key is saved in plain text in a file on this computer",
    id: "Kunci API-mu disimpan sebagai teks biasa di sebuah berkas di komputer ini",
  },
  { source: 'PRD 12.4: "Runs on your own computer and opens in your browser."', key: "privacy.local", en: "Runs on your own computer and opens in your browser.", id: "Berjalan di komputermu sendiri dan terbuka di peramban." },
];

describe("allowed wording (ASSESSMENT_SPEC section 12 and PRD section 12.4)", () => {
  for (const allowed of ALLOWED) {
    it(allowed.source, () => {
      expect(dictionaries.en[allowed.key], `en:${allowed.key} is missing`).toBeDefined();
      expect(dictionaries.en[allowed.key]).toContain(allowed.en);
      expect(dictionaries.id[allowed.key], `id:${allowed.key} is missing`).toBeDefined();
      expect(dictionaries.id[allowed.key]).toContain(allowed.id);
      expect(dictionaries.id[allowed.key]).not.toBe(dictionaries.en[allowed.key]);
    });
  }
});

/**
 * ASSESSMENT_SPEC 12: "A level without the word estimate" is not allowed. The
 * keys that belong to the profile, the evidence drill-down and the game are
 * checked: any string with a CEFR level or a `{level}` slot must also say it
 * is an estimate or a suggested starting level. Curriculum labels such as
 * "Region A1" on the quest map name a unit's level, not the learner's, and
 * live under other prefixes.
 */
const LEVEL_PREFIXES = ["skill.", "progress.", "game.", "wardrobe.", "cosmetic."];
const HAS_LEVEL = /\b(pre-)?[ABC][12]\b|\{level\}/;
const SAYS_ESTIMATE = /\bestimate|\bperkiraan|suggested starting level|level awal yang disarankan/i;

describe("a level is always called an estimate", () => {
  for (const [language, table] of tables) {
    it(`${language}: every profile, evidence and game string with a level says estimate`, () => {
      const bad = Object.entries(table)
        .filter(([key]) => LEVEL_PREFIXES.some((prefix) => key.startsWith(prefix)))
        .filter(([, value]) => HAS_LEVEL.test(value) && !SAYS_ESTIMATE.test(value))
        .map(([key, value]) => `${language}:${key} = ${value}`);
      expect(bad).toEqual([]);
    });
  }

  it("sees a bare level when one is written", () => {
    expect(HAS_LEVEL.test("Speaking: A2")).toBe(true);
    expect(SAYS_ESTIMATE.test("Speaking: A2")).toBe(false);
  });
});
