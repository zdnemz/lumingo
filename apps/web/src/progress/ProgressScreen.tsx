"use client";

import { useCallback, useMemo, useRef, useState } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import { GamePanel } from "@/game/GamePanel";
import type { ProgressOverview } from "@/generated/ProgressOverview";
import type { UnitList } from "@/generated/UnitList";
import { usePreferences, useT } from "@/state/PreferencesProvider";
import { useResource } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import { StatusBanner } from "@/ui/StatusBanner";
import type { ProfileSkill } from "./model";
import {
  ErrorPatterns,
  HistoryPanel,
  PronunciationPanel,
  RecentPractice,
  ReviewQueue,
  UnitsPanel,
  type UnitTitles,
} from "./ProgressPanels";
import { SkillDrillDown } from "./SkillDrillDown";
import { SkillProfile } from "./SkillProfile";

function isEmpty(overview: ProgressOverview): boolean {
  return (
    overview.estimates.length === 0 &&
    overview.recent_sessions.length === 0 &&
    overview.errors.length === 0 &&
    overview.reviews_due.length === 0 &&
    overview.units.length === 0
  );
}

/** Unit titles in the learner's language. A unit list that failed to load leaves the map empty and the ids show. */
function titlesOf(list: UnitList | null, language: "en" | "id"): UnitTitles {
  const titles = new Map<string, string>();
  for (const unit of list?.units ?? []) {
    titles.set(unit.id, language === "id" ? (unit.title.id ?? unit.title.en) : unit.title.en);
  }
  return titles;
}

function Assessment({ overview, units }: { overview: ProgressOverview; units: UnitList | null }) {
  const t = useT();
  const { display } = usePreferences();
  const [open, setOpen] = useState<ProfileSkill | null>(null);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const titles = useMemo(() => titlesOf(units, display.language), [units, display.language]);

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
      {isEmpty(overview) ? (
        <StatusBanner tone="info" title={t("progress.empty.title")}>
          <p>{t("progress.empty.body")}</p>
        </StatusBanner>
      ) : null}

      <Panel title={t("skill.profile")} raised>
        <SkillProfile
          estimates={overview.estimates}
          open={open}
          onOpen={(skill, button) => {
            trigger.current = button;
            setOpen(skill);
          }}
        />
      </Panel>

      {open !== null ? (
        <SkillDrillDown
          skill={open}
          estimate={overview.estimates.find((entry) => entry.skill === open)}
          onClose={() => {
            setOpen(null);
            trigger.current?.focus();
          }}
        />
      ) : null}

      <PronunciationPanel reviews={overview.reviews_due} />
      <HistoryPanel estimates={overview.estimates} />
      <ErrorPatterns errors={overview.errors} />
      <ReviewQueue overview={overview} titles={titles} />
      <UnitsPanel units={overview.units} titles={titles} />
      <RecentPractice sessions={overview.recent_sessions} titles={titles} />
    </div>
  );
}

/**
 * The progress page: the four-skill profile, the way down to the evidence,
 * history, error patterns and the review queue, from `GET /api/progress`
 * and `GET /api/units`; then the game, which is a separate panel with its own
 * request so it can neither hide nor stand in for the assessment.
 */
export function ProgressScreen() {
  const t = useT();
  const api = useApi();
  const loadProgress = useCallback((signal: AbortSignal) => api.getProgress(signal), [api]);
  const loadUnits = useCallback((signal: AbortSignal) => api.listUnits(signal), [api]);
  const progress = useResource<ProgressOverview>(loadProgress);
  const units = useResource<UnitList>(loadUnits);
  const reloadProgress = progress.refresh;
  const reloadUnits = units.refresh;
  const state = progress.state;

  return (
    <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
      <div className="px-stack" style={{ "--gap": "var(--space-2)" } as React.CSSProperties}>
        <div className="px-row">
          <p>{t("progress.estimated.note")}</p>
          <Button
            onClick={() => {
              reloadProgress();
              reloadUnits();
            }}
          >
            {t("progress.refresh")}
          </Button>
        </div>
        <p className="px-hint">{t("progress.limits")}</p>
      </div>

      <section className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties} aria-labelledby="assessment-title">
        <h2 id="assessment-title">{t("progress.assessment.title")}</h2>
        {state.status === "loading" ? <p role="status">{t("state.loading")}</p> : null}
        {state.status === "error" ? <ErrorBanner error={state.error} onRetry={progress.reload} /> : null}
        {state.status === "ready" ? <Assessment overview={state.data} units={units.state.status === "ready" ? units.state.data : null} /> : null}
      </section>

      <GamePanel />
    </div>
  );
}
