// Sprites are drawn as text. Each character is one pixel, "." is empty.
//
//   k outline    g gold        G dark gold   w highlight
//   e eye        b blush       m mouth
//   x icon colour (the surrounding text colour)
//   a accent (the theme's primary colour)
//
// Everything here is original artwork made for this project.

export interface SpriteDef {
  readonly rows: readonly string[];
}

function sprite(...rows: string[]): SpriteDef {
  return { rows };
}

// ---- Lumi, the lantern spirit (16 x 16) ------------------------------------

const LUMI_TOP = [
  "................",
  ".....kkkkkk.....",
  "....k......k....",
  "....k......k....",
  "...kkkkkkkkkk...",
  "..kGGGGGGGGGGk..",
  ".kggwwggggggggk.",
  ".kgwwgggggggggk.",
] as const;

const LUMI_BOTTOM = [
  "..kggggggggGGk..",
  "..kGGGGGGGGGGk..",
  "...kkkkkkkkkk...",
  "....kk....kk....",
] as const;

function lumi(face: readonly [string, string, string, string], overrides?: Record<number, string>): SpriteDef {
  const rows = [...LUMI_TOP, ...face, ...LUMI_BOTTOM];
  for (const [index, row] of Object.entries(overrides ?? {})) {
    rows[Number(index)] = row;
  }
  return { rows };
}

