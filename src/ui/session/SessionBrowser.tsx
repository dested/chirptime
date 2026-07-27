import React from "react";
import type { SessionSummary } from "../types";
import { fmtBytes, fmtDateLong, fmtDuration, fmtTimeAmPm } from "./format";

interface Props {
  sessions: SessionSummary[];
  loading: boolean;
  error: string | null;
  onOpen: (id: string) => void;
  /**
   * The session the recorder is actually running right now, from
   * `get_session_status`. Without it a force-killed session — which never got an
   * `ended_epoch_ms` written — reads as "Recording" forever.
   */
  liveSessionId?: string | null;
}

export function SessionBrowser({ sessions, loading, error, onOpen, liveSessionId }: Props) {
  if (loading) {
    return <div className="flex-1 flex items-center justify-center text-slate-400 text-sm">Loading...</div>;
  }

  if (error) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center gap-3 text-slate-400 text-sm">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#CBD5E1" strokeWidth="1.5" strokeLinecap="round">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 8v5M12 16v.5" />
        </svg>
        <p>{error}</p>
      </div>
    );
  }

  if (!sessions.length) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center gap-3 text-slate-400 text-sm">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#CBD5E1" strokeWidth="1.5" strokeLinecap="round">
          <rect x="2" y="6" width="14" height="12" rx="2" />
          <path d="M16 10l6-3v10l-6-3" />
        </svg>
        <p>No recorded sessions yet</p>
      </div>
    );
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto">
      <div className="max-w-[960px] w-full mx-auto px-8 py-6 flex flex-col gap-2">
        {sessions.map((s) => (
          <button
            key={s.id}
            onClick={() => onOpen(s.id)}
            className="w-full flex items-center gap-4 px-4 py-3.5 rounded-[14px] border border-slate-200 bg-white text-left cursor-pointer hover:border-sky-500 hover:shadow-[0_4px_16px_rgba(14,165,233,0.12)] hover:-translate-y-px transition-all"
          >
            <div className="w-10 h-10 shrink-0 rounded-[10px] bg-sky-50 flex items-center justify-center">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="#0EA5E9" strokeWidth="1.8" strokeLinecap="round">
                <rect x="2" y="6" width="14" height="12" rx="2" />
                <path d="M16 10l6-3v10l-6-3" />
              </svg>
            </div>

            <div className="flex-1 min-w-0 flex flex-col gap-0.5">
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold text-slate-900">{fmtDateLong(s.started_epoch_ms)}</span>
                <span className="text-[13px] text-slate-400 tabular-nums font-mono">
                  {fmtTimeAmPm(s.started_epoch_ms)}
                </span>
                {s.ended_epoch_ms === null && s.id === liveSessionId && (
                  <span className="flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-md bg-red-50 text-red-600">
                    <span className="w-1.5 h-1.5 rounded-full bg-red-600 dot-active" />
                    Recording
                  </span>
                )}
                {s.ended_epoch_ms === null && s.id !== liveSessionId && (
                  <span
                    className="text-[10px] font-semibold px-2 py-0.5 rounded-md bg-amber-50 text-amber-700"
                    title="No end time was written — the app stopped before finalizing. The footage is intact; the last segment of each track may be unplayable."
                  >
                    Interrupted
                  </span>
                )}
                {!s.has_manifest && (
                  <span className="text-[10px] font-semibold px-2 py-0.5 rounded-md bg-slate-100 text-slate-500">
                    No manifest
                  </span>
                )}
              </div>
              <span className="text-[11px] text-slate-400 tabular-nums font-mono">{s.id}</span>
            </div>

            <div className="flex items-center gap-6 shrink-0">
              <Stat label="Duration" value={fmtDuration(s.duration_ms)} />
              <Stat label="Tracks" value={String(s.track_count)} />
              <Stat label="Segments" value={String(s.segment_count)} />
              <Stat label="Size" value={fmtBytes(s.bytes)} />
            </div>

            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#CBD5E1" strokeWidth="2" strokeLinecap="round" className="shrink-0">
              <path d="M9 18l6-6-6-6" />
            </svg>
          </button>
        ))}
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col items-end gap-0.5">
      <span className="text-[10px] font-semibold uppercase tracking-wide text-slate-400">{label}</span>
      <span className="text-[13px] font-bold text-slate-900 tabular-nums font-mono">{value}</span>
    </div>
  );
}
