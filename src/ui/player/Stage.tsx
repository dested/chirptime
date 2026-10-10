/**
 * The stage: the one dark surface. Focus shows one screen large with the camera
 * and a mini desk map under it; Desk shows every monitor where it really sits,
 * with the camera in the largest empty corner.
 *
 * Only the tracks on screen are attached to the engine, so Focus decodes two
 * videos, not five. The mini map is thumbnails, refreshed every 30 s of footage.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { SessionManifest, SessionTrack } from "../types";
import type { EngineSnapshot, SyncEngine, TrackState } from "../session/engine";
import { TrackSurface } from "../session/TrackSurface";
import { thumbUrl } from "../hooks/mediaUrl";
import { screenNumber, screenTracks, thumbFor } from "./timeline";

export type View = "focus" | "desk";

interface Props {
  engine: SyncEngine;
  manifest: SessionManifest;
  view: View;
  focusedId: string | null;
  wide: boolean;
  onFocus: (id: string) => void;
  onView: (v: View) => void;
}

const PAD = 10;

function useSize<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) setSize({ w: r.width, h: r.height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, size] as const;
}

/** The engine's per-track state at 5 Hz, and the playhead rounded for thumbnails. */
function useTrackStates(engine: SyncEngine): { states: Record<string, TrackState>; thumbT: number } {
  const pick = (s: EngineSnapshot) => {
    const states: Record<string, TrackState> = {};
    for (const [id, d] of Object.entries(s.tracks)) states[id] = d.state;
    return { states, thumbT: s.masterEpochMs - (s.masterEpochMs % 10_000) };
  };
  const [v, setV] = useState(() => pick(engine.snapshot()));
  useEffect(() => {
    setV(pick(engine.snapshot()));
    return engine.subscribeState((s) => {
      const next = pick(s);
      setV((prev) =>
        prev.thumbT === next.thumbT && Object.keys(next.states).every((k) => prev.states[k] === next.states[k]) ? prev : next,
      );
    });
  }, [engine]);
  return v;
}

export function Stage({ engine, manifest, view, focusedId, wide, onFocus, onView }: Props) {
  const [ref, size] = useSize<HTMLDivElement>();
  const { states, thumbT } = useTrackStates(engine);
  const screens = useMemo(() => screenTracks(manifest), [manifest]);
  const webcam = manifest.tracks.find((t) => t.kind === "webcam") ?? null;
  const focused = screens.find((t) => t.id === focusedId) ?? screens[0] ?? webcam;

  return (
    <div ref={ref} className="relative min-h-0 flex-1 overflow-hidden rounded-card bg-mat">
      {size.w > 0 &&
        (view === "desk" || !focused ? (
          <DeskView engine={engine} manifest={manifest} screens={screens} webcam={webcam} states={states} w={size.w} h={size.h} onPick={onFocus} />
        ) : (
          <FocusView
            engine={engine}
            manifest={manifest}
            screens={screens}
            webcam={webcam}
            focused={focused}
            states={states}
            thumbT={thumbT}
            w={size.w}
            h={size.h}
            rowH={wide ? 110 : 78}
            onFocus={onFocus}
          />
        ))}
      <ViewSwitch
        view={view}
        onView={onView}
        rowMid={view === "focus" && focused ? focusGeom(focused, screens.length > 0, size.w, size.h, wide ? 110 : 78).rowMid : null}
      />
    </div>
  );
}

function ViewSwitch({ view, onView, rowMid }: { view: View; onView: (v: View) => void; rowMid: number | null }) {
  // In Focus it sits mid-row under the screen; in Desk, bottom right.
  const style = rowMid !== null ? { right: PAD, top: rowMid - 15 } : { right: PAD, bottom: PAD };
  const btn = (v: View, label: string) => (
    <button
      className={`h-[26px] rounded-[7px] px-2.5 text-[12px] font-semibold transition-colors duration-150 ${
        view === v ? "bg-[#F3F0E8] text-ink" : "text-[#F3F0E8]/60 hover:text-[#F3F0E8]"
      }`}
      onClick={() => onView(v)}
    >
      {label}
    </button>
  );
  return (
    <div className="absolute z-10 flex rounded-[9px] bg-white/[0.08] p-0.5" style={style}>
      {btn("focus", "Focus")}
      {btn("desk", "Desk")}
    </div>
  );
}

/* ---- Focus ------------------------------------------------------------- */

interface FocusProps {
  engine: SyncEngine;
  manifest: SessionManifest;
  screens: SessionTrack[];
  webcam: SessionTrack | null;
  focused: SessionTrack;
  states: Record<string, TrackState>;
  thumbT: number;
  w: number;
  h: number;
  rowH: number;
  onFocus: (id: string) => void;
}

