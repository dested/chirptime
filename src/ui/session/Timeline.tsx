import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { SessionManifest, SessionTrack } from "../types";
import type { SyncEngine } from "./engine";
import { fmtClock, fmtClockShort, fmtElapsed } from "./format";

/**
 * Session-wide scrubber. One row per track showing every segment, every gap and
 * every marker across the whole session, with a playhead in master epoch-ms.
 */

interface Props {
  manifest: SessionManifest;
  engine: SyncEngine;
  start: number;
  end: number;
  focusedTrackId: string | null;
  onFocusTrack: (id: string) => void;
}

const TICK_STEPS_S = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 14400];

const KIND_COLOR: Record<string, { seg: string; segAlt: string; text: string }> = {
  screen: { seg: "bg-sky-500/40", segAlt: "bg-sky-500/55", text: "text-sky-400" },
  webcam: { seg: "bg-purple-500/40", segAlt: "bg-purple-500/55", text: "text-purple-400" },
  mic: { seg: "bg-white/20", segAlt: "bg-white/30", text: "text-white/50" },
  "system-audio": { seg: "bg-white/20", segAlt: "bg-white/30", text: "text-white/50" },
};

export function Timeline({ manifest, engine, start, end, focusedTrackId, onFocusTrack }: Props) {
  const span = Math.max(1, end - start);
  const headRef = useRef<HTMLDivElement>(null);
  const laneRef = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<{ pct: number; epoch: number } | null>(null);
  const draggingRef = useRef(false);

  const pct = useCallback((epoch: number) => ((epoch - start) / span) * 100, [start, span]);

  useEffect(() => {
    return engine.subscribeFrame((s) => {
      if (headRef.current) {
        headRef.current.style.left = `${((s.masterEpochMs - start) / span) * 100}%`;
      }
    });
  }, [engine, start, span]);

  const ticks = useMemo(() => {
    const target = 10;
    const stepS = TICK_STEPS_S.find((s) => span / (s * 1000) <= target) ?? TICK_STEPS_S[TICK_STEPS_S.length - 1];
    const stepMs = stepS * 1000;
    const first = Math.ceil(start / stepMs) * stepMs;
    const out: Array<{ epoch: number; label: string }> = [];
    for (let t = first; t <= end; t += stepMs) {
      out.push({ epoch: t, label: stepS >= 60 ? fmtClockShort(t) : fmtClock(t) });
    }
    return out;
  }, [start, end, span]);

  const epochFromEvent = useCallback(
    (clientX: number) => {
      const el = laneRef.current;
      if (!el) return start;
      const r = el.getBoundingClientRect();
      const f = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
      return start + f * span;
    },
    [start, span],
  );

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      draggingRef.current = true;
      try {
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      } catch {
        // no active pointer (synthetic event) — dragging still works via move/up
      }
      engine.seekTo(epochFromEvent(e.clientX));
    },
    [engine, epochFromEvent],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const epoch = epochFromEvent(e.clientX);
      setHover({ pct: ((epoch - start) / span) * 100, epoch });
      if (draggingRef.current) engine.seekTo(epoch);
    },
    [engine, epochFromEvent, start, span],
  );

  const onPointerUp = useCallback((e: React.PointerEvent) => {
    draggingRef.current = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* see onPointerDown */
    }
  }, []);

  return (
    <div className="shrink-0 border-t border-white/[0.06] bg-white/[0.02] select-none">
      <div className="flex">
        {/* label gutter */}
        <div className="w-[132px] shrink-0 border-r border-white/[0.06]">
          <div className="h-6 flex items-end pb-1 px-3 text-[10px] font-semibold uppercase tracking-wide text-white/25">
            Tracks
          </div>
          {manifest.tracks.map((t) => (
            <TrackLabel key={t.id} track={t} focused={t.id === focusedTrackId} onClick={() => onFocusTrack(t.id)} />
          ))}
        </div>

        {/* lanes */}
        <div
          ref={laneRef}
          className="relative flex-1 min-w-0 cursor-pointer"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerLeave={() => setHover(null)}
        >
          {/* ruler */}
          <div className="relative h-6 border-b border-white/[0.05]">
            {ticks.map((t) => (
              <div key={t.epoch} className="absolute top-0 bottom-0" style={{ left: `${pct(t.epoch)}%` }}>
                <div className="w-px h-2 bg-white/15" />
                <span className="absolute top-2 left-1 text-[9px] text-white/30 tabular-nums font-mono whitespace-nowrap">
                  {t.label}
                </span>
              </div>
            ))}
            {/* markers */}
            {manifest.markers.map((m, i) => (
              <div
                key={i}
                className="absolute top-0 h-full flex items-start"
                style={{ left: `${pct(m.epoch_ms)}%` }}
                title={`${fmtClock(m.epoch_ms)} — ${m.label}`}
              >
                <svg width="9" height="11" viewBox="0 0 9 11" className="-ml-[4px] text-amber-400">
                  <path d="M4.5 0 L9 3 L4.5 6 L0 3 Z" fill="currentColor" />
                  <path d="M4.5 5 V11" stroke="currentColor" strokeWidth="1" />
                </svg>
              </div>
            ))}
          </div>

          {manifest.tracks.map((t) => (
            <Lane key={t.id} track={t} start={start} span={span} />
          ))}

          {/* hover guide */}
          {hover && (
            <div className="absolute top-0 bottom-0 w-px bg-white/25 pointer-events-none" style={{ left: `${hover.pct}%` }}>
              <div className="absolute -top-0 left-1 px-1.5 py-0.5 rounded bg-black/80 text-[10px] text-white/80 tabular-nums font-mono whitespace-nowrap">
                {fmtElapsed(hover.epoch - start)}
              </div>
            </div>
          )}

          {/* playhead */}
          <div ref={headRef} className="absolute top-0 bottom-0 w-px bg-sky-400 pointer-events-none z-[2]" style={{ left: 0 }}>
            <div className="absolute -top-px -left-[3.5px] w-2 h-2 rounded-full bg-sky-400" />
          </div>
        </div>
      </div>
    </div>
  );
}

