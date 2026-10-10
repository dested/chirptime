/**
 * The preview's geometry: where each picture sits in the output frame, the
 * same arithmetic `export.rs` uses (build_plan, corner_xy, mask_pgm), in
 * frame pixels so the preview can be any size. Pure.
 */
import type { SessionTrack } from "../types";
import type { CameraOpts, Corner, Layout } from "./types";

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Piece {
  track: SessionTrack;
  box: Box;
  fit: "contain" | "cover" | "fill";
  /** the corner camera: draggable, masked */
  corner: boolean;
  radius: number;
}

/** Output aspect (w / h) of a layout. */
export function layoutAspect(layout: Layout): number {
  return layout === "short" ? 9 / 16 : 16 / 9;
}

export function trackDims(t: SessionTrack): { w: number; h: number } {
  return { w: Math.max(1, t.width || 1280), h: Math.max(1, t.height || 720) };
}

/** Pixel area on the desk (monitor size), else the encoded size. */
export function screenArea(t: SessionTrack): number {
  return t.monitor ? t.monitor.w * t.monitor.h : t.width * t.height;
}

export function largestScreen(screens: readonly SessionTrack[]): SessionTrack | null {
  let best: SessionTrack | null = null;
  for (const t of screens) if (!best || screenArea(t) > screenArea(best)) best = t;
  return best;
}

export function cornerCameraBox(fw: number, fh: number, cam: CameraOpts): Box {
  const size = Math.min(0.5, Math.max(0.1, cam.size));
  const w = fw * size;
  const h = (w * 9) / 16;
  const [bw, bh] = cam.shape === "circle" ? [h, h] : [w, h];
  const { x, y } = cornerXY(fw, fh, bw, bh, cam.corner);
  return { x, y, w: bw, h: bh };
}

export function cornerMargin(fw: number, fh: number): number {
  return Math.min(fw, fh) * 0.04;
}

export function cornerXY(fw: number, fh: number, bw: number, bh: number, corner: Corner): { x: number; y: number } {
  const m = cornerMargin(fw, fh);
  const left = m;
  const right = fw - bw - m;
  const top = m;
  const bottom = fh - bh - m;
  switch (corner) {
    case "top-left":
      return { x: left, y: top };
    case "top-right":
      return { x: right, y: top };
    case "bottom-left":
      return { x: left, y: bottom };
    case "bottom-right":
      return { x: right, y: bottom };
  }
}

/** The corner a dragged box's centre is nearest to. */
export function nearestCorner(fw: number, fh: number, box: Box): Corner {
  const cx = box.x + box.w / 2;
  const cy = box.y + box.h / 2;
  const v = cy < fh / 2 ? "top" : "bottom";
  const h = cx < fw / 2 ? "left" : "right";
  return `${v}-${h}`;
}

/** The camera mask's corner radius: rounded is 14% of the short side, a circle half. */
export function cameraRadius(box: Box, cam: CameraOpts): number {
  return cam.shape === "circle" ? Math.min(box.w, box.h) / 2 : Math.min(box.w, box.h) * 0.14;
}

function contain(src: { w: number; h: number }, bx: Box): Box {
  const s = Math.min(bx.w / src.w, bx.h / src.h);
  const w = src.w * s;
  const h = src.h * s;
  return { x: bx.x + (bx.w - w) / 2, y: bx.y + (bx.h - h) / 2, w, h };
}

/** Every screen at its real position, the whole desk contained with a 3% margin. */
function deskBoxes(screens: readonly SessionTrack[], fw: number, fh: number): { track: SessionTrack; box: Box }[] {
  let run = 0;
  const rects = screens.map((t) => {
    const m = t.monitor;
    const r = m ? { x: m.x, y: m.y, w: m.w, h: m.h } : { x: run, y: 0, w: trackDims(t).w, h: trackDims(t).h };
    run += r.w;
    return { track: t, r };
  });
  if (!rects.length) return [];
  const x0 = Math.min(...rects.map((q) => q.r.x));
  const y0 = Math.min(...rects.map((q) => q.r.y));
  const x1 = Math.max(...rects.map((q) => q.r.x + q.r.w));
  const y1 = Math.max(...rects.map((q) => q.r.y + q.r.h));
  const margin = Math.min(fw, fh) * 0.03;
  const s = Math.min((fw - 2 * margin) / Math.max(1, x1 - x0), (fh - 2 * margin) / Math.max(1, y1 - y0));
  const ox = (fw - (x1 - x0) * s) / 2;
  const oy = (fh - (y1 - y0) * s) / 2;
  return rects.map(({ track, r }) => ({ track, box: { x: ox + (r.x - x0) * s, y: oy + (r.y - y0) * s, w: r.w * s, h: r.h * s } }));
}

export interface ComposeInput {
  layout: Layout;
  fw: number;
  fh: number;
  /** the screen shown (Devlog, Short, Side by side) */
  screen: SessionTrack | null;
  /** every screen with a picture somewhere in the range (Desk), placed once for the whole export */
  deskScreens: readonly SessionTrack[];
  /** false where no shown screen has footage: the camera takes the frame */
  screenNow: boolean;
  /** null when the camera is off or missing */
  camera: SessionTrack | null;
  cam: CameraOpts;
}

/** What the frame shows at one instant, bottom layer first. */
export function compose(c: ComposeInput): Piece[] {
  const { fw, fh, camera } = c;
  const frame: Box = { x: 0, y: 0, w: fw, h: fh };
  const out: Piece[] = [];
  const anyScreen = c.screenNow && (c.layout === "desk" ? c.deskScreens.length > 0 : c.screen !== null);

  // No screen to show: the camera fills the frame.
  if (!anyScreen) {
    if (camera) out.push({ track: camera, box: frame, fit: "cover", corner: false, radius: 0 });
    return out;
  }

  let screenBox = frame;
  let screenFit: Piece["fit"] = "contain";
  let camBox: Box | null = null;
  if (c.layout === "short") {
    screenFit = "cover";
    if (camera) {
      const top = fh * 0.6;
      screenBox = { x: 0, y: 0, w: fw, h: top };
      camBox = { x: 0, y: top, w: fw, h: fh - top };
    }
  } else if (c.layout === "side-by-side" && camera) {
    const half = fw / 2;
    screenBox = { x: 0, y: 0, w: half, h: fh };
    camBox = { x: half, y: 0, w: fw - half, h: fh };
  }

  if (c.layout === "desk") {
    for (const d of deskBoxes(c.deskScreens, fw, fh)) out.push({ track: d.track, box: d.box, fit: "fill", corner: false, radius: 0 });
  } else if (c.screen) {
    const box = screenFit === "contain" ? contain(trackDims(c.screen), screenBox) : screenBox;
    out.push({ track: c.screen, box, fit: screenFit, corner: false, radius: 0 });
  }

  if (camera) {
    if (camBox) {
      out.push({ track: camera, box: camBox, fit: "cover", corner: false, radius: 0 });
    } else {
      const box = cornerCameraBox(fw, fh, c.cam);
      out.push({ track: camera, box, fit: "cover", corner: true, radius: cameraRadius(box, c.cam) });
    }
  }
  return out;
}