function aspect(t: SessionTrack): number {
  if (t.monitor && t.monitor.w && t.monitor.h) return t.monitor.w / t.monitor.h;
  return t.width && t.height ? t.width / t.height : 16 / 9;
}

/**
 * The focused screen at its monitor's aspect, the row under it, and the two
 * centred as a block when the screen is width-bound, so no dead band opens up
 * between them.
 */
function focusGeom(focused: SessionTrack, hasScreens: boolean, w: number, h: number, rowH: number) {
  const showRow = hasScreens && focused.kind === "screen";
  const areaW = w - PAD * 2;
  const areaH = h - PAD * 2 - (showRow ? rowH + PAD : 0);
  const a = aspect(focused);
  let bw = areaW;
  let bh = bw / a;
  if (bh > areaH) {
    bh = areaH;
    bw = bh * a;
  }
  const by = PAD + Math.max(0, (areaH - bh) / 2);
  const y2 = by + bh + PAD;
  return { showRow, bx: PAD + (areaW - bw) / 2, by, bw, bh, y2, rowMid: showRow ? y2 + rowH / 2 : h - PAD - 15 };
}

function FocusView({ engine, manifest, screens, webcam, focused, states, thumbT, w, h, rowH, onFocus }: FocusProps) {
  const { showRow, bx, by, bw, bh, y2 } = focusGeom(focused, screens.length > 0, w, h, rowH);
  const camW = Math.round((rowH * 16) / 9);
  const n = screens.length;

  return (
    <>
      <TrackSurface
        engine={engine}
        track={focused}
        state={states[focused.id]}
        className="rounded-[4px]"
        style={{ left: bx, top: by, width: bw, height: bh, boxShadow: "0 0 0 1.5px rgba(255,216,77,.9)" }}
      />
      {showRow && (
        <>
          {webcam ? (
            <TrackSurface
              engine={engine}
              track={webcam}
              state={states[webcam.id]}
              fit="cover"
              label="Camera"
              className="rounded-[6px]"
              style={{ left: PAD, top: y2, width: camW, height: rowH }}
            />
          ) : null}
          <MiniDesk
            manifest={manifest}
            screens={screens}
            focusedId={focused.id}
            thumbT={thumbT}
            x={PAD + (webcam ? camW + 16 : 0)}
            y={y2 + 4}
            h={rowH - 8}
            onFocus={onFocus}
          />
          <div
            className="pointer-events-none absolute flex flex-col justify-center gap-1 whitespace-nowrap text-[12px] text-[#F3F0E8]/60"
            style={{ left: PAD + (webcam ? camW + 16 : 0) + miniWidth(manifest, rowH - 8) + 14, top: y2, height: rowH }}
          >
            <span>
              <span className="font-semibold text-[#F3F0E8]">Screen {screenNumber(focused)}</span>
              {n > 1 ? ` · press 1–${Math.min(n, 9)} to switch, D for the desk` : " · D for the desk"}
            </span>
          </div>
        </>
      )}
    </>
  );
}

function miniWidth(m: SessionManifest, h: number): number {
  return m.desktop.h ? (m.desktop.w * h) / m.desktop.h : 0;
}

function MiniDesk({
  manifest,
  screens,
  focusedId,
  thumbT,
  x,
  y,
  h,
  onFocus,
}: {
  manifest: SessionManifest;
  screens: SessionTrack[];
  focusedId: string;
  thumbT: number;
  x: number;
  y: number;
  h: number;
  onFocus: (id: string) => void;
}) {
  const d = manifest.desktop;
  if (!d.w || !d.h) return null;
  const sc = h / d.h;
  const g = 1.5;
  return (
    <>
      {screens
        .filter((t) => t.monitor)
        .map((t) => {
          const m = t.monitor;
          if (!m) return null;
          const on = t.id === focusedId;
          const tw = m.w * sc - 2 * g;
          const th = m.h * sc - 2 * g;
          const at = thumbFor(manifest, t, thumbT, 30_000);
          return (
            <button
              key={t.id}
              title={`Screen ${screenNumber(t)}`}
              className="absolute overflow-hidden rounded-[2px] bg-[#1d1d1b] transition-opacity duration-150 hover:opacity-100"
              style={{
                left: x + (m.x - d.x) * sc + g,
                top: y + (m.y - d.y) * sc + g,
                width: tw,
                height: th,
                boxShadow: on ? "0 0 0 1.5px var(--color-canary)" : undefined,
                opacity: on ? 1 : 0.75,
              }}
              onClick={() => onFocus(t.id)}
            >
              {at !== null && (
                <img
                  src={thumbUrl(manifest.id, t.id, at, 160)}
                  alt=""
                  draggable={false}
                  className="h-full w-full object-cover"
                  onError={(e) => {
                    e.currentTarget.style.visibility = "hidden";
                  }}
                  onLoad={(e) => {
                    e.currentTarget.style.visibility = "visible";
                  }}
                />
              )}
              <span
                className="num absolute bottom-px left-[3px] text-[9px] font-semibold"
                style={{ color: on ? "var(--color-canary)" : "rgba(255,255,255,.7)" }}
              >
                {screenNumber(t)}
              </span>
            </button>
          );
        })}
    </>
  );
}

