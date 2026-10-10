/** The composer card's small controls: segmented buttons, the ink toggle and the slider, drawn like the mock. */
import { useRef } from "react";
import type { ReactNode } from "react";

export function Seg<T extends string>({
  options,
  value,
  onChange,
  className = "",
  disabled = false,
}: {
  options: readonly { value: T; label: ReactNode; title?: string }[];
  value: T | null;
  onChange: (v: T) => void;
  className?: string;
  disabled?: boolean;
}) {
  return (
    <div className={`flex rounded-ctl bg-sunk p-0.5 ${disabled ? "opacity-50" : ""} ${className}`} role="radiogroup">
      {options.map((o) => {
        const on = o.value === value;
        return (
          <button
            key={o.value}
            role="radio"
            aria-checked={on}
            title={o.title}
            disabled={disabled}
            onClick={() => onChange(o.value)}
            className={`h-[26px] flex-1 whitespace-nowrap rounded-[6px] px-2 text-[12px] font-semibold transition-colors duration-150 ${
              on ? "bg-card text-ink shadow-[0_1px_2px_rgba(0,0,0,.08)]" : "text-ink-3 hover:text-ink-2"
            }`}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

export function Toggle({ on, onChange, label, disabled = false }: { on: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={`relative h-[18px] w-[30px] shrink-0 rounded-[9px] transition-colors duration-150 disabled:opacity-50 ${on ? "bg-ink" : "bg-ink-4"}`}
    >
      <span className={`absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white transition-[left] duration-150 ${on ? "left-[14px]" : "left-[2px]"}`} />
    </button>
  );
}

/** A labelled toggle row: "Lower system audio while I talk". */
export function ToggleRow({ on, onChange, label, disabled }: { on: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <label className={`mt-[9px] flex items-center gap-2.5 text-[12px] text-ink-2 ${disabled ? "opacity-50" : "cursor-pointer"}`}>
      <Toggle on={on} onChange={onChange} label={label} disabled={disabled} />
      <span onClick={() => !disabled && onChange(!on)}>{label}</span>
    </label>
  );
}

export function Slider({
  value,
  min,
  max,
  step,
  onChange,
  label,
  disabled = false,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  label: string;
  disabled?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const f = Math.max(0, Math.min(1, (value - min) / (max - min)));
  const at = (clientX: number) => {
    const el = ref.current;
    if (!el) return value;
    const r = el.getBoundingClientRect();
    const raw = min + Math.max(0, Math.min(1, (clientX - r.left) / Math.max(1, r.width))) * (max - min);
    return Math.round(raw / step) * step;
  };
  return (
    <div
      ref={ref}
      role="slider"
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-disabled={disabled}
      tabIndex={disabled ? -1 : 0}
      className={`relative h-4 flex-1 touch-none outline-none ${disabled ? "opacity-50" : "cursor-pointer"}`}
      onPointerDown={(e) => {
        if (disabled) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        onChange(at(e.clientX));
      }}
      onPointerMove={(e) => {
        if (!disabled && e.currentTarget.hasPointerCapture(e.pointerId)) onChange(at(e.clientX));
      }}
      onKeyDown={(e) => {
        if (disabled) return;
        if (e.key === "ArrowLeft" || e.key === "ArrowDown") onChange(Math.max(min, value - step));
        else if (e.key === "ArrowRight" || e.key === "ArrowUp") onChange(Math.min(max, value + step));
        else return;
        e.preventDefault();
      }}
    >
      <div className="absolute left-0 right-0 top-1.5 h-1 rounded-[2px] bg-sunk">
        <div className="absolute bottom-0 left-0 top-0 rounded-[2px] bg-ink" style={{ width: `${f * 100}%` }} />
      </div>
      <span
        className="absolute top-0 -ml-2 h-4 w-4 rounded-full bg-card shadow-[0_0_0_1px_var(--color-line),0_1px_3px_rgba(0,0,0,.18)]"
        style={{ left: `${f * 100}%` }}
      />
    </div>
  );
}
