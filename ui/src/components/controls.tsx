import type { ReactNode } from "react";

export function Toggle({ value, onChange, label }: { value: boolean; onChange: (v: boolean) => void; label?: string }) {
  return (
    <button
      role="switch"
      aria-checked={value}
      aria-label={label}
      className={`toggle ${value ? "on" : ""}`}
      onClick={() => onChange(!value)}
      style={{ border: undefined, padding: 0 }}
    />
  );
}

export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: ReactNode; title?: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="segmented" role="radiogroup">
      {options.map((o) => (
        <button
          key={String(o.value)}
          role="radio"
          aria-checked={o.value === value}
          title={o.title}
          className={o.value === value ? "on" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/**
 * Range slider. `onStart`/`onEnd` bracket a drag so the editor can record a
 * single undo step for the whole gesture.
 */
export function Slider({
  value,
  min,
  max,
  step = 0.01,
  onChange,
  onStart,
  onEnd,
  label,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number) => void;
  onStart?: () => void;
  onEnd?: () => void;
  label?: string;
}) {
  const fill = ((value - min) / (max - min)) * 100;
  return (
    <input
      type="range"
      className="slider"
      aria-label={label}
      min={min}
      max={max}
      step={step}
      value={value}
      style={{ ["--fill" as string]: `${Math.max(0, Math.min(100, fill))}%` }}
      onPointerDown={onStart}
      onPointerUp={onEnd}
      onKeyDown={onStart}
      onKeyUp={onEnd}
      onChange={(e) => onChange(Number(e.target.value))}
    />
  );
}

export function SliderField({
  label,
  value,
  display,
  ...rest
}: {
  label: string;
  value: number;
  display?: string;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number) => void;
  onStart?: () => void;
  onEnd?: () => void;
}) {
  return (
    <div className="field">
      <div className="field-row" style={{ minHeight: 0 }}>
        <span className="label">{label}</span>
        <span className="label-value">{display ?? value.toFixed(2)}</span>
      </div>
      <Slider value={value} label={label} {...rest} />
    </div>
  );
}

export function ToggleRow({ label, value, onChange, hint }: { label: string; value: boolean; onChange: (v: boolean) => void; hint?: string }) {
  return (
    <div className="field-row">
      <div className="col" style={{ gap: 2 }}>
        <span className="label" style={{ color: "var(--text)" }}>{label}</span>
        {hint && <span className="faint" style={{ fontSize: 11.5 }}>{hint}</span>}
      </div>
      <Toggle value={value} onChange={onChange} label={label} />
    </div>
  );
}
