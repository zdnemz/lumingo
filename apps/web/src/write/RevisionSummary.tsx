"use client";

import { useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";

export interface RevisionSummaryProps {
  fixed: number;
  remaining: number;
  added: number;
}

/** How the second draft compares with the first. Counts are words, and each count has its own label. */
export function RevisionSummary({ fixed, remaining, added }: RevisionSummaryProps) {
  const t = useT();
  return (
    <section className="revision" aria-label={t("write.revision")}>
      <h3>{t("write.revision")}</h3>
      <div className="px-row">
        <Badge tone="success">{t("write.fixed", { count: fixed })}</Badge>
        <Badge tone="danger">{t("write.remaining", { count: remaining })}</Badge>
        <Badge tone="info">{t("write.new", { count: added })}</Badge>
      </div>
    </section>
  );
}
