import React, { useEffect, useRef } from "react";
import type { SessionTrack } from "../types";
import type { SyncEngine, TrackState } from "./engine";

/**
 * One track's viewport on the mat. Holds the two media elements the engine
 * swaps between — the visible one and the one pre-rolling the next segment. The
 * engine owns their src, currentTime, playbackRate and opacity; React only owns
 * layout. The surface is absolutely positioned: place it with `style`.
 */

interface Props {
  engine: SyncEngine;
  track: SessionTrack;
  state: TrackState | undefined;
  fit?: "contain" | "cover" | "fill";
  /** small label in the corner, e.g. "Camera" */
  label?: string;
  onSelect?: () => void;
  className?: string;
  style?: React.CSSProperties;
  /**
   * While the track has no footage: the last frame before the hole (a /thumb
   * URL, or null when there is none) and why it is empty ("Away", "Cut",
   * "The display went dark"), shown dimmed instead of a blank panel.
   */
  still?: { url: string | null; reason: string | null };
}

const EMPTY_TEXT: Record<Exclude<TrackState, "playing">, string> = {
  gap: "No footage here",
  pending: "Not started yet",
  unreadable: "Segment unreadable",
  ended: "Track ended",
};

const MEDIA_STYLE: React.CSSProperties = {
  position: "absolute",
  inset: 0,
  width: "100%",
  height: "100%",
  background: "#000",
};

export function TrackSurface({ engine, track, state, fit = "contain", label, onSelect, className, style, still }: Props) {
  const aRef = useRef<HTMLVideoElement>(null);
  const bRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    const a = aRef.current;
    const b = bRef.current;
    if (!a || !b) return;
    engine.attachTrack(track.id, a, b);
    return () => engine.detachTrack(track.id);
  }, [engine, track.id]);

  const s = state ?? "pending";

  return (
    <div
      className={`absolute overflow-hidden bg-black ${onSelect ? "cursor-pointer" : ""} ${className ?? ""}`}
      style={style}
      onClick={onSelect}
    >
      <video ref={aRef} style={{ ...MEDIA_STYLE, objectFit: fit }} playsInline muted preload="auto" />
      <video ref={bRef} style={{ ...MEDIA_STYLE, objectFit: fit, opacity: 0 }} playsInline muted preload="auto" />

      {s !== "playing" &&
        (still && (s === "gap" || s === "ended") ? (
          <div className="absolute inset-0 z-[2] bg-[#141412]">
            {still.url && (
              <img
                key={still.url}
                src={still.url}
                alt=""
                draggable={false}
                className="absolute inset-0 h-full w-full opacity-[.32] [filter:grayscale(.4)]"
                style={{ objectFit: fit }}
                onError={(e) => {
                  e.currentTarget.style.visibility = "hidden";
                }}
              />
            )}
            <div className="absolute inset-0 grid place-items-center">
              <span className="rounded-[7px] bg-black/45 px-2.5 py-1 text-[12px] font-medium text-[#F3F0E8]/75">
                No footage{still.reason ? ` · ${still.reason}` : ""}
              </span>
            </div>
          </div>
        ) : (
          <div className="absolute inset-0 z-[2] grid place-items-center bg-[#141412] text-[12px] font-medium text-[#F3F0E8]/45">
            {EMPTY_TEXT[s]}
          </div>
        ))}

      {label && (
        <span className="absolute left-1.5 bottom-1 z-[3] text-[10px] font-semibold text-white [text-shadow:0_1px_2px_rgba(0,0,0,.8)]">
          {label}
        </span>
      )}
    </div>
  );
}
