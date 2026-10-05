// Display preferences that live in the browser: motion, theme, language, and
// the retro overlay. They never leave the machine.

export const PREFS_KEY = "lumingo.prefs.v1";

export type MotionSetting = "system" | "on" | "off";
export type Theme = "night" | "day" | "forest" | "ember";
export type ThemeSetting = "system" | Theme;
export type Language = "id" | "en";

export interface Preferences {
  motion: MotionSetting;
  theme: ThemeSetting;
  language: Language;
  /** Scanline overlay. Off unless the learner turns it on. */
  crt: boolean;
}

export interface ResolvedDisplay {
  motion: "on" | "off";
  theme: Theme;
  crt: "on" | "off";
  language: Language;
}

export interface SystemHints {
  prefersReducedMotion: boolean;
  prefersDark: boolean;
  /** `navigator.language`, for the first-run language guess. */
  browserLanguage: string;
}

export const MOTION_SETTINGS: readonly MotionSetting[] = ["system", "on", "off"];
export const THEMES: readonly Theme[] = ["night", "day", "forest", "ember"];
export const THEME_SETTINGS: readonly ThemeSetting[] = ["system", ...THEMES];
export const LANGUAGES: readonly Language[] = ["id", "en"];

/** The first-run guess: Indonesian browsers get Indonesian, everyone else English. */
export function guessLanguage(browserLanguage: string): Language {
  return browserLanguage.toLowerCase().startsWith("id") ? "id" : "en";
}

export function defaultPreferences(browserLanguage = "en"): Preferences {
  return {
    motion: "system",
    theme: "system",
    language: guessLanguage(browserLanguage),
    crt: false,
  };
}

function isOneOf<T extends string>(list: readonly T[], value: unknown): value is T {
  return typeof value === "string" && (list as readonly string[]).includes(value);
}

/** Reads stored preferences. Anything missing or invalid falls back field by field. */
export function parsePreferences(raw: string | null, browserLanguage = "en"): Preferences {
  const defaults = defaultPreferences(browserLanguage);
  if (raw === null) return defaults;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return defaults;
  }
  if (typeof value !== "object" || value === null) return defaults;
  const record = value as Record<string, unknown>;
  return {
    motion: isOneOf(MOTION_SETTINGS, record.motion) ? record.motion : defaults.motion,
    theme: isOneOf(THEME_SETTINGS, record.theme) ? record.theme : defaults.theme,
    language: isOneOf(LANGUAGES, record.language) ? record.language : defaults.language,
    crt: typeof record.crt === "boolean" ? record.crt : defaults.crt,
  };
}

export function resolveMotion(setting: MotionSetting, prefersReducedMotion: boolean): "on" | "off" {
  if (setting === "system") return prefersReducedMotion ? "off" : "on";
  return setting;
}

export function resolveTheme(setting: ThemeSetting, prefersDark: boolean): Theme {
  if (setting === "system") return prefersDark ? "night" : "day";
  return setting;
}

export function resolveDisplay(prefs: Preferences, hints: SystemHints): ResolvedDisplay {
  return {
    motion: resolveMotion(prefs.motion, hints.prefersReducedMotion),
    theme: resolveTheme(prefs.theme, hints.prefersDark),
    crt: prefs.crt ? "on" : "off",
    language: prefs.language,
  };
}

/** Writes the resolved values where the stylesheets and screen readers read them. */
export function applyToDocument(display: ResolvedDisplay, root: HTMLElement): void {
  root.dataset.motion = display.motion;
  root.dataset.theme = display.theme;
  root.dataset.crt = display.crt;
  root.lang = display.language;
}

export function readSystemHints(win: Window): SystemHints {
  return {
    prefersReducedMotion: win.matchMedia("(prefers-reduced-motion: reduce)").matches,
    prefersDark: win.matchMedia("(prefers-color-scheme: dark)").matches,
    browserLanguage: win.navigator.language || "en",
  };
}

/** Storage can be blocked or full. Every access is wrapped, and the page works without it. */
export function loadPreferences(win: Window): Preferences {
  const browserLanguage = win.navigator.language || "en";
  try {
    return parsePreferences(win.localStorage.getItem(PREFS_KEY), browserLanguage);
  } catch {
    return defaultPreferences(browserLanguage);
  }
}

export function savePreferences(win: Window, prefs: Preferences): void {
  try {
    win.localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // Blocked or full: the setting still applies for this visit.
  }
}

/**
 * Runs in <head> before the first paint, so the page never flashes the wrong
 * theme or plays a first animation for someone who turned motion off. It must
 * stay in step with resolveDisplay above, and the test file checks that it does.
 * If anything fails it chooses the still, safe option.
 */
export const BOOT_SCRIPT = `(function(){var d=document.documentElement;try{var s={};try{s=JSON.parse(localStorage.getItem("${PREFS_KEY}")||"{}")||{}}catch(e){}var rm=matchMedia("(prefers-reduced-motion: reduce)").matches;var dk=matchMedia("(prefers-color-scheme: dark)").matches;var m=s.motion==="on"||s.motion==="off"?s.motion:"system";d.dataset.motion=m==="system"?(rm?"off":"on"):m;var t=s.theme==="night"||s.theme==="day"||s.theme==="forest"||s.theme==="ember"?s.theme:"system";d.dataset.theme=t==="system"?(dk?"night":"day"):t;d.dataset.crt=s.crt===true?"on":"off";var l=s.language==="id"||s.language==="en"?s.language:((navigator.language||"en").toLowerCase().indexOf("id")===0?"id":"en");d.lang=l}catch(e){d.dataset.motion="off"}})();`;
