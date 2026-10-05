"use client";

import { useEventStream, type StreamStatus } from "@/api/useEventStream";
import type { ServerEvent } from "@/generated/ServerEvent";
import type { MessageKey } from "@/i18n";
import { Sprite } from "@/sprites/Sprite";
import { useT } from "@/state/PreferencesProvider";
import { Panel } from "@/ui/Panel";

const STATUS_TEXT: Record<StreamStatus, MessageKey> = {
  connecting: "link.state.connecting",
  live: "link.state.live",
  lost: "link.state.lost",
  refused: "link.state.refused",
};

const STATUS_ICON = {
  connecting: "icon-gear",
  live: "icon-check",
  lost: "icon-cross",
  refused: "icon-lock",
} as const;

function describe(event: ServerEvent): string {
  return event.type === "Snapshot" ? `#${event.seq} Snapshot` : `#${event.seq} Heartbeat`;
}

/** Shows the live event stream from the local server. This is what S0-08 proves works end to end. */
export function LinkStatus() {
  const t = useT();
  const { status, snapshot, events } = useEventStream();
  const latestUptime = events.find((event) => event.type === "Heartbeat");
  const uptimeMs = latestUptime?.type === "Heartbeat" ? latestUptime.uptime_ms : snapshot?.uptime_ms;
  const tone = status === "live" ? "success" : status === "connecting" ? "primary" : "danger";

  return (
    <Panel title={t("link.title")} tone={tone} raised>
      <div className="px-stack">
        <p>{t("link.intro")}</p>
        <p className="px-row" role="status">
          <Sprite name={STATUS_ICON[status]} scale={3} />
          <strong>{t(STATUS_TEXT[status])}</strong>
        </p>
        {snapshot ? (
          <dl className="link__facts">
            <div>
              <dt>{t("link.version")}</dt>
              <dd>{snapshot.server_version}</dd>
            </div>
            <div>
              <dt>{t("link.mode")}</dt>
              <dd>{snapshot.dev_mode ? t("link.mode.dev") : t("link.mode.normal")}</dd>
            </div>
            <div>
              <dt>{t("link.uptime")}</dt>
              <dd>{t("link.seconds", { count: Math.floor((uptimeMs ?? 0) / 1000) })}</dd>
            </div>
          </dl>
        ) : null}
        <div>
          <h3>{t("link.events")}</h3>
          {events.length === 0 ? (
            <p className="px-hint">{t("link.events.empty")}</p>
          ) : (
            <ol className="link__events">
              {events.map((event) => (
                <li key={event.seq} className="phonetic">
                  {describe(event)}
                </li>
              ))}
            </ol>
          )}
        </div>
      </div>
    </Panel>
  );
}
