"use client";

import { usePathname } from "next/navigation";
import { useServerState } from "@/api/ServerState";
import { useT } from "@/state/PreferencesProvider";
import { Button, LinkButton } from "@/ui/Button";
import { StatusBanner } from "@/ui/StatusBanner";

/**
 * States of the program that every screen can be in, each with its recovery:
 * the server cannot be reached, the server refused this page, no provider.
 */
export function ConnectionBanner() {
  const t = useT();
  const pathname = usePathname();
  const { status, snapshot, retry } = useServerState();
  const onSetupPage = pathname?.startsWith("/onboarding") ?? false;

  if (status === "refused") {
    return (
      <StatusBanner
        tone="danger"
        title={t("error.refused.title")}
        actions={<Button onClick={() => window.location.reload()}>{t("banner.refused.action")}</Button>}
      >
        <p>{t("error.refused.body")}</p>
      </StatusBanner>
    );
  }
  if (status === "lost") {
    return (
      <StatusBanner tone="danger" title={t("banner.lost.title")} actions={<Button onClick={retry}>{t("banner.lost.action")}</Button>}>
        <p>{t("banner.lost.body")}</p>
      </StatusBanner>
    );
  }
  if (snapshot !== null && snapshot.provider === null && !onSetupPage) {
    return (
      <StatusBanner
        tone="warning"
        title={t("banner.noprovider.title")}
        actions={<LinkButton href="/onboarding/">{t("banner.noprovider.action")}</LinkButton>}
      >
        <p>{t("banner.noprovider.body")}</p>
      </StatusBanner>
    );
  }
  return null;
}
