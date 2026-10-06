"use client";

import { useState, type FormEvent, type Ref } from "react";
import { useApi } from "@/api/ApiProvider";
import { ErrorBanner } from "@/components/ErrorBanner";
import type { ProviderInfo } from "@/generated/ProviderInfo";
import type { ProviderProtocol } from "@/generated/ProviderProtocol";
import type { SaveProviderRequest } from "@/generated/SaveProviderRequest";
import { useT } from "@/state/PreferencesProvider";
import { useAction } from "@/state/useResource";
import { Button } from "@/ui/Button";
import { Segmented } from "@/ui/Segmented";
import { Switch } from "@/ui/Switch";
import { TextField } from "@/ui/TextField";
import type { ProviderPreset } from "./keyGuide";

const PROTOCOLS: readonly ProviderProtocol[] = ["openai_chat", "anthropic_messages"];
const MAX_NAME_CHARS = 64;

export interface ProviderFormProps {
  /** A saved profile to change. Its name is fixed, because saving under the same name replaces it. */
  initial?: ProviderInfo;
  /** Values to start from, such as the key guide's. Never a key. */
  preset?: ProviderPreset;
  /** Whether "use this profile" starts on. */
  makeActiveDefault: boolean;
  onSaved: (saved: ProviderInfo) => void;
  onCancel?: () => void;
  /** Lets the screen move focus to the key field, for the wrong-key recovery. */
  keyRef?: Ref<HTMLInputElement>;
}

interface Values {
  name: string;
  protocol: ProviderProtocol;
  baseUrl: string;
  model: string;
}

function startValues(initial: ProviderInfo | undefined, preset: ProviderPreset | undefined): Values {
  if (initial) {
    return { name: initial.name, protocol: initial.protocol, baseUrl: initial.base_url, model: initial.model };
  }
  return {
    name: preset?.name ?? "",
    protocol: preset?.protocol ?? "openai_chat",
    baseUrl: preset?.base_url ?? "",
    model: preset?.model ?? "",
  };
}

/**
 * Adds a provider profile or changes a saved one. The key lives in this form's
 * state only until the request is sent: after a successful save the field is
 * emptied, and nothing on the page shows the key again, only `key_last4`.
 */
export function ProviderForm({ initial, preset, makeActiveDefault, onSaved, onCancel, keyRef }: ProviderFormProps) {
  const t = useT();
  const api = useApi();
  const action = useAction();
  const [values, setValues] = useState<Values>(() => startValues(initial, preset));
  const [appliedPreset, setAppliedPreset] = useState(preset);
  const [key, setKey] = useState("");
  const [clearKey, setClearKey] = useState(false);
  const [makeActive, setMakeActive] = useState(makeActiveDefault);
  const [attempted, setAttempted] = useState(false);

  // A new preset from the key guide replaces the typed values once, while the page renders.
  if (preset !== appliedPreset) {
    setAppliedPreset(preset);
    if (preset && !initial) setValues(startValues(undefined, preset));
  }

  const editing = initial !== undefined;
  const name = values.name.trim();
  const errors = {
    name: name.length === 0 || [...name].length > MAX_NAME_CHARS ? t("provider.error.name") : undefined,
    url: /^https?:\/\/\S+$/i.test(values.baseUrl.trim()) ? undefined : t("provider.error.url"),
    model: values.model.trim() === "" ? t("provider.error.model") : undefined,
  };
  const valid = !errors.name && !errors.url && !errors.model;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setAttempted(true);
    if (!valid || action.busy) return;
    const trimmedKey = key.trim();
    const request: SaveProviderRequest = {
      name,
      protocol: values.protocol,
      base_url: values.baseUrl.trim(),
      model: values.model.trim(),
      api_key: trimmedKey === "" ? undefined : trimmedKey,
      clear_key: clearKey && trimmedKey === "" ? true : undefined,
      make_active: makeActive,
    };
    const saved = await action.run(() => api.saveProvider(request));
    if (saved) {
      // The key is gone from this page as soon as the program has it.
      setKey("");
      setClearKey(false);
      onSaved(saved);
    }
  };

  return (
    <form className="px-stack" onSubmit={(event) => void submit(event)} noValidate>
      <TextField
        label={t("provider.name")}
        hint={t("provider.name.hint")}
        value={values.name}
        readOnly={editing}
        autoComplete="off"
        spellCheck={false}
        error={attempted ? errors.name : undefined}
        onChange={(next) => setValues({ ...values, name: next })}
      />
      <Segmented
        legend={t("provider.protocol")}
        hint={t("provider.protocol.hint")}
        value={values.protocol}
        onChange={(protocol) => setValues({ ...values, protocol })}
        options={PROTOCOLS.map((value) => ({ value, label: t(`provider.protocol.${value}`) }))}
      />
      <TextField
        label={t("provider.base_url")}
        hint={t("provider.base_url.hint")}
        value={values.baseUrl}
        inputMode="url"
        autoComplete="off"
        spellCheck={false}
        error={attempted ? errors.url : undefined}
        onChange={(next) => setValues({ ...values, baseUrl: next })}
      />
      <TextField
        label={t("provider.model")}
        hint={t("provider.model.hint")}
        value={values.model}
        autoComplete="off"
        spellCheck={false}
        error={attempted ? errors.model : undefined}
        onChange={(next) => setValues({ ...values, model: next })}
      />
      <TextField
        label={t("provider.key")}
        hint={editing ? t("provider.key.hint.edit") : t("provider.key.hint.new")}
        type="password"
        value={key}
        inputRef={keyRef}
        autoComplete="off"
        spellCheck={false}
        onChange={setKey}
      />
      {editing && initial.has_key ? (
        <Switch
          label={t("provider.key.clear")}
          checked={clearKey}
          onChange={setClearKey}
          onText={t("settings.state.on")}
          offText={t("settings.state.off")}
        />
      ) : null}
      <Switch
        label={t("provider.make_active")}
        checked={makeActive}
        onChange={setMakeActive}
        onText={t("settings.state.on")}
        offText={t("settings.state.off")}
      />
      {action.error ? <ErrorBanner error={action.error} /> : null}
      <div className="px-row">
        <Button type="submit" variant="primary" busy={action.busy}>
          {action.busy ? t("provider.saving") : editing ? t("provider.save.update") : t("provider.save")}
        </Button>
        {onCancel ? <Button onClick={onCancel}>{t("provider.cancel")}</Button> : null}
      </div>
    </form>
  );
}
