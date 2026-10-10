/**
 * The day ribbon (the whole session, cuts collapsed to a red seam) and the zoom
 * strip under it (the bracketed region, screen frames). Both take a drag to
 * select and a click to seek; the playhead on both is written imperatively from
 * the engine's frame subscription.
 */
import { useEffect, useMemo, useRef } from "react";
import type { PointerEvent as ReactPointerEvent, RefObject } from "react";
import type { SessionManifest, SessionTrack } from "../types";
import type { SyncEngine } from "../session/engine";
import { thumbUrl } from "../hooks/mediaUrl";
import { fmtClock12, fmtHourLabel, fmtSpan, fmtTick, fmtTimeAmPm } from "../session/format";
import type { Hole } from "../session/cuts";
import type { Axis, Outage, SnapKind, SnapTarget, Span } from "./timeline";
import { epochAt, frac, groupOutages, makeAxis, snap, snapLabel, thumbFor } from "./timeline";

export interface Sel {
  from: number;
  to: number;
  snapFrom: SnapKind | null;
  snapTo: SnapKind | null;
}

export type SelPhase = "select" | "confirm" | "collapsing";

interface Props {
  engine: SyncEngine;
  manifest: SessionManifest;
  axis: Axis;
  holes: readonly Hole[];
  idle: Span[];
  outages: Outage[];
  zoom: { from: number; to: number };
  onZoom: (z: { from: number; to: number }) => void;
  sel: Sel | null;
  phase: SelPhase;
  /** a lone in or out point (I or O pressed once) */
  edge: number | null;
  /** which edge the last snap hint belongs to */
  snapHint: { edge: "from" | "to"; kind: SnapKind } | null;
  targets: SnapTarget[];
  focused: SessionTrack | null;
  webcam: SessionTrack | null;
  width: number;
  wide: boolean;
  onSeek: (t: number) => void;
  onSelect: (s: Sel, done: boolean, source: "day" | "zoom") => void;
}

const DAY_THUMB_W = 160;
const ZOOM_THUMB_W = 240;
const VEIL = "rgba(245,243,238,.55)";
const MIN = 60_000;

/* ---- pointer gestures shared by both strips --------------------------- */

type Gesture =
  | { mode: "pending"; x0: number; t0: number }
  | { mode: "select"; x0: number; t0: number }
  | { mode: "edge"; which: "from" | "to" }
  | { mode: "scrub" };

function useStripGestures(opts: {
  axis: Axis;
  width: number;
  targets: SnapTarget[];
  sel: Sel | null;
  locked: boolean;
  onSeek: (t: number) => void;
  onSelect: (s: Sel, done: boolean) => void;
}) {
  const g = useRef<Gesture | null>(null);
  const o = useRef(opts);
  o.current = opts;

  const tAt = (e: { clientX: number }, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    return epochAt(o.current.axis, f);
  };
  const pxPerMs = () => o.current.width / o.current.axis.span;
  const snapT = (t: number) => snap(o.current.targets, t, pxPerMs());

  const build = (a: number, b: number): Sel => {
    const sa = snapT(Math.min(a, b));
    const sb = snapT(Math.max(a, b));
    return { from: sa.t, to: sb.t, snapFrom: sa.kind, snapTo: sb.kind };
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    const { sel, locked, axis, width } = o.current;
    const t = tAt(e, el);
    if (sel && !locked) {
      const r = el.getBoundingClientRect();
      const x = e.clientX - r.left;
      const xf = frac(axis, sel.from) * width;
      const xt = frac(axis, sel.to) * width;
      if (Math.abs(x - xf) <= 6) g.current = { mode: "edge", which: "from" };
      else if (Math.abs(x - xt) <= 6) g.current = { mode: "edge", which: "to" };
    }
    if (!g.current) g.current = { mode: "pending", x0: e.clientX, t0: t };
    el.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLElement>) => {
    const cur = g.current;
    if (!cur) return;
    const el = e.currentTarget;
    const t = tAt(e, el);
    const { sel, locked, onSelect, onSeek } = o.current;
    if (cur.mode === "scrub") {
      onSeek(t);
    } else if (cur.mode === "edge" && sel) {
      const s = snapT(t);
      const next: Sel =
        cur.which === "from"
          ? { ...sel, from: Math.min(s.t, sel.to - 1000), snapFrom: s.kind }
          : { ...sel, to: Math.max(s.t, sel.from + 1000), snapTo: s.kind };
      onSelect(next, false);
    } else if (cur.mode === "pending" && Math.abs(e.clientX - cur.x0) > 4 && !locked) {
      g.current = { mode: "select", x0: cur.x0, t0: cur.t0 };
      onSelect(build(cur.t0, t), false);
    } else if (cur.mode === "select") {
      onSelect(build(cur.t0, t), false);
    }
  };

  const onPointerUp = (e: ReactPointerEvent<HTMLElement>) => {
    const cur = g.current;
    g.current = null;
    if (!cur) return;
    const el = e.currentTarget;
    const t = tAt(e, el);
    const { sel, onSelect, onSeek } = o.current;
    if (cur.mode === "pending") onSeek(cur.t0);
    else if (cur.mode === "select") {
      const s = build(cur.t0, t);
      if (s.to - s.from >= 1000) onSelect(s, true);
    } else if (cur.mode === "edge" && sel) onSelect(sel, true);
  };

  const onPointerCancel = () => {
    g.current = null;
  };

  /** The playhead knob hands its drag to the strip, which scrubs while it holds the pointer. */
  const startScrubOn = (strip: HTMLElement, pointerId: number) => {
    g.current = { mode: "scrub" };
    strip.setPointerCapture(pointerId);
  };

  return { handlers: { onPointerDown, onPointerMove, onPointerUp, onPointerCancel }, startScrubOn };
}

