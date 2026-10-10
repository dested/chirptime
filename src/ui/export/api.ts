/** The export commands and the `export-progress` event, typed. */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ExportEstimate, ExportPreset, ExportProgress, ExportRecord, ExportRequest } from "./types";

/** Fired on `window` after an export is deleted, for anything counting them. */
export const EXPORTS_CHANGED = "chirptime:exports-changed";

export function estimateExport(req: ExportRequest): Promise<ExportEstimate> {
  return invoke<ExportEstimate>("estimate_export", { req });
}

/** Starts the job and returns its id; the finished export keeps that id. */
export function startExport(req: ExportRequest): Promise<string> {
  return invoke<string>("start_export", { req });
}

export function cancelExport(jobId: string): Promise<void> {
  return invoke("cancel_export", { jobId });
}

/** Newest first. */
export async function listExports(): Promise<ExportRecord[]> {
  const rows = await invoke<ExportRecord[]>("list_exports");
  return [...rows].sort((a, b) => b.createdEpochMs - a.createdEpochMs);
}

export function deleteExport(id: string): Promise<void> {
  return invoke("delete_export", { id });
}

/** Show the file in Explorer, or with `play`, open it in the default player. */
export function revealExport(id: string, play = false): Promise<void> {
  return invoke("reveal_export", { id, play });
}

export function onExportProgress(cb: (p: ExportProgress) => void): () => void {
  const off = listen<ExportProgress>("export-progress", (e) => cb(e.payload));
  return () => void off.then((f) => f()).catch(() => {});
}

/* ---- Save preset ---------------------------------------------------------- */

const PRESET_KEY = "chirptime.export.preset";

const LAYOUTS = ["devlog", "short", "side-by-side", "desk"] as const;
const CORNERS = ["top-left", "top-right", "bottom-left", "bottom-right"] as const;
const SHAPES = ["rounded", "circle"] as const;
const SPEEDS = ["1x", "2x", "4x", "timelapse"] as const;
const SIZES = ["original", "1080p", "share"] as const;

function pick<T extends string>(list: readonly T[], v: unknown, d: T): T {
  return list.find((x) => x === v) ?? d;
}
function rec(v: unknown): Record<string, unknown> {
  return typeof v === "object" && v !== null ? Object.fromEntries(Object.entries(v)) : {};
}
function numOr(v: unknown, d: number): number {
  return typeof v === "number" && Number.isFinite(v) ? v : d;
}
function dbOr(v: unknown, d: number | null): number | null {
  return v === null ? null : typeof v === "number" && Number.isFinite(v) ? v : d;
}

export const DEFAULT_PRESET: ExportPreset = {
  layout: "devlog",
  follow: true,
  camera: { enabled: true, corner: "bottom-right", shape: "rounded", size: 0.28 },
  sound: { micDb: 0, systemDb: -12, duck: true },
  speed: "1x",
  skipIdle: false,
  size: "1080p",
};

/** The saved preset, or the defaults; a broken or foreign value degrades field by field. */
export function loadPreset(): ExportPreset {
  let raw: unknown = null;
  try {
    const s = window.localStorage.getItem(PRESET_KEY);
    raw = s ? JSON.parse(s) : null;
  } catch {
    raw = null;
  }
  const r = rec(raw);
  const d = DEFAULT_PRESET;
  const c = rec(r.camera);
  const s = rec(r.sound);
  return {
    layout: pick(LAYOUTS, r.layout, d.layout),
    follow: typeof r.follow === "boolean" ? r.follow : d.follow,
    camera: {
      enabled: typeof c.enabled === "boolean" ? c.enabled : d.camera.enabled,
      corner: pick(CORNERS, c.corner, d.camera.corner),
      shape: pick(SHAPES, c.shape, d.camera.shape),
      size: Math.min(0.5, Math.max(0.1, numOr(c.size, d.camera.size))),
    },
    sound: {
      micDb: dbOr(s.micDb, d.sound.micDb),
      systemDb: dbOr(s.systemDb, d.sound.systemDb),
      duck: typeof s.duck === "boolean" ? s.duck : d.sound.duck,
    },
    speed: pick(SPEEDS, r.speed, d.speed),
    skipIdle: typeof r.skipIdle === "boolean" ? r.skipIdle : d.skipIdle,
    size: pick(SIZES, r.size, d.size),
  };
}

export function savePreset(p: ExportPreset): boolean {
  try {
    window.localStorage.setItem(PRESET_KEY, JSON.stringify(p));
    return true;
  } catch {
    return false;
  }
}
