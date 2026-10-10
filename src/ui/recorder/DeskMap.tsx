/**
 * The desk map: your monitors where they really sit, with the camera in the
 * largest empty corner, the same picture the player uses. In the widget it is
 * the "what will record" picker while idle and the health map while recording.
 */
import { useState } from "react";
import { fitCorner } from "../player/Stage";
import type { DeskScreen } from "./setup";
import { CheckBox, IconCamera } from "./ui";

export type TileLook =
  | { kind: "pick"; on: boolean }
  | { kind: "live"; thumb: string | null; health: "ok" | "warn" | "dead" | "paused" };

interface Props {
  screens: DeskScreen[];
  width: number;
  maxHeight?: number;
  look: (index: number) => TileLook;
  /** null hides the camera tile */
  camera: TileLook | null;
  onPick?: (index: number) => void;
  onCamera?: () => void;
  /** first run: bigger numbers, the resolution and a check per screen */
  detail?: boolean;
}

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

const PICK_BG = "#ECE9E2";
const WARN = "#E39A3B";

/** A still that falls back to the dark mat when the frame is not there (a gap, or not written yet). */
function Still({ src, dim }: { src: string | null; dim: "none" | "dead" | "paused" }) {
  const [failed, setFailed] = useState<string | null>(null);
  if (!src || failed === src) return null;
  const filter = dim === "dead" ? "grayscale(1) brightness(.45)" : dim === "paused" ? "grayscale(.6) brightness(.7)" : undefined;
  return (
    <img
      src={src}
      alt=""
      draggable={false}
      onError={() => setFailed(src)}
      className="absolute inset-0 h-full w-full object-cover"
      style={{ filter }}
    />
  );
}

export function DeskMap({ screens, width, maxHeight = Infinity, look, camera, onPick, onCamera, detail = false }: Props) {
  if (screens.length === 0) {
    return (
      <div
        className="grid place-items-center rounded-chip border border-dashed border-line text-[11.5px] text-ink-3"
        style={{ width, height: Math.round(width * 0.36) }}
      >
        No screens found
      </div>
    );
  }
  const x0 = Math.min(...screens.map((s) => s.x));
  const y0 = Math.min(...screens.map((s) => s.y));
  const bw = Math.max(...screens.map((s) => s.x + s.w)) - x0;
  const bh = Math.max(...screens.map((s) => s.y + s.h)) - y0;
  const sc = Math.min(width / bw, maxHeight / bh);
  const W = bw * sc;
  const H = bh * sc;
  const gap = detail ? 3 : 1.5;
  const boxes: (Box & { s: DeskScreen })[] = screens.map((s) => ({
    s,
    x: (s.x - x0) * sc + gap,
    y: (s.y - y0) * sc + gap,
    w: s.w * sc - 2 * gap,
    h: s.h * sc - 2 * gap,
  }));
  const desk: Box = { x: 0, y: 0, w: W, h: H };
  // The camera takes the largest empty corner of the desk, as in the player;
  // with none free it sits small at the bottom right.
  const camW = W * 0.16;
  const cam = camera
    ? (fitCorner(desk, boxes, 0, W * 0.34, W * 0.12) ?? { x: W - camW, y: H - (camW * 9) / 16, w: camW, h: (camW * 9) / 16 })
    : null;
  const radius = detail ? 4 : 3;

  return (
    <div className="relative shrink-0" style={{ width: W, height: H }}>
      {boxes.map(({ s, ...b }) => {
        const l = look(s.index);
        const pos = { left: b.x, top: b.y, width: b.w, height: b.h, borderRadius: radius };
        const label = `Screen ${s.index + 1}`;
        if (l.kind === "pick") {
          const usable = s.capturable;
          const on = l.on && usable;
          const style = on
            ? { ...pos, background: PICK_BG, boxShadow: `inset 0 0 0 ${detail ? 2 : 1.5}px var(--color-ink)` }
            : { ...pos, background: "transparent", border: "1px dashed var(--color-ink-4)" };
          const title = !usable
            ? `${label} cannot be recorded: no encoder for its graphics card`
            : on
              ? `${label}: recording. Click to leave it out`
              : `${label}: left out. Click to record it`;
          const inner = detail ? (
            <span className="text-center font-mono text-[11px] font-semibold leading-[1.3]" style={{ color: on ? "var(--color-ink)" : "var(--color-ink-4)" }}>
              {s.index + 1}
              <br />
              <span className="num text-[10px] font-normal text-ink-3">
                {s.w}×{s.h}
              </span>
            </span>
          ) : (
            <span
              className="absolute bottom-[2px] left-[3px] font-mono text-[9px] font-semibold"
              style={{ color: on ? "var(--color-ink-2)" : "var(--color-ink-4)" }}
            >
              {s.index + 1}
            </span>
          );
          return (
            <button
              key={s.index}
              title={title}
              aria-pressed={on}
              aria-label={label}
              disabled={!onPick || !usable}
              onClick={() => onPick?.(s.index)}
              className={`absolute grid place-items-center overflow-hidden transition-colors ${onPick && usable ? "hover:brightness-[.97]" : ""} ${!usable ? "cursor-not-allowed" : ""}`}
              style={style}
            >
              {inner}
              {detail && usable && (
                <span className="absolute right-1.5 top-1.5">
                  <CheckBox on={on} size={14} />
                </span>
              )}
            </button>
          );
        }
        const ring = l.health === "warn" || l.health === "dead" ? `0 0 0 1.5px ${WARN}` : undefined;
        return (
          <div key={s.index} className="absolute overflow-hidden bg-[#111]" style={{ ...pos, boxShadow: ring }} title={label}>
            <Still src={l.thumb} dim={l.health === "dead" ? "dead" : l.health === "paused" ? "paused" : "none"} />
            {l.health === "dead" && (
              <div
                className="absolute inset-0"
                style={{ background: "repeating-linear-gradient(-45deg, rgba(243,240,232,.07) 0 3px, transparent 3px 7px)" }}
              />
            )}
          </div>
        );
      })}
      {cam && camera && (
        <CameraTile box={cam} look={camera} onClick={onCamera} />
      )}
    </div>
  );
}

function CameraTile({ box, look, onClick }: { box: Box; look: TileLook; onClick?: () => void }) {
  const pos = { left: box.x, top: box.y, width: box.w, height: box.h, borderRadius: 5 };
  if (look.kind === "pick") {
    const style = look.on
      ? { ...pos, background: PICK_BG, boxShadow: "inset 0 0 0 1.5px var(--color-ink)" }
      : { ...pos, background: "transparent", border: "1px dashed var(--color-ink-4)" };
    return (
      <button
        title={look.on ? "Camera: recording. Click to leave it out" : "Camera: left out. Click to record it"}
        aria-pressed={look.on}
        aria-label="Camera"
        disabled={!onClick}
        onClick={onClick}
        className="absolute grid place-items-center"
        style={style}
      >
        <IconCamera className={look.on ? "text-ink-2" : "text-ink-4"} />
      </button>
    );
  }
  const ring = look.health === "warn" || look.health === "dead" ? `0 0 0 1.5px ${WARN}` : undefined;
  return (
    <div className="absolute overflow-hidden bg-[#111]" style={{ ...pos, boxShadow: ring }} title="Camera">
      <Still src={look.thumb} dim={look.health === "dead" ? "dead" : look.health === "paused" ? "paused" : "none"} />
    </div>
  );
}
