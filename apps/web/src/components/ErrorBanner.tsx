"use client";

import { ApiError, isUnreachable } from "@/api/client";
import type { ErrorCode } from "@/generated/ErrorCode";
import type { MessageKey } from "@/i18n";
import { useT } from "@/state/PreferencesProvider";
import { Button } from "@/ui/Button";
import { StatusBanner } from "@/ui/StatusBanner";

const CODE_TEXT: Record<ErrorCode | "unknown", MessageKey> = {
  not_found: "error.code.not_found",
  invalid_input: "error.code.invalid_input",
  conflict: "error.code.conflict",
  read_only: "error.code.read_only",
  busy: "error.code.busy",
  not_available: "error.code.not_available",
  licence_not_accepted: "error.code.licence_not_accepted",
  provider_not_configured: "error.code.provider_not_configured",
  shutting_down: "error.code.shutting_down",
  storage: "error.code.storage",
  internal: "error.code.internal",
  unknown: "error.code.unknown",
};

function textFor(code: string): MessageKey {
  return code in CODE_TEXT ? CODE_TEXT[code as ErrorCode] : CODE_TEXT.unknown;
}

export interface ErrorBannerProps {
  error: unknown;
  /** Runs the failed thing again. Shown as the recovery action for every error. */
  onRetry?: () => void;
}

/**
 * Turns whatever a request threw into a banner with a recovery action:
 * "the program is not answering", "the program refused this page", or the
 * program's own coded answer, with its English sentence on a details line.
 */
export function ErrorBanner({ error, onRetry }: ErrorBannerProps) {
  const t = useT();
  if (isUnreachable(error)) {
    return (
      <StatusBanner
        tone="danger"
        title={t("error.unreachable.title")}
        actions={onRetry ? <Button onClick={onRetry}>{t("state.retry")}</Button> : undefined}
      >
        <p>{t("error.unreachable.body")}</p>
      </StatusBanner>
    );
  }
  if (error instanceof ApiError && error.refused) {
    return (
      <StatusBanner
        tone="danger"
        title={t("error.refused.title")}
        actions={<Button onClick={() => window.location.reload()}>{t("state.reload")}</Button>}
      >
        <p>{t("error.refused.body")}</p>
      </StatusBanner>
    );
  }
  const code = error instanceof ApiError ? error.code : "unknown";
  const detail = error instanceof ApiError ? error.detail : "";
  return (
    <StatusBanner
      tone="danger"
      title={t(textFor(code))}
      actions={onRetry ? <Button onClick={onRetry}>{t("state.retry")}</Button> : undefined}
    >
      {detail ? <p className="px-hint">{t("state.details", { text: detail })}</p> : null}
    </StatusBanner>
  );
}
