"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { AppShell } from "@/components/AppShell";
import { useT } from "@/state/PreferencesProvider";
import { UnitScreen } from "@/unit/UnitScreen";

/**
 * Reads the unit id from the query string. The build is a static export, so the
 * page is prerendered without a query and the id appears on the client; the
 * Suspense boundary is what Next needs for that.
 */
function Route() {
  const id = useSearchParams().get("id");
  return <UnitScreen id={id} />;
}

export default function UnitPage() {
  const t = useT();
  return (
    <AppShell>
      <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
        <Suspense fallback={<p role="status">{t("state.loading")}</p>}>
          <Route />
        </Suspense>
      </div>
    </AppShell>
  );
}