/* ---- Desk -------------------------------------------------------------- */

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/**
 * The biggest 16:9 camera that fits in a corner of the stage without covering a
 * monitor. Corners are tried at every size from large to small; the largest
 * fit wins. Falls back to a small camera bottom-right.
 */
export function cameraCorner(stage: Rect, monitors: Rect[], margin = 10): Rect {
  const maxW = stage.w * 0.3;
  const minW = 96;
  let best: Rect | null = null;
  for (const corner of ["tl", "tr", "bl", "br"] as const) {
    for (let cw = maxW; cw >= minW; cw -= 8) {
      const ch = (cw * 9) / 16;
      const r: Rect = {
        x: corner === "tl" || corner === "bl" ? stage.x + margin : stage.x + stage.w - margin - cw,
        y: corner === "tl" || corner === "tr" ? stage.y + margin : stage.y + stage.h - margin - ch,
        w: cw,
        h: ch,
      };
      if (!monitors.some((m) => intersects(r, { x: m.x - 4, y: m.y - 4, w: m.w + 8, h: m.h + 8 }))) {
        if (!best || r.w > best.w) best = r;
        break;
      }
    }
  }
  if (best) return best;
  const cw = Math.max(minW, stage.w * 0.16);
  return { x: stage.x + stage.w - margin - cw, y: stage.y + stage.h - margin - (cw * 9) / 16, w: cw, h: (cw * 9) / 16 };
}

function DeskView({
  engine,
  manifest,
  screens,
  webcam,
  states,
  w,
  h,
  onPick,
}: {
  engine: SyncEngine;
  manifest: SessionManifest;
  screens: SessionTrack[];
  webcam: SessionTrack | null;
  states: Record<string, TrackState>;
  w: number;
  h: number;
  onPick: (id: string) => void;
}) {
  const d = manifest.desktop;
  // leave the bottom-right free for the Focus | Desk switch
  const areaW = w - PAD * 2;
  const areaH = h - PAD * 2 - 36;
  const placed = screens.filter((t) => t.monitor);
  const sc = d.w && d.h ? Math.min(areaW / d.w, areaH / d.h) : 0;
  const ox = PAD + (areaW - d.w * sc) / 2;
  const oy = PAD + (areaH - d.h * sc) / 2;
  const rects = placed.map((t) => {
    const m = t.monitor;
    return m
      ? { t, r: { x: ox + (m.x - d.x) * sc + 2, y: oy + (m.y - d.y) * sc + 2, w: m.w * sc - 4, h: m.h * sc - 4 } }
      : null;
  });
  const cam = webcam ? cameraCorner({ x: 0, y: 0, w, h: h - 36 }, rects.flatMap((x) => (x ? [x.r] : []))) : null;

  return (
    <>
      {rects.map(
        (x) =>
          x && (
            <TrackSurface
              key={x.t.id}
              engine={engine}
              track={x.t}
              state={states[x.t.id]}
              fit="fill"
              label={`${screenNumber(x.t)}`}
              onSelect={() => onPick(x.t.id)}
              className="rounded-[3px] transition-shadow duration-150 hover:shadow-[0_0_0_1.5px_rgba(255,216,77,.9)]"
              style={{ left: x.r.x, top: x.r.y, width: x.r.w, height: x.r.h }}
            />
          ),
      )}
      {webcam && cam && (
        <TrackSurface
          engine={engine}
          track={webcam}
          state={states[webcam.id]}
          fit="cover"
          label="Camera"
          className="rounded-[6px]"
          style={{ left: cam.x, top: cam.y, width: cam.w, height: cam.h }}
        />
      )}
      <div className="pointer-events-none absolute bottom-[18px] left-[14px] text-[12px] text-[#F3F0E8]/60">
        <span className="font-semibold text-[#F3F0E8]">Desk</span> · click a screen or press 1–{Math.min(9, Math.max(1, screens.length))} to focus it
      </div>
    </>
  );
}
