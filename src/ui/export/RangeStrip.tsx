/**
 * The export range on the same calm strip the player uses: screen thumbnails
 * across a window around the range, a paper veil outside it, a canary-deep
 * outline, the ink playhead, and canary time chips on both edges. Drag an edge
 * (or its chip) to move it, snapping to gaps, marks and cuts; click inside to
 * move the preview there.
 */
import { useMemo, useRef, useState } from "react";
import type { PointerEvent } from "react";
import type { SessionManifest, SessionTrack } from "../types";
import { thumbUrl } from "../hooks/mediaUrl";
import { fmtClock12, fmtTick } from "../session/format";
import { epochAt, frac, snap, thumbFor } from "../player/timeline";
import type { Axis, SnapTarget } from "../player/timeline";

const H = 48;
const TILE_W = 86;
const EDGE_GRAB_PX = 9;
const MIN_RANGE_MS = 1_000;

interface Props {
  manifest: SessionManifest;
  axis: Axis;
  width: number;
  from: number;
  to: number;
  playhead: number;
  hero: SessionTrack | null;
  targets: readonly SnapTarget[];
  /** `done` once the drag ends: the window may re-centre then */
  onRange: (from: number, to: number, done: boolean) => void;
  onSeek: (t: number) => void;
}

type Drag = { edge: "from" | "to" } | { edge: "seek" };

export function RangeStrip({ manifest, axis, width, from, to, playhead, hero, targets, onRange, onSeek }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const X = (t: number) => Math.max(0, Math.min(1, frac(axis, t))) * width;
  const x0 = X(from);
  const x1 = X(to);
  const ph = X(playhead);

  const tiles = useMemo(() => {
    if (!hero || width <= 0) return [];
    const n = Math.max(1, Math.ceil(width / TILE_W));
    const w = width / n;
    return Array.from({ length: n }, (_, i) => {
      const t = epochAt(axis, (i + 0.5) / n);
      const at = thumbFor(manifest, hero, t);
      return { left: i * w, w, at };
    });
  }, [axis, hero, manifest, width]);

  const timeAt = (clientX: number) => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return from;
    return epochAt(axis, Math.max(0, Math.min(1, (clientX - r.left) / Math.max(1, r.width))));
  };
  const pxPerMs = width / Math.max(1, axis.span);

  const moveEdge = (edge: "from" | "to", clientX: number, done: boolean) => {
    const t = snap(targets, timeAt(clientX), pxPerMs).t;
    if (edge === "from") onRange(Math.min(t, to - MIN_RANGE_MS), to, done);
    else onRange(from, Math.max(t, from + MIN_RANGE_MS), done);
  };

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    const x = e.clientX - r.left;
    const edge: Drag = Math.abs(x - x0) <= EDGE_GRAB_PX ? { edge: "from" } : Math.abs(x - x1) <= EDGE_GRAB_PX ? { edge: "to" } : { edge: "seek" };
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag(edge);
    if (edge.edge === "seek") onSeek(Math.max(from, Math.min(to, timeAt(e.clientX))));
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
    if (drag.edge === "seek") onSeek(Math.max(from, Math.min(to, timeAt(e.clientX))));
    else moveEdge(drag.edge, e.clientX, false);
  };
  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (drag && drag.edge !== "seek") moveEdge(drag.edge, e.clientX, true);
    setDrag(null);
  };

  const chipDown = (edge: "from" | "to") => (e: PointerEvent<HTMLDivElement>) => {
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ edge });
  };
  const chipMove = (e: PointerEvent<HTMLDivElement>) => {
    if (drag && drag.edge !== "seek" && e.currentTarget.hasPointerCapture(e.pointerId)) moveEdge(drag.edge, e.clientX, false);
  };

  return (
    <div className="relative select-none" style={{ width, height: H + 34 }}>
      <div
        ref={ref}
        className={`absolute left-0 top-0 overflow-hidden rounded-[7px] bg-sunk ${drag?.edge === "from" || drag?.edge === "to" ? "cursor-ew-resize" : "cursor-pointer"}`}
        style={{ width, height: H }}
        onPointerDown={onDown}
        onPointerMove={(e) => {
          onMove(e);
          // the edge cursor while hovering an edge
          const r = ref.current?.getBoundingClientRect();
          if (r && !drag) {
            const x = e.clientX - r.left;
            e.currentTarget.style.cursor = Math.abs(x - x0) <= EDGE_GRAB_PX || Math.abs(x - x1) <= EDGE_GRAB_PX ? "ew-resize" : "pointer";
          }
        }}
        onPointerUp={onUp}
        onPointerCancel={() => setDrag(null)}
      >
        {hero &&
          tiles.map((t, i) =>
            t.at === null ? null : (
              <img
                key={i}
                src={thumbUrl(manifest.id, hero.id, t.at, 172)}
                alt=""
                draggable={false}
                className="absolute top-0 h-full object-cover"
                style={{ left: t.left, width: t.w - 1 }}
                onError={(e) => {
                  e.currentTarget.style.visibility = "hidden";
                }}
              />
            ),
          )}
        <div className="absolute left-0 top-0 h-full bg-[rgba(245,243,238,.6)]" style={{ width: x0 }} />
        <div className="absolute top-0 h-full bg-[rgba(245,243,238,.6)]" style={{ left: x1, width: Math.max(0, width - x1) }} />
      </div>
      <div
        className="pointer-events-none absolute rounded-[8px] border-[2.5px] border-canary-deep"
        style={{ left: x0 - 2, top: -3, width: Math.max(4, x1 - x0 + 4), height: H + 6 }}
      />
      {playhead >= from && playhead <= to && (
        <div className="pointer-events-none absolute w-0.5 bg-ink" style={{ left: ph - 1, top: -4, height: H + 8 }} />
      )}
      <span className="num absolute left-0 text-[10.5px] text-ink-3" style={{ top: H + 9 }}>
        {fmtTick(axis.from)}
      </span>
      <span className="num absolute right-0 text-[10.5px] text-ink-3" style={{ top: H + 9 }}>
        {fmtTick(axis.to)}
      </span>
      {(
        [
          ["from", from, x0],
          ["to", to, x1],
        ] as const
      ).map(([edge, t, x]) => (
        <div
          key={edge}
          role="slider"
          aria-label={edge === "from" ? "Range start" : "Range end"}
          aria-valuenow={t}
          className="num absolute grid h-5 w-20 cursor-ew-resize touch-none place-items-center rounded-[6px] bg-canary text-[11px] font-semibold text-canary-ink"
          style={{ left: Math.max(0, Math.min(width - 80, x - 40)), top: H + 6, zIndex: drag?.edge === edge ? 2 : 1 }}
          onPointerDown={chipDown(edge)}
          onPointerMove={chipMove}
          onPointerUp={(e) => {
            if (drag && drag.edge !== "seek") moveEdge(drag.edge, e.clientX, true);
            setDrag(null);
          }}
        >
          {fmtClock12(t)}
        </div>
      ))}
    </div>
  );
}
