import React, { useEffect, useRef } from "react";
import type { SessionTrack } from "../types";
import type { SyncEngine, TrackDebug } from "./engine";
import { HARD_SEEK_MS, SOFT_NUDGE_MS } from "./engine";

/**
 * One track's viewport. Holds the two media elements the engine swaps between —
 * the visible one and the one pre-rolling the next 60s segment. The engine owns
 * their src, currentTime, playbackRate and opacity; React only owns layout.
 */

interface Props {
  engine: SyncEngine;
  track: SessionTrack;
  debug: TrackDebug | undefined;
  showSync: boolean;
  fit: "contain" | "cover";
  /** click target — usually "focus this monitor" */
  onSelect?: () => void;
  selected?: boolean;
  /** hide the label chip (used by the PiP) */
  bare?: boolean;
  className?: string;
  style?: React.CSSProperties;
}

const ACCENT: Record<string, string> = {
  screen: "text-sky-400",
  webcam: "text-purple-400",
  mic: "text-white/50",
  "system-audio": "text-white/50",
};

export function TrackSurface({
  engine,
  track,
  debug,
  showSync,
  fit,
  onSelect,
  selected,
  bare,
  className,
  style,
}: Props) {
  const aRef = useRef<HTMLVideoElement>(null);
  const bRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    const a = aRef.current;
    const b = bRef.current;
    if (!a || !b) return;
    engine.attachTrack(track.id, a, b);
    return () => engine.detachTrack(track.id);
  }, [engine, track.id]);

  const state = debug?.state ?? "pending";
  const drift = debug?.driftMs ?? 0;
  const absDrift = Math.abs(drift);
  const driftTone =
    absDrift > HARD_SEEK_MS ? "text-red-400" : absDrift > SOFT_NUDGE_MS ? "text-amber-400" : "text-emerald-400";

  const mediaStyle: React.CSSProperties = {
    position: "absolute",
    inset: 0,
    width: "100%",
    height: "100%",
    objectFit: fit,
    background: "#07070B",
  };

  return (
    <div
      className={`relative overflow-hidden rounded-lg bg-[#07070B] border ${
        selected ? "border-sky-500/70" : "border-white/[0.06]"
      } ${onSelect ? "cursor-pointer" : ""} ${className ?? ""}`}
      style={style}
      onClick={onSelect}
    >
      <video ref={aRef} style={mediaStyle} playsInline muted preload="auto" />
      <video ref={bRef} style={{ ...mediaStyle, opacity: 0 }} playsInline muted preload="auto" />

      {state !== "playing" && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-[#07070B] text-white/35 z-[2]">
          <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
            {state === "gap" ? (
              <>
                <path d="M3 12h5M16 12h5" />
                <path d="M10.5 12h1M13 12h1" opacity="0.4" />
                <circle cx="12" cy="12" r="9" opacity="0.25" />
              </>
            ) : (
              <>
                <rect x="3" y="4" width="18" height="14" rx="2" />
                <path d="M8 21h8M12 18v3" />
              </>
            )}
          </svg>
          <span className="text-[11px] font-medium">
            {state === "gap"
              ? "No footage"
              : state === "pending"
                ? "Not started yet"
                : state === "unreadable"
                  ? "Segment unreadable"
                  : "Track ended"}
          </span>
        </div>
      )}

      {!bare && (
        <div className="absolute top-2 left-2 z-[3] flex items-center gap-1.5 px-2 py-1 rounded-md bg-black/55 backdrop-blur-sm">
          <span className={`text-[10px] font-semibold ${ACCENT[track.kind] ?? "text-white/60"}`}>{track.id}</span>
          <span className="text-[10px] text-white/40 tabular-nums font-mono">
            {track.width}×{track.height}
          </span>
          <span className="text-[10px] text-white/30 uppercase">{track.codec}</span>
        </div>
      )}

      {showSync && (
        <div className="absolute bottom-2 left-2 z-[3] flex items-center gap-2 px-2 py-1 rounded-md bg-black/70 backdrop-blur-sm tabular-nums font-mono text-[10px]">
          <span className="text-white/35">seg {debug ? debug.segIndex + 1 : 0}</span>
          <span className={driftTone}>
            {drift >= 0 ? "+" : ""}
            {drift.toFixed(0)}ms
          </span>
          <span className="text-white/25">max {(debug?.maxAbsDriftMs ?? 0).toFixed(0)}</span>
          <span className="text-white/25">rs {debug?.readyState ?? 0}</span>
        </div>
      )}
    </div>
  );
}