/** The playhead's x on a strip, written straight to the DOM every frame. */
function usePlayheadX(engine: SyncEngine, axis: Axis, width: number, refs: RefObject<HTMLElement | null>[]) {
  const a = useRef(axis);
  a.current = axis;
  const w = useRef(width);
  w.current = width;
  const els = useRef(refs);
  els.current = refs;
  useEffect(() => {
    const place = (t: number) => {
      const f = frac(a.current, t);
      const visible = f >= -0.001 && f <= 1.001;
      for (const r of els.current) {
        const el = r.current;
        if (!el) continue;
        el.style.transform = `translateX(${f * w.current}px)`;
        el.style.visibility = visible ? "visible" : "hidden";
      }
    };
    place(engine.masterEpochMs);
    return engine.subscribeFrame((s) => place(s.masterEpochMs));
  }, [engine, axis, width]);
}

/* ---- the whole thing ---------------------------------------------------- */

export function Ribbon(p: Props) {
  const thH = p.wide ? 60 : 50;
  const zH = p.wide ? 52 : 40;
  const zoomAxis = useMemo(() => makeAxis(p.holes, p.zoom.from, p.zoom.to), [p.holes, p.zoom.from, p.zoom.to]);

  // While playing, keep the playhead inside the bracket: when it leaves, move the
  // bracket so the playhead sits a tenth of the way in.
  const zoomRef = useRef(p.zoom);
  zoomRef.current = p.zoom;
  const onZoom = p.onZoom;
  const { axis } = p;
  useEffect(() => {
    return p.engine.subscribeState((s) => {
      const z = zoomRef.current;
      if (!s.playing || (s.masterEpochMs >= z.from && s.masterEpochMs <= z.to)) return;
      onZoom(placeZoom(axis, s.masterEpochMs, z, 0.1));
    });
  }, [p.engine, axis, onZoom]);

  return (
    <div className="relative shrink-0 select-none" style={{ width: p.width }}>
      <DayStrip {...p} thH={thH} />
      <ZoomStrip {...p} zoomAxis={zoomAxis} zH={zH} />
    </div>
  );
}

/** A window of the same collapsed length as `z`, placed so `t` sits at `at` of it, clamped to the session. */
export function placeZoom(axis: Axis, t: number, z: { from: number; to: number }, at: number): { from: number; to: number } {
  const len = Math.min(axis.span, frac(axis, z.to) * axis.span - frac(axis, z.from) * axis.span);
  let a = frac(axis, t) * axis.span - len * at;
  a = Math.max(0, Math.min(axis.span - len, a));
  return { from: epochAt(axis, a / axis.span), to: epochAt(axis, (a + len) / axis.span) };
}