function TrackLabel({ track, focused, onClick }: { track: SessionTrack; focused: boolean; onClick: () => void }) {
  const c = KIND_COLOR[track.kind] ?? KIND_COLOR.mic;
  return (
    <button
      onClick={onClick}
      title={track.label}
      className={`w-full h-[22px] flex items-center gap-1.5 px-3 text-left transition-all cursor-pointer ${
        focused ? "bg-sky-500/10" : "hover:bg-white/[0.04]"
      }`}
    >
      <span className={`text-[10px] font-semibold truncate ${focused ? "text-sky-400" : c.text}`}>{track.id}</span>
    </button>
  );
}

function Lane({ track, start, span }: { track: SessionTrack; start: number; span: number }) {
  const c = KIND_COLOR[track.kind] ?? KIND_COLOR.mic;
  return (
    <div className="relative h-[22px] border-b border-white/[0.04]">
      {/* the track's own span, as a faint bed so gaps read as holes */}
      {track.gaps.map((g, i) => (
        <div
          key={`g${i}`}
          className="absolute top-[5px] h-[12px] rounded-sm border border-dashed border-red-400/40 bg-red-500/10"
          style={{
            left: `${((g.from_epoch_ms - start) / span) * 100}%`,
            width: `${Math.max(0.15, ((g.to_epoch_ms - g.from_epoch_ms) / span) * 100)}%`,
          }}
          title={`Gap ${((g.to_epoch_ms - g.from_epoch_ms) / 1000).toFixed(1)}s — ${g.reason}`}
        />
      ))}
      {track.segments.map((s, i) => (
        <div
          key={s.file}
          className={`absolute top-[5px] h-[12px] ${i % 2 === 0 ? c.seg : c.segAlt} border-r border-[#0A0A0F]/70`}
          style={{
            left: `${((s.start_epoch_ms - start) / span) * 100}%`,
            width: `${Math.max(0.05, (s.duration_ms / span) * 100)}%`,
          }}
          title={`${s.file} · ${(s.duration_ms / 1000).toFixed(2)}s`}
        />
      ))}
    </div>
  );
}
