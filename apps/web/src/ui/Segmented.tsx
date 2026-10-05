import { useId } from "react";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
}

export interface SegmentedProps<T extends string> {
  legend: string;
  value: T;
  options: readonly SegmentedOption<T>[];
  onChange: (value: T) => void;
  hint?: string;
}

/** A radio group drawn as a row of buttons. Arrow keys and screen readers work as they do for radios. */
export function Segmented<T extends string>({ legend, value, options, onChange, hint }: SegmentedProps<T>) {
  const name = useId();
  const hintId = useId();
  return (
    <fieldset className="px-segmented" aria-describedby={hint ? hintId : undefined}>
      <legend className="px-label">{legend}</legend>
      {options.map((option) => {
        const id = `${name}-${option.value}`;
        return (
          <span key={option.value} className="px-segmented__option">
            <input
              id={id}
              className="px-segmented__input"
              type="radio"
              name={name}
              value={option.value}
              checked={option.value === value}
              onChange={() => onChange(option.value)}
            />
            <label htmlFor={id} className="px-segmented__item">
              {option.label}
            </label>
          </span>
        );
      })}
      {hint ? (
        <p id={hintId} className="px-hint" style={{ flexBasis: "100%" }}>
          {hint}
        </p>
      ) : null}
    </fieldset>
  );
}