/* ---- day ribbon ----------------------------------------------------------- */

function hourStep(pxPerMs: number): number {
  for (const m of [15, 30, 60, 120, 180, 240]) if (m * MIN * pxPerMs >= 84) return m * MIN;
  return 360 * MIN;
}

function DayStrip(p: Props & { thH: number }) {
  const { axis, width: W, thH, manifest, webcam, holes } = p;
  const phRef = useRef<HTMLDivElement>(null);
  const knobRef = useRef<HTMLDivElement>(null);
  usePlayheadX(p.engine, axis, W, [phRef, knobRef]);

  const locked = p.sel !== null && p.phase !== "select";
  const { handlers, startScrubOn } = useStripGestures({
    axis,
    width: W,
    targets: p.targets,
    sel: p.sel,
    locked,
    onSeek: p.onSeek,
    onSelect: (s, done) => p.onSelect(s, done, "day"),
  });
  const X = (t: number) => frac(axis, t) * W;
  const inHole = (t: number) => holes.some((h) => t > h.from && t < h.to);

  /* hour labels */
  const step = hourStep(W / axis.span);
  const ticks: number[] = [];
  for (let t = Math.ceil(axis.from / step) * step; t <= axis.to; t += step) {
    if (!inHole(t)) ticks.push(t);
  }

  /* camera tiles: every 30 min, or as many as fit at 64 px */
  const n = Math.max(1, Math.max(Math.ceil(axis.span / (30 * MIN)), Math.floor(W / 64)));
  const tiles = useMemo(() => {
    const out: { i: number; t: number | null }[] = [];
    for (let i = 0; i < n; i++) {
      let t: number | null = null;
      if (webcam) {
        for (const f of [0.5, 0.15, 0.85]) {
          t = thumbFor(manifest, webcam, epochAt(axis, (i + f) / n));
          if (t !== null) break;
        }
      }
      out.push({ i, t });
    }
    return out;
  }, [n, axis, manifest, webcam]);

  /* landmarks under the strip: marks and screens-stopped, labels dropped where they would collide */
  const landmarks = useMemo(() => {
    const list: { x: number; kind: "mark" | "stop"; label: string; at: number }[] = [];
    for (const m of manifest.markers) {
      if (!inHoleStrict(holes, m.epoch_ms)) list.push({ x: 0, kind: "mark", label: m.label.trim() || "Mark", at: m.epoch_ms });
    }
    for (const g of groupOutages(p.outages, "from")) {
      if (inHoleStrict(holes, g[0].from)) continue;
      const label = g.length === 1 ? `Screen ${g[0].screen} stopped` : "Screens stopped";
      list.push({ x: 0, kind: "stop", label, at: g[0].from });
    }
    list.sort((a, b) => a.at - b.at);
    const xs = list.map((l) => frac(axis, l.at) * W);
    let lastEnd = -Infinity;
    // A label shows only where it clears the label before it and the next dot.
    return list.map((l, i) => {
      const x = xs[i] ?? 0;
      const w = l.label.length * 6.4 + 18;
      const nextDot = xs[i + 1] ?? Infinity;
      const showLabel = x - 4 >= lastEnd + 6 && x - 4 + w <= nextDot - 8 && x + w <= W + 4;
      lastEnd = showLabel ? x - 4 + w : Math.max(lastEnd, x + 6);
      return { ...l, x, showLabel };
    });
  }, [manifest.markers, p.outages, holes, axis, W]);

  const sel = p.sel;
  const red = p.phase !== "select";

  return (
    <div className="relative" style={{ height: 24 + thH + 26 }}>
      {/* hour labels */}
      {ticks.map((t) => {
        const x = X(t);
        const end = x > W - 34;
        const onHour = new Date(t).getMinutes() === 0;
        return (
          <div
            key={t}
            className="num pointer-events-none absolute top-0 whitespace-nowrap text-[10.5px] text-ink-3"
            style={{ left: x - 1, transform: end ? "translateX(-100%)" : undefined }}
          >
            {onHour ? fmtHourLabel(t) : fmtTick(t)}
          </div>
        );
      })}

      {/* the strip */}
      <div
        data-strip
        className="absolute left-0 cursor-crosshair overflow-hidden rounded-ctl bg-sunk"
        style={{ top: 24, width: W, height: thH, touchAction: "none" }}
        {...handlers}
      >
        {tiles.map((tile) =>
          tile.t !== null && webcam ? (
            <img
              key={tile.i}
              src={thumbUrl(manifest.id, webcam.id, tile.t, DAY_THUMB_W)}
              alt=""
              draggable={false}
              className="pointer-events-none absolute top-0 h-full object-cover"
              style={{ left: (tile.i * W) / n, width: W / n - 1 }}
              onError={(e) => {
                e.currentTarget.style.visibility = "hidden";
              }}
            />
          ) : null,
        )}
        {/* away: a paper veil, labelled */}
        {p.idle.map((s) => {
          const a = X(s.from);
          const b = X(s.to);
          if (b - a < 1) return null;
          return (
            <div
              key={s.from}
              className="pointer-events-none absolute top-0 grid h-full place-items-center overflow-hidden bg-[rgba(245,243,238,.62)]"
              style={{ left: a, width: b - a }}
            >
              {b - a > 64 && <span className="whitespace-nowrap text-[11.5px] font-semibold text-ink-2">Away {fmtSpan(s.to - s.from)}</span>}
            </div>
          );
        })}
        {sel && (
          <>
            <div className="pointer-events-none absolute left-0 top-0 h-full" style={{ width: Math.max(0, X(sel.from)), background: VEIL }} />
            <div className="pointer-events-none absolute right-0 top-0 h-full" style={{ left: X(sel.to), background: VEIL }} />
          </>
        )}
      </div>

      {/* selection box */}
      {sel && (
        <SelBox x0={X(sel.from)} x1={X(sel.to)} top={24 - 3} h={thH + 6} red={red} collapsing={p.phase === "collapsing"} radius={9} />
      )}
      {p.edge !== null && !sel && (
        <div className="pointer-events-none absolute w-[2px] bg-canary-deep" style={{ left: X(p.edge) - 1, top: 24 - 4, height: thH + 8 }} />
      )}

      {/* cut seams */}
      {holes.map((h) =>
        h.to > axis.from && h.from < axis.to ? (
          <div key={h.from}>
            <div className="pointer-events-none absolute w-[2px] rounded-[1px] bg-rec" style={{ left: X(h.from) - 1, top: 24 - 4, height: thH + 8 }} />
            <div className="num pointer-events-none absolute w-[64px] text-center text-[10.5px] font-semibold text-rec" style={{ left: X(h.from) - 32, top: 24 + thH + 6 }}>
              −{fmtSpan(h.to - h.from)}
            </div>
          </div>
        ) : null,
      )}

      {/* landmarks */}
      {landmarks.map((l) => (
        <div
          key={`${l.kind}-${l.at}`}
          className={`absolute flex cursor-pointer items-center gap-[5px] whitespace-nowrap text-[11px] font-semibold ${l.kind === "stop" ? "text-[#9A5A0C]" : "text-ink-2"}`}
          style={{ left: l.x - 4, top: 24 + thH + 6 }}
          onClick={() => p.onSeek(l.at)}
          title={`${l.label} · ${fmtTimeAmPm(l.at)}`}
        >
          <span className={`h-2 w-2 shrink-0 ${l.kind === "mark" ? "rounded-[2px] bg-canary-deep" : "rounded-full bg-[#E39A3B]"}`} />
          {l.showLabel && l.label}
        </div>
      ))}

      {/* zoom bracket */}
      {!sel && <ZoomBracket axis={axis} width={W} top={24 - 3} h={thH + 6} zoom={p.zoom} onZoom={p.onZoom} />}

      {/* playhead: an ink line with a canary knob you can drag */}
      <div ref={phRef} className="pointer-events-none absolute left-0 w-[2px] bg-ink" style={{ top: 24 - 6, height: thH + 12, marginLeft: -1 }} />
      <div
        ref={knobRef}
        className="absolute left-0 h-[10px] w-[10px] cursor-grab rounded-full bg-canary shadow-[0_0_0_2px_var(--color-ink)]"
        style={{ top: 24 - 11, marginLeft: -5 }}
        onPointerDown={(e) => {
          const strip = e.currentTarget.parentElement?.querySelector<HTMLElement>("[data-strip]");
          if (!strip) return;
          e.stopPropagation();
          startScrubOn(strip, e.pointerId);
        }}
      />
    </div>
  );
}

