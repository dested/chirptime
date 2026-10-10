/** Icons and small controls shared by the recorder widget, Settings and first run. */
import type { ReactNode, SelectHTMLAttributes } from "react";

type IconProps = { size?: number; className?: string };

const stroke = (size: number, sw: number, className: string | undefined, children: ReactNode) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={sw}
    strokeLinecap="round"
    strokeLinejoin="round"
    className={className}
    aria-hidden="true"
  >
    {children}
  </svg>
);

/** The ChirpTime mark: a record dot that chirps. The dot is the status. */
export function CtMark({ size = 18, dot = "var(--color-ink)", arcs = "var(--color-ink-3)" }: { size?: number; dot?: string; arcs?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="9" cy="12" r="5.2" fill={dot} />
      <path d="M16.2 8.6a5 5 0 010 6.8" stroke={arcs} strokeWidth="2" strokeLinecap="round" />
      <path d="M19.4 5.8a9 9 0 010 12.4" stroke={arcs} strokeWidth="2" strokeLinecap="round" opacity=".55" />
    </svg>
  );
}

export const IconLibrary = ({ size = 15, className }: IconProps) =>
  stroke(size, 2, className, <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M3 10h18M9 4v16" /></>);
export const IconOptions = ({ size = 15, className }: IconProps) =>
  stroke(size, 2, className, <><path d="M4 7h10M18 7h2M4 17h4M12 17h8" /><circle cx="16" cy="7" r="2" /><circle cx="10" cy="17" r="2" /></>);
export const IconMinimize = ({ size = 12, className }: IconProps) => stroke(size, 2.2, className, <path d="M5 12h14" />);
export const IconClose = ({ size = 12, className }: IconProps) => stroke(size, 2.2, className, <path d="M6 6l12 12M18 6L6 18" />);
export const IconPill = ({ size = 15, className }: IconProps) =>
  stroke(size, 2, className, <><rect x="3" y="9" width="18" height="7" rx="3.5" /><path d="M8 5h8" /></>);
export const IconExpand = ({ size = 13, className }: IconProps) =>
  stroke(size, 2.2, className, <path d="M4 14v6h6M20 10V4h-6M4 20l6-6M20 4l-6 6" />);
export const IconChevron = ({ size = 14, className }: IconProps) => stroke(size, 2.2, className, <path d="M9 6l6 6-6 6" />);
export const IconDown = ({ size = 12, className }: IconProps) => stroke(size, 2.4, className, <path d="M6 9l6 6 6-6" />);
export const IconBack = ({ size = 15, className }: IconProps) => stroke(size, 2.2, className, <path d="M15 18l-6-6 6-6" />);
export const IconFlag = ({ size = 14, className }: IconProps) => stroke(size, 2.4, className, <path d="M5 21V4h12l-2 4 2 4H5" />);
export const IconCamera = ({ size = 13, className }: IconProps) =>
  stroke(size, 2, className, <><rect x="2" y="6" width="14" height="12" rx="2" /><path d="M16 10l6-3v10l-6-3" /></>);
export const IconWarn = ({ size = 13, className }: IconProps) =>
  stroke(size, 2.4, className, <><path d="M12 3l9.5 17h-19z" /><path d="M12 10v4M12 17.5v.5" /></>);

export function IconCheck({ size = 8, color = "#fff", width = 4 }: { size?: number; color?: string; width?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={width} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 12l5 5L20 6" />
    </svg>
  );
}

export function IconPause({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <rect x="5" y="4" width="5" height="16" rx="1" />
      <rect x="14" y="4" width="5" height="16" rx="1" />
    </svg>
  );
}

export function IconPlay({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M7 4.5v15a1 1 0 001.5.86l12-7.5a1 1 0 000-1.72l-12-7.5A1 1 0 007 4.5z" />
    </svg>
  );
}

/** An on/off switch, 30×18 like the mocks. */
export function Toggle({ on, onChange, disabled, label }: { on: boolean; onChange: (on: boolean) => void; disabled?: boolean; label: string }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={`relative h-[18px] w-[30px] shrink-0 rounded-full transition-colors duration-150 disabled:opacity-50 ${on ? "bg-ink" : "bg-ink-4"}`}
    >
      <span
        className={`absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white transition-[left] duration-150 ${on ? "left-[14px]" : "left-[2px]"}`}
      />
    </button>
  );
}

/** A native select dressed as the mocks' `.sel`. */
export function Select({ className = "", children, ...rest }: SelectHTMLAttributes<HTMLSelectElement> & { children: ReactNode }) {
  return (
    <span className={`relative flex min-w-0 items-center ${className}`}>
      <select
        {...rest}
        className="h-8 w-full min-w-0 appearance-none truncate rounded-ctl border border-line bg-card pl-2.5 pr-7 text-[12.5px] text-ink outline-none transition-colors hover:border-ink-4 focus-visible:border-ink-3 disabled:text-ink-3"
      >
        {children}
      </select>
      <IconDown className="pointer-events-none absolute right-2.5 text-ink-3" />
    </span>
  );
}

/** A filled square check, as in the mocks' chips and first run. */
export function CheckBox({ on, size = 12 }: { on: boolean; size?: number }) {
  return (
    <span
      className={`grid shrink-0 place-items-center rounded-[3px] ${on ? "bg-ink" : "border border-ink-4 bg-card"}`}
      style={{ width: size, height: size }}
    >
      {on && <IconCheck size={Math.round(size * 0.68)} />}
    </span>
  );
}
