import { useId, type InputHTMLAttributes } from "react";

export interface TextFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "id" | "onChange" | "value"> {
  label: string;
  value: string;
  onChange: (value: string) => void;
  hint?: string;
  error?: string;
  /** Lets a screen move focus here, for example after "enter the key again". */
  inputRef?: React.Ref<HTMLInputElement>;
}

/** A labelled one-line field. The hint and the error are tied to the input for screen readers. */
export function TextField({ label, value, onChange, hint, error, inputRef, ...rest }: TextFieldProps) {
  const id = useId();
  const hintId = useId();
  const errorId = useId();
  const described = [hint ? hintId : null, error ? errorId : null].filter(Boolean).join(" ");
  return (
    <div>
      <label className="px-label" htmlFor={id}>
        {label}
      </label>
      <input
        {...rest}
        id={id}
        ref={inputRef}
        className="px-field"
        value={value}
        aria-invalid={error ? true : undefined}
        aria-describedby={described || undefined}
        onChange={(event) => onChange(event.target.value)}
      />
      {hint ? (
        <p id={hintId} className="px-hint">
          {hint}
        </p>
      ) : null}
      {error ? (
        <p id={errorId} className="px-error">
          {error}
        </p>
      ) : null}
    </div>
  );
}