function inHoleStrict(holes: readonly Hole[], t: number): boolean {
  return holes.some((h) => t >= h.from && t < h.to);
}

function SelBox({ x0, x1, top, h, red, collapsing, radius }: { x0: number; x1: number; top: number; h: number; red: boolean; collapsing: boolean; radius: number }) {
  return (
    <div
      className="pointer-events-none absolute origin-left rounded-[9px] transition-transform duration-[240ms] ease-[cubic-bezier(.2,.7,.2,1)]"
      style={{
        left: x0 - 2,
        top,
        width: x1 - x0 + 4,
        height: h,
        borderRadius: radius,
        border: `2.5px solid ${red ? "var(--color-rec)" : "var(--color-canary-deep)"}`,
        background: red ? "rgba(229,55,43,.14)" : "transparent",
        transform: collapsing ? "scaleX(0)" : "scaleX(1)",
      }}
    />
  );
}

/** The ink bracket over the day ribbon: drag its top edge to move it, its sides to resize. */
function ZoomBracket({
  axis,
  width: W,
  top,
  h,
  zoom,
  onZoom,
}: {
  axis: Axis;
  width: number;
  top: number;
  h: number;
  zoom: { from: number; to: number };
  onZoom: (z: { from: number; to: number }) => void;
}) {
  const drag = useRef<{ mode: "move" | "left" | "right"; x0: number; a0: number; b0: number } | null>(null);
  const a = frac(axis, zoom.from);
  const b = frac(axis, zoom.to);
  const minF = Math.min(1, (10 * MIN) / axis.span);

  const down = (mode: "move" | "left" | "right") => (e: ReactPointerEvent<HTMLDivElement>) => {
    e.stopPropagation();
    drag.current = { mode, x0: e.clientX, a0: a, b0: b };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const move = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const df = (e.clientX - d.x0) / W;
    let na = d.a0;
    let nb = d.b0;
    if (d.mode === "move") {
      const len = d.b0 - d.a0;
      na = Math.max(0, Math.min(1 - len, d.a0 + df));
      nb = na + len;
    } else if (d.mode === "left") {
      na = Math.max(0, Math.min(d.b0 - minF, d.a0 + df));
    } else {
      nb = Math.min(1, Math.max(d.a0 + minF, d.b0 + df));
    }
    onZoom({ from: epochAt(axis, na), to: epochAt(axis, nb) });
  };
  const up = () => {
    drag.current = null;
  };
  const handlers = { onPointerMove: move, onPointerUp: up, onPointerCancel: up };

  return (
    <div className="pointer-events-none absolute rounded-[9px] border-2 border-ink" style={{ left: a * W - 2, top, width: (b - a) * W + 4, height: h }}>
      <div className="pointer-events-auto absolute -top-[5px] left-2 right-2 h-[9px] cursor-grab" onPointerDown={down("move")} {...handlers} title="Drag to move the zoom" />
      <div className="pointer-events-auto absolute -bottom-[5px] left-2 right-2 h-[9px] cursor-grab" onPointerDown={down("move")} {...handlers} />
      <div className="pointer-events-auto absolute -left-[5px] top-0 h-full w-[9px] cursor-ew-resize" onPointerDown={down("left")} {...handlers} />
      <div className="pointer-events-auto absolute -right-[5px] top-0 h-full w-[9px] cursor-ew-resize" onPointerDown={down("right")} {...handlers} />
    </div>
  );
}

