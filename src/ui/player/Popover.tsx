/** A small anchored panel: opens under (or above) its trigger, closes on outside click or Esc. */
import { useEffect, useRef } from "react";
import type { ReactNode } from "react";

interface Props {
  open: boolean;
  onClose: () => void;
  /** which edge of the trigger it hangs from */
  align?: "left" | "right";
  side?: "below" | "above";
  className?: string;
  children: ReactNode;
}

export function Popover({ open, onClose, align = "left", side = "below", className, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const el = ref.current;
      // the trigger is the popover's parent: clicking it toggles, so leave it alone
      const host = el?.parentElement;
      if (el && host && e.target instanceof Node && !host.contains(e.target)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      ref={ref}
      className={`absolute z-40 rounded-[10px] border border-line bg-card p-1 shadow-[0_12px_32px_-12px_rgba(26,25,23,.35)] ${
        align === "right" ? "right-0" : "left-0"
      } ${side === "below" ? "top-[calc(100%+6px)]" : "bottom-[calc(100%+6px)]"} ${className ?? ""}`}
    >
      {children}
    </div>
  );
}

export function MenuItem({
  onClick,
  children,
  checked,
  danger,
}: {
  onClick: () => void;
  children: ReactNode;
  checked?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      className={`flex h-8 w-full items-center gap-2 whitespace-nowrap rounded-[7px] px-2.5 text-left text-[12.5px] font-medium transition-colors duration-150 hover:bg-paper ${
        danger ? "text-rec" : "text-ink"
      }`}
      onClick={onClick}
      role={checked === undefined ? "menuitem" : "menuitemcheckbox"}
      aria-checked={checked}
    >
      {checked !== undefined && (
        <span
          className={`grid h-3.5 w-3.5 shrink-0 place-items-center rounded-[4px] border ${
            checked ? "border-ink bg-ink text-white" : "border-ink-4 bg-card"
          }`}
        >
          {checked && (
            <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="4" strokeLinecap="round" strokeLinejoin="round">
              <path d="M5 12.5l4.5 4.5L19 7.5" />
            </svg>
          )}
        </span>
      )}
      {children}
    </button>
  );
}