export const SPRITES = {
  "lumi-idle": lumi([
    ".kgggeegggeeggk.",
    ".kgggeegggeeggk.",
    ".kgbbggggggbbgk.",
    ".kgggggmmggggGk.",
  ]),
  "lumi-blink": lumi([
    ".kggggggggggggk.",
    ".kgggeegggeeggk.",
    ".kgbbggggggbbgk.",
    ".kgggggmmggggGk.",
  ]),
  "lumi-happy": lumi([
    ".kgggeegggeeggk.",
    ".kgggeegggeeggk.",
    ".kgbbmggggmbbgk.",
    ".kggggmmmmgggGk.",
  ]),
  "lumi-think": lumi(
    [
      ".kgggeegggeeggk.",
      ".kgggeegggeeggk.",
      ".kgbbggggggbbgk.",
      ".kgggmmmmggggGk.",
    ],
    { 1: ".....kkkkkk.x.x." },
  ),
  "lumi-talk": lumi(
    [
      ".kgggeegggeeggk.",
      ".kgggeegggeeggk.",
      ".kgbbggggggbbgk.",
      ".kggggmmmmgggGk.",
    ],
    { 12: "..kggggmmggGGk.." },
  ),
  "lumi-sad": lumi([
    ".kgggeegggeeggk.",
    ".kgggeegggeeggk.",
    ".kgbbggmmggbbgk.",
    ".kggggmggmgggGk.",
  ]),

  // ---- Skills (12 x 12) -------------------------------------------------------
  "skill-listening": sprite(
    "...xxxxxx...",
    "..xx....xx..",
    ".xx......xx.",
    ".x........x.",
    ".x........x.",
    "xxx......xxx",
    "xxx......xxx",
    "xxx......xxx",
    "xxx......xxx",
    "............",
  ),
  "skill-speaking": sprite(
    "....xxxx....",
    "...xxxxxx...",
    "...xxxxxx...",
    "...xxxxxx...",
    "...xxxxxx...",
    ".x.xxxxxx.x.",
    ".x..xxxx..x.",
    "..xx....xx..",
    "...xxxxxx...",
    ".....xx.....",
    ".....xx.....",
    "....xxxx....",
  ),
  "skill-reading": sprite(
    "............",
    "xxxxx..xxxxx",
    "x...x..x...x",
    "x.x.x..x.x.x",
    "x...x..x...x",
    "x.x.x..x.x.x",
    "x...x..x...x",
    "xxxxx..xxxxx",
    "............",
  ),
  "skill-writing": sprite(
    "..........xx",
    ".........xxx",
    "........xxx.",
    ".......xxx..",
    "......xxx...",
    ".....xxx....",
    "....xxx.....",
    "...xxx......",
    "..xxx.......",
    ".xxx........",
    "xx..........",
    "x...........",
  ),

  // ---- Interface icons (12 x 12) ---------------------------------------------------
  "icon-check": sprite(
    "............",
    "..........xx",
    ".........xx.",
    "........xx..",
    "xx.....xx...",
    ".xx...xx....",
    "..xx.xx.....",
    "...xxx......",
    "....x.......",
    "............",
    "............",
    "............",
  ),
  "icon-cross": sprite(
    "............",
    "............",
    "..xx....xx..",
    "...xx..xx...",
    "....xxxx....",
    ".....xx.....",
    "....xxxx....",
    "...xx..xx...",
    "..xx....xx..",
    "............",
    "............",
    "............",
  ),
  "icon-lock": sprite(
    "............",
    "....xxxx....",
    "...xx..xx...",
    "...xx..xx...",
    "...xx..xx...",
    "..xxxxxxxx..",
    "..xxxxxxxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxxxxxxx..",
    "..xxxxxxxx..",
    "............",
  ),
  "icon-star": sprite(
    "............",
    ".....xx.....",
    ".....xx.....",
    "....xxxx....",
    "xxxxxxxxxxxx",
    ".xxxxxxxxxx.",
    "..xxxxxxxx..",
    "...xxxxxx...",
    "..xxx..xxx..",
    "..xx....xx..",
    "............",
    "............",
  ),
  "icon-flame": sprite(
    "......x.....",
    ".....xx.....",
    "....xxx..x..",
    "...xxxx.xx..",
    "..xxxxx.xxx.",
    "..xxxxxxxxx.",
    "..xxxxxxxxx.",
    "..xxxxxxxxx.",
    "...xxxxxxx..",
    "....xxxxx...",
    "............",
    "............",
  ),
  "icon-gem": sprite(
    "............",
    "............",
    "...xxxxxx...",
    "..xxxxxxxx..",
    ".xxxxxxxxxx.",
    "xxxxxxxxxxxx",
    ".xxxxxxxxxx.",
    "..xxxxxxxx..",
    "...xxxxxx...",
    "....xxxx....",
    ".....xx.....",
    "............",
  ),
  "icon-gear": sprite(
    "............",
    ".....xx.....",
    "..x.xxxx.x..",
    "..xxxxxxxx..",
    "...xx..xx...",
    ".xxxx..xxxx.",
    ".xxxx..xxxx.",
    "...xx..xx...",
    "..xxxxxxxx..",
    "..x.xxxx.x..",
    ".....xx.....",
    "............",
  ),
  "icon-map": sprite(
    "..x.........",
    "..xxx.......",
    "..xxxxx.....",
    "..xxxxxxx...",
    "..xxxxxxxxx.",
    "..xxxxxxx...",
    "..xxxxx.....",
    "..xxx.......",
    "..x.........",
    "..x.........",
    "..x.........",
    ".xxxx.......",
  ),
  "icon-chat": sprite(
    "............",
    ".xxxxxxxxxx.",
    "xxxxxxxxxxxx",
    "xxxxxxxxxxxx",
    "xxxxxxxxxxxx",
    "xxxxxxxxxxxx",
    "xxxxxxxxxxxx",
    ".xxxxxxxxxx.",
    "...xxxx.....",
    "....xxx.....",
    ".....x......",
    "............",
  ),
  "icon-play": sprite(
    "............",
    "..xx........",
    "..xxxx......",
    "..xxxxxx....",
    "..xxxxxxxx..",
    "..xxxxxxxxxx",
    "..xxxxxxxxxx",
    "..xxxxxxxx..",
    "..xxxxxx....",
    "..xxxx......",
    "..xx........",
    "............",
  ),
  "icon-pause": sprite(
    "............",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "..xxx..xxx..",
    "............",
    "............",
  ),
  "icon-speaker": sprite(
    "............",
    "....x.......",
    "...xx...x...",
    "xxxxx....x..",
    "xxxxx..x..x.",
    "xxxxx..x..x.",
    "xxxxx..x..x.",
    "xxxxx....x..",
    "...xx...x...",
    "....x.......",
    "............",
    "............",
  ),
  "icon-arrow": sprite(
    "............",
    ".......x....",
    ".......xx...",
    ".xxxxxxxxx..",
    ".xxxxxxxxxx.",
    ".xxxxxxxxx..",
    ".......xx...",
    ".......x....",
    "............",
  ),
} as const satisfies Record<string, SpriteDef>;

export type SpriteName = keyof typeof SPRITES;

export type LumiMood = "idle" | "blink" | "happy" | "think" | "talk" | "sad";
