"use client";

import { useState } from "react";
import { AppShell } from "@/components/AppShell";
import { Hud } from "@/game/Hud";
import { QuestMap, type QuestRegion } from "@/learn/QuestMap";
import { SkillBars, type SkillEstimate } from "@/progress/SkillBars";
import { PhonemeChips, type WordResult } from "@/pron/PhonemeChips";
import { SPRITES, type LumiMood, type SpriteName } from "@/sprites/data";
import { Mascot } from "@/sprites/Mascot";
import { Sprite } from "@/sprites/Sprite";
import { Badge } from "@/ui/Badge";
import { Button } from "@/ui/Button";
import { DialogBox } from "@/ui/DialogBox";
import { Meter } from "@/ui/Meter";
import { Panel } from "@/ui/Panel";
import { Segmented } from "@/ui/Segmented";
import { Switch } from "@/ui/Switch";
import { Typewriter } from "@/ui/Typewriter";

// Sample data only. This page exists to look at the design system, and it is not linked from the menu.
const REGIONS: QuestRegion[] = [
  {
    level: "A1",
    nodes: [
      { id: "a1-u01", number: 1, title: "Hello and goodbye", state: "done", skillsPractised: 4 },
      { id: "a1-u02", number: 2, title: "My family", state: "done", skillsPractised: 4 },
      { id: "a1-u03", number: 3, title: "Food I like", state: "current", skillsPractised: 2 },
      { id: "a1-u04", number: 4, title: "My day", state: "open", skillsPractised: 0 },
      { id: "a1-u05", number: 5, title: "Around town", state: "locked", skillsPractised: 0 },
    ],
  },
];

const ESTIMATES: SkillEstimate[] = [
  { skill: "listening", status: "estimate", level: "A1", confidence: "low", scoredTasks: 6, sessions: 3 },
  { skill: "speaking", status: "estimate", level: "A2", confidence: "medium", scoredTasks: 14, sessions: 5 },
  { skill: "reading", status: "estimate", level: "A2", confidence: "high", scoredTasks: 22, sessions: 7 },
  { skill: "writing", status: "insufficient", scoredTasks: 1, sessions: 1, moreTasksNeeded: 2 },
];

const WORDS: WordResult[] = [
  {
    text: "think",
    phonemes: [
      { symbol: "θ", tone: "off", heard: "s" },
      { symbol: "ɪ", tone: "good" },
      { symbol: "ŋ", tone: "close" },
      { symbol: "k", tone: "good" },
    ],
  },
];

const MOODS: LumiMood[] = ["idle", "blink", "happy", "think", "talk", "sad"];
const SWATCHES = [
  "bg", "bg-deep", "surface", "surface-raised", "border", "text", "text-muted", "primary", "success", "danger",
  "warning", "info", "skill-listening", "skill-speaking", "skill-reading", "skill-writing",
];

export default function StyleguidePage() {
  const [on, setOn] = useState(true);
  const [choice, setChoice] = useState<"a" | "b" | "c">("a");
  const icons = Object.keys(SPRITES).filter((name) => !name.startsWith("lumi-")) as SpriteName[];

  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
        <h1>Style guide</h1>

        <Panel title="Colour tokens" raised>
          <ul className="sg-swatches">
            {SWATCHES.map((name) => (
              <li key={name} className="sg-swatch">
                <span className="sg-swatch__chip" style={{ background: `var(--color-${name})` }} />
                <code>{name}</code>
              </li>
            ))}
          </ul>
        </Panel>

        <Panel title="Type" raised>
          <div className="px-stack">
            <h1>Display one</h1>
            <h2>Display two</h2>
            <h3>Display three</h3>
            <p>Body text in Pixelify Sans: the quick brown fox jumps over the lazy dog. 0123456789</p>
            <p className="phonetic">/θɪŋk/ /ʃɪp/ /ðɛn/ /ˈwɔtər/ /ə/ /ŋ/ (Noto Sans Mono, the only font with IPA)</p>
          </div>
        </Panel>

        <Panel title="Buttons and fields" raised>
          <div className="px-stack">
            <div className="px-row">
              <Button variant="primary">Start</Button>
              <Button>Default</Button>
              <Button variant="danger">Delete</Button>
              <Button variant="ghost">Ghost</Button>
              <Button small>Small</Button>
              <Button busy>Busy</Button>
              <Button disabled>Disabled</Button>
            </div>
            <div>
              <label className="px-label" htmlFor="sg-text">
                Text field
              </label>
              <input id="sg-text" className="px-field" placeholder="Type here" />
            </div>
            <Switch label="A switch" checked={on} onChange={setOn} onText="ON" offText="OFF" hint="State is written in the track." />
            <Segmented
              legend="A segmented control"
              value={choice}
              onChange={setChoice}
              options={[
                { value: "a", label: "One" },
                { value: "b", label: "Two" },
                { value: "c", label: "Three" },
              ]}
            />
          </div>
        </Panel>

        <Panel title="Meters, badges, panels" raised>
          <div className="px-stack">
            <Meter value={35} label="Plain" />
            <Meter value={70} label="Listening" tone="listening" />
            <Meter value={55} label="Speaking" tone="speaking" />
            <Meter value={80} label="Reading" tone="reading" />
            <Meter value={20} label="Writing" tone="writing" />
            <div className="px-row">
              <Badge>Plain</Badge>
              <Badge tone="estimate">Estimate</Badge>
              <Badge tone="success">Done</Badge>
              <Badge tone="danger">Problem</Badge>
              <Badge tone="info">Info</Badge>
            </div>
            <div className="px-grid" style={{ "--min": "10rem" } as React.CSSProperties}>
              <Panel tone="listening">Listening</Panel>
              <Panel tone="speaking">Speaking</Panel>
              <Panel tone="reading">Reading</Panel>
              <Panel tone="writing">Writing</Panel>
            </div>
          </div>
        </Panel>

        <Panel title="Lumi and the tutor box" raised>
          <div className="px-stack">
            <div className="px-row">
              {MOODS.map((mood) => (
                <figure key={mood} className="sg-figure">
                  <Mascot mood={mood} scale={5} label={`Lumi, ${mood}`} />
                  <figcaption>{mood}</figcaption>
                </figure>
              ))}
            </div>
            <DialogBox name="Lumi" more>
              <Typewriter text="Hello! Today we talk about food. What do you like to eat?" />
            </DialogBox>
          </div>
        </Panel>

        <Panel title="Icons" raised>
          <ul className="sg-icons">
            {icons.map((name) => (
              <li key={name} className="sg-icon">
                <Sprite name={name} scale={3} />
                <code>{name}</code>
              </li>
            ))}
          </ul>
        </Panel>

        <Panel title="HUD" raised>
          <Hud streak={5} sparks={240} rank={2} rankProgress={40} restTokens={1} />
        </Panel>

        <Panel title="Skill profile" raised>
          <SkillBars estimates={ESTIMATES} />
        </Panel>

        <Panel title="Pronunciation" raised>
          <PhonemeChips words={WORDS} />
        </Panel>

        <QuestMap regions={REGIONS} />
      </div>
    </AppShell>
  );
}
