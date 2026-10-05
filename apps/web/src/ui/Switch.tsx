import { useId } from "react";

export interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  /** "ON" and "OFF" in the learner's language, drawn inside the track. */
  onText: string;
  offText: string;
  hint?: string;
  disabled?: boolean;
}

/** A two-state setting. The state is written in the track, so colour is never the only cue. */
export function Switch({ checked, onChange, label, onText, offText, hint, disabled }: SwitchProps) {
  const hintId = useId();
  return (
    <div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-describedby={hint ? hintId : undefined}
        disabled={disabled}
        className="px-switch"
        onClick={() => onChange(!checked)}
      >
        <span className="px-switch__track" aria-hidden="true">
          <span className="px-switch__state">{checked ? onText : offText}</span>
          <span className="px-switch__knob" />
        </span>
        <span>{label}</span>
      </button>
      {hint ? (
        <p id={hintId} className="px-hint">
          {hint}
        </p>
      ) : null}
    </div>
  );
}