/* ---- zoom strip ------------------------------------------------------------ */

function tickStep(pxPerMs: number, minPx: number): number {
  for (const m of [1, 2, 5, 10, 15, 20, 30, 60, 120]) if (m * MIN * pxPerMs >= minPx) return m * MIN;
  return 240 * MIN;
}

function ZoomStrip(p: Props & { zoomAxis: Axis; zH: number }) {
  const { zoomAxis: za, width: W, zH, manifest, focused, webcam, holes } = p;
  const top = p.wide ? 34 : 26;
  const phRef = useRef<HTMLDivElement>(null);
  usePlayheadX(p.engine, za, W, [phRef]);

  const locked = p.sel !== null && p.phase !== "select";
  const { handlers } = useStripGestures({
    axis: za,
    width: W,
    targets: p.targets,
    sel: p.sel,
    locked,
    onSeek: p.onSeek,
    onSelect: (s, done) => p.onSelect(s, done, "zoom"),
  });
  const X = (t: number) => frac(za, t) * W;

  /* frames: the focused screen every 10 min, or as many as fit at 96 px; camera where it has none */
  const n = Math.max(1, Math.max(Math.ceil(za.span / (10 * MIN)), Math.floor(W / 96)));
  const tiles = useMemo(() => {
    const out: { i: number; track: SessionTrack; t: number; dim: boolean }[] = [];
    for (let i = 0; i < n; i++) {
      const at = epochAt(za, (i + 0.5) / n);
      const ft = focused ? thumbFor(manifest, focused, at) : null;
      const wt = webcam ? thumbFor(manifest, webcam, at) : null;
      if (focused && ft !== null) out.push({ i, track: focused, t: ft, dim: focused.kind !== "screen" });
      else if (webcam && wt !== null) out.push({ i, track: webcam, t: wt, dim: true });
    }
    return out;
  }, [n, za, manifest, focused, webcam]);

  /* "Camera only after 2:02 PM": where the focused screen is down */
  const cameraOnly = useMemo(() => {
    if (!focused || focused.kind !== "screen" || !webcam) return [];
    return p.outages
      .filter((o) => o.trackId === focused.id && o.to > za.from && o.from < za.to)
      .map((o) => ({
        x: Math.max(0, X(o.from)),
        text: o.from >= za.from ? `Camera only after ${fmtTimeAmPm(o.from)}` : `Camera only until ${fmtTimeAmPm(o.to)}`,
        w: Math.min(W, X(o.to)) - Math.max(0, X(o.from)),
      }));
  }, [focused, webcam, p.outages, za, W]);

  const sel = p.sel;
  const red = p.phase !== "select";
  const selX = sel ? { a: X(sel.from), b: X(sel.to) } : null;

  /* ticks */
  const step = tickStep(W / za.span, p.wide ? 70 : 90);
  const ticks: number[] = [];
  for (let t = Math.ceil(za.from / step) * step; t <= za.to; t += step) {
    if (holes.some((h) => t > h.from && t < h.to)) continue;
    const x = X(t);
    if (x > W - 40) continue;
    if (selX && (Math.abs(x - selX.a) < 52 || Math.abs(x - selX.b) < 52)) continue;
    ticks.push(t);
  }

  const marks = manifest.markers.filter((m) => m.epoch_ms >= za.from && m.epoch_ms <= za.to && !inHoleStrict(holes, m.epoch_ms));

  return (
    <div className="relative" style={{ marginTop: top - 26, height: zH + 56 }}>
      <div
        data-strip
        className="absolute left-0 top-0 cursor-crosshair overflow-hidden rounded-[7px] bg-sunk"
        style={{ width: W, height: zH, touchAction: "none" }}
        {...handlers}
      >
        {tiles.map((tile) => (
          <img
            key={`${tile.i}-${tile.track.id}`}
            src={thumbUrl(manifest.id, tile.track.id, tile.t, ZOOM_THUMB_W)}
            alt=""
            draggable={false}
            className="pointer-events-none absolute top-0 h-full object-cover"
            style={{ left: (tile.i * W) / n, width: W / n - 1, opacity: tile.dim ? 0.55 : 1 }}
            onError={(e) => {
              e.currentTarget.style.visibility = "hidden";
            }}
          />
        ))}
        {p.idle.map((s) => {
          const a = Math.max(0, X(s.from));
          const b = Math.min(W, X(s.to));
          if (b - a < 1) return null;
          return (
            <div
              key={s.from}
              className="pointer-events-none absolute top-0 grid h-full place-items-center overflow-hidden bg-[rgba(245,243,238,.62)]"
              style={{ left: a, width: b - a }}
            >
              {b - a > 64 && <span className="whitespace-nowrap text-[11.5px] font-semibold text-ink-2">Away {fmtSpan(s.to - s.from)}</span>}
            </div>
          );
        })}
        {cameraOnly.map((c) =>
          c.w > 40 ? (
            <div key={c.text} className="pointer-events-none absolute top-0 flex h-full items-center pl-2.5" style={{ left: c.x, width: c.w }}>
              <span className="whitespace-nowrap text-[11.5px] font-semibold text-white [text-shadow:0_1px_3px_rgba(0,0,0,.7)]">{c.text}</span>
            </div>
          ) : null,
        )}
        {sel && selX && (
          <>
            <div className="pointer-events-none absolute left-0 top-0 h-full" style={{ width: Math.max(0, selX.a), background: VEIL }} />
            <div className="pointer-events-none absolute right-0 top-0 h-full" style={{ left: Math.min(W, selX.b), background: VEIL }} />
          </>
        )}
      </div>

      {/* seams */}
      {holes.map((h) =>
        h.from >= za.from && h.from <= za.to ? (
          <div key={h.from} className="pointer-events-none absolute w-[2px] bg-rec" style={{ left: X(h.from) - 1, top: -4, height: zH + 8 }} />
        ) : null,
      )}

      {/* marks */}
      {!sel &&
        marks.map((m) => (
          <div key={m.epoch_ms}>
            <div className="pointer-events-none absolute w-[2px] bg-canary-deep" style={{ left: X(m.epoch_ms) - 1, top: -4, height: zH + 8 }} />
            {X(m.epoch_ms) < W - 120 && (
              <div
                className="pointer-events-none absolute flex h-[18px] items-center whitespace-nowrap rounded-[5px] bg-canary px-1.5 text-[11px] font-bold text-canary-ink"
                style={{ left: X(m.epoch_ms) + 4, top: 4 }}
              >
                {m.label.trim() || "Mark"} · {fmtTimeAmPm(m.epoch_ms)}
              </div>
            )}
          </div>
        ))}

      {/* selection */}
      {sel && selX && (
        <>
          <SelBox x0={selX.a} x1={selX.b} top={-3} h={zH + 6} red={red} collapsing={p.phase === "collapsing"} radius={8} />
          {[
            { x: selX.a, t: sel.from, edge: "from" as const },
            { x: selX.b, t: sel.to, edge: "to" as const },
          ].map((c) =>
            c.x >= -40 && c.x <= W + 40 ? (
              <div
                key={c.edge}
                className={`num pointer-events-none absolute grid h-5 w-[84px] place-items-center rounded-chip text-[11px] font-semibold ${
                  red ? "bg-rec text-white" : "bg-canary text-canary-ink"
                }`}
                style={{ left: Math.min(W - 84, Math.max(0, c.x - 42)), top: zH + 6 }}
              >
                {fmtClock12(c.t)}
              </div>
            ) : null,
          )}
          {!red && p.snapHint && (
            <div
              className="pointer-events-none absolute whitespace-nowrap text-[11.5px] text-ink-3"
              style={
                p.snapHint.edge === "from"
                  ? { left: Math.min(W - 200, Math.max(0, selX.a - 42)), top: zH + 31 }
                  : { left: Math.max(0, Math.min(W - 200, selX.b - 42)), top: zH + 31 }
              }
            >
              {snapLabel(p.snapHint.kind)}
            </div>
          )}
        </>
      )}
      {p.edge !== null && !sel && p.edge >= za.from && p.edge <= za.to && (
        <div className="pointer-events-none absolute w-[2px] bg-canary-deep" style={{ left: X(p.edge) - 1, top: -4, height: zH + 8 }} />
      )}

      {/* ticks */}
      {ticks.map((t) => (
        <div key={t} className="num pointer-events-none absolute whitespace-nowrap text-[10.5px] text-ink-3" style={{ left: X(t) + 3, top: zH + 4 }}>
          {fmtTick(t)}
        </div>
      ))}

      {/* playhead */}
      {!sel && <div ref={phRef} className="pointer-events-none absolute left-0 w-[2px] bg-ink" style={{ top: -4, height: zH + 8, marginLeft: -1 }} />}
    </div>
  );
}
