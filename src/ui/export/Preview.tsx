/**
 * The live preview: the output frame on the dark mat, built from the
 * session's own <video>s through the player's engine (read-only). The corner
 * camera wears the canary outline and can be dragged; it snaps to the nearest
 * corner on release. Silent: the sound settings only shape the file.
 */
import { useRef, useState } from "react";
import type { SyncEngine, TrackState } from "../session/engine";
import { TrackSurface } from "../session/TrackSurface";
import type { Box, Piece } from "./compose";
import { nearestCorner } from "./compose";
import type { Corner } from "./types";

export const MAT_PAD = 12;

interface Props {
  engine: SyncEngine;
  /** the mat's outer size */
  matW: number;
  matH: number;
  /** the frame's size inside the mat */
  fw: number;
  fh: number;
  pieces: Piece[];
  states: Record<string, TrackState>;
  /** tracks with no footage at the playhead: hidden, as the export leaves them black */
  hidden: ReadonlySet<string>;
  onCorner: (c: Corner) => void;
  onTogglePlay: () => void;
}

export function Preview({ engine, matW, matH, fw, fh, pieces, states, hidden, onCorner, onTogglePlay }: Props) {
  const [drag, setDrag] = useState<{ box: Box; dx: number; dy: number } | null>(null);
  const moved = useRef(false);
  const left = (matW - fw) / 2;
  const top = (matH - fh) / 2;

  return (
    <div className="relative overflow-hidden rounded-card bg-mat" style={{ width: matW, height: matH }}>
      <div
        className="absolute overflow-hidden rounded-[3px] bg-black"
        style={{ left, top, width: fw, height: fh }}
        onClick={() => {
          if (moved.current) {
            moved.current = false;
            return;
          }
          onTogglePlay();
        }}
      >
        {pieces.map((p) => {
          const box = p.corner && drag ? drag.box : p.box;
          const off = hidden.has(p.track.id);
          if (!p.corner) {
            return (
              <TrackSurface
                key={p.track.id}
                engine={engine}
                track={p.track}
                state={states[p.track.id]}
                fit={p.fit}
                style={{ left: box.x, top: box.y, width: box.w, height: box.h, visibility: off ? "hidden" : "visible" }}
              />
            );
          }
          return (
            <div
              key={p.track.id}
              className={`absolute touch-none ${drag ? "cursor-grabbing" : "cursor-grab"}`}
              style={{
                left: box.x,
                top: box.y,
                width: box.w,
                height: box.h,
                borderRadius: p.radius,
                boxShadow: "0 0 0 2px var(--color-canary), 0 10px 30px rgba(0,0,0,.5)",
                visibility: off ? "hidden" : "visible",
                transition: drag ? "none" : "left 160ms ease, top 160ms ease",
                zIndex: 5,
              }}
              title="Drag to another corner"
              onPointerDown={(e) => {
                e.stopPropagation();
                e.currentTarget.setPointerCapture(e.pointerId);
                moved.current = false;
                setDrag({ box: p.box, dx: e.clientX - p.box.x, dy: e.clientY - p.box.y });
              }}
              onPointerMove={(e) => {
                if (!drag || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
                moved.current = true;
                const x = Math.max(0, Math.min(fw - drag.box.w, e.clientX - drag.dx));
                const y = Math.max(0, Math.min(fh - drag.box.h, e.clientY - drag.dy));
                setDrag({ ...drag, box: { ...drag.box, x, y } });
              }}
              onPointerUp={() => {
                if (drag && moved.current) onCorner(nearestCorner(fw, fh, drag.box));
                setDrag(null);
              }}
              onPointerCancel={() => setDrag(null)}
              onClick={(e) => e.stopPropagation()}
            >
              <TrackSurface
                engine={engine}
                track={p.track}
                state={states[p.track.id]}
                fit="cover"
                style={{ inset: 0, borderRadius: p.radius }}
              />
              {p.radius < Math.min(box.w, box.h) / 2 && (
                <span
                  className="pointer-events-none absolute -bottom-px -right-px z-[4] h-3 w-3 bg-canary"
                  style={{ borderRadius: `3px 0 ${p.radius}px 0` }}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
