"use client";

import { useT } from "@/state/PreferencesProvider";
import { Badge } from "@/ui/Badge";
import { LinkButton } from "@/ui/Button";
import { Panel } from "@/ui/Panel";
import noticesJson from "./notices.json";

/** The kinds the data file holds, in the order the page shows them. */
const KINDS = ["crate", "package", "font", "model", "release"] as const;

export type ComponentKind = (typeof KINDS)[number];

export interface Component {
  name: string;
  kind: ComponentKind;
  licence: string;
  attribution?: string;
}

export interface Model {
  id: string;
  licence: string;
  licenceUrl: string;
  downloadable: boolean;
}

export interface Notices {
  project: { code: string; content: string };
  components: Component[];
  models: Model[];
}

/**
 * What this build ships, read from the file the generator writes at build
 * time. The page shows this file and nothing else: no server call, and no
 * file the release zip does not hold.
 */
export const notices = noticesJson as Notices;

/** The components of one kind. */
function ofKind(kind: ComponentKind): Component[] {
  return notices.components.filter((component) => component.kind === kind);
}

/** The About page: the project's own licence, the components this build ships, the fonts, the models, and the release. */
export function AboutScreen() {
  const t = useT();
  return (
    <div className="px-stack" style={{ "--gap": "var(--space-6)" } as React.CSSProperties}>
      <p>{t("about.intro")}</p>
      <OwnLicence />
      <Components />
      <Fonts />
      <Models />
      <Release />
      <div>
        <LinkButton href="/settings/">{t("about.back")}</LinkButton>
      </div>
    </div>
  );
}

/** The two licences of the project itself: the code and the content. */
function OwnLicence() {
  const t = useT();
  return (
    <Panel title={t("about.own.title")} raised>
      <div className="px-stack">
        <div>
          <p className="px-label">{t("about.own.code")}</p>
          <p>{notices.project.code}</p>
        </div>
        <div>
          <p className="px-label">{t("about.own.content")}</p>
          <p>{notices.project.content}</p>
        </div>
      </div>
    </Panel>
  );
}

/** The third-party components, one group per kind, each group with its count. */
function Components() {
  const t = useT();
  const groups = KINDS.filter((kind) => kind !== "release")
    .map((kind) => ({ kind, items: ofKind(kind) }))
    .filter((group) => group.items.length > 0);
  return (
    <Panel title={t("about.components.title")} raised>
      {groups.length === 0 ? (
        <p>{t("about.components.empty")}</p>
      ) : (
        <div className="px-stack" style={{ "--gap": "var(--space-5)" } as React.CSSProperties}>
          {groups.map((group) => (
            <ComponentGroup key={group.kind} kind={group.kind} items={group.items} />
          ))}
        </div>
      )}
    </Panel>
  );
}

/** One group of components: the kind, how many there are, and the names with their licences. */
function ComponentGroup({ kind, items }: { kind: ComponentKind; items: Component[] }) {
  const t = useT();
  return (
    <section className="px-stack" style={{ "--gap": "var(--space-3)" } as React.CSSProperties}>
      <div className="px-row">
        <h3 className="px-panel__title about__kind">{t(`about.kind.${kind}`)}</h3>
        <Badge tone="info">{t("about.components.count", { count: items.length })}</Badge>
      </div>
      <div className="table-wrap about__scroll">
        <table className="table">
          <thead>
            <tr>
              <th scope="col">{t("about.components.name")}</th>
              <th scope="col">{t("about.components.licence")}</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => (
              <tr key={item.name}>
                <th scope="row">
                  {item.name}
                  {item.attribution ? <div className="px-hint">{item.attribution}</div> : null}
                </th>
                <td>{item.licence}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** The bundled fonts: which licence they carry, and who wrote them. */
function Fonts() {
  const t = useT();
  return (
    <Panel title={t("about.fonts.title")} raised>
      <div className="px-stack">
        <p>{t("about.fonts.note")}</p>
        <p className="px-hint">{t("about.components.attribution")}</p>
      </div>
    </Panel>
  );
}

/** The speech models the manifest names, and whether one of them can be downloaded yet. */
function Models() {
  const t = useT();
  const models = notices.models;
  const downloadable = models.filter((model) => model.downloadable);
  return (
    <Panel title={t("about.models.title")} raised>
      {models.length === 0 ? (
        <p>{t("about.models.empty")}</p>
      ) : (
        <div className="px-stack">
          {downloadable.length === 0 ? <p className="px-hint">{t("about.models.pending")}</p> : null}
          <ul className="rows">
            {models.map((model) => (
              <li key={model.id} className="rows__item">
                <span className="rows__text">
                  {model.id}
                  {model.downloadable ? <span className="px-hint about__note">{t("about.models.downloadable")}</span> : null}
                </span>
                <span className="px-badge">{model.licence}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </Panel>
  );
}

/** The files the release zip holds. */
function Release() {
  const t = useT();
  return (
    <Panel title={t("about.release.title")} raised>
      <ComponentGroup kind="release" items={ofKind("release")} />
    </Panel>
  );
}
