/** The ink toast at the bottom of the player: a delete with its undo countdown, or a plain message. */
import { useEffect, useState } from "react";
import { IconX } from "../app/icons";

export type ToastState =
  | {
      kind: "deleted";
      /** changes on every delete, so the countdown restarts */
      cutId: string;
      title: string;
      detail: string;
      startedAt: number;
      until: number;
      undoing: boolean;
    }
  | { kind: "message"; id: number; title: string; detail: string | null; tone: "info" | "error" };

interface Props {
  toast: ToastState | null;
  onUndo: () => void;
  onDismiss: () => void;
}

export function Toast({ toast, onUndo, onDismiss }: Props) {
  // A plain message clears itself; a delete clears when its undo runs out.
  useEffect(() => {
    if (!toast) return;
    const ms = toast.kind === "deleted" ? Math.max(0, toast.until - Date.now()) : toast.tone === "error" ? 7000 : 2500;
    const t = window.setTimeout(onDismiss, ms);
    return () => window.clearTimeout(t);
  }, [toast, onDismiss]);

  if (!toast) return null;

  if (toast.kind === "message") {
    return (
      <div className="toast-in pointer-events-auto flex min-h-[42px] max-w-[560px] items-center gap-3 rounded-card bg-ink py-2 pl-4 pr-2 text-white shadow-[0_12px_30px_-10px_rgba(26,25,23,.55)]">
        <span className={`text-[13px] font-semibold ${toast.tone === "error" ? "text-[#FF8A80]" : ""}`}>{toast.title}</span>
        {toast.detail && <span className="text-[12px] text-white/70">{toast.detail}</span>}
        <button className="ml-auto grid h-7 w-7 shrink-0 place-items-center rounded-[7px] text-white/60 hover:bg-white/10" onClick={onDismiss} aria-label="Dismiss">
          <IconX size={12} />
        </button>
      </div>
    );
  }

  return <DeletedToast toast={toast} onUndo={onUndo} />;
}

function DeletedToast({ toast, onUndo }: { toast: Extract<ToastState, { kind: "deleted" }>; onUndo: () => void }) {
  // The bar starts at whatever is left and runs linearly to zero.
  const [startFrac] = useState(() => {
    const total = toast.until - toast.startedAt;
    return total > 0 ? Math.max(0, Math.min(1, (toast.until - Date.now()) / total)) : 0;
  });
  const remaining = Math.max(0, toast.until - Date.now());

  return (
    <div className="toast-in pointer-events-auto relative flex h-[42px] w-[400px] items-center gap-2.5 overflow-hidden rounded-card bg-ink pl-4 pr-2 text-white shadow-[0_12px_30px_-10px_rgba(26,25,23,.55)]">
      <span className="whitespace-nowrap text-[13px] font-semibold">{toast.title}</span>
      <span className="num whitespace-nowrap text-[11.5px] text-white/60">{toast.detail}</span>
      <div className="flex-1" />
      <button
        className="flex h-[30px] items-center gap-2 rounded-ctl bg-white/10 px-3 text-[13px] font-bold text-canary transition-colors duration-150 hover:bg-white/15 disabled:opacity-60"
        onClick={onUndo}
        disabled={toast.undoing}
      >
        {toast.undoing ? "Undoing" : "Undo"}
        <span className="kbd !border-white/25 !bg-transparent !text-white/55">Ctrl Z</span>
      </button>
      <span
        key={toast.cutId}
        className="absolute bottom-0 left-0 h-[2px] origin-left bg-canary"
        // a bar as wide as what is left, shrinking to nothing as undo runs out
        style={{ width: `${startFrac * 100}%`, animation: `countdown ${remaining}ms linear forwards` }}
      />
    </div>
  );
}
