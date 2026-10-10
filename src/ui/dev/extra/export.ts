/**
 * Mock IPC handlers owned by `export` (round 2). Return `UNHANDLED` for any
 * command this file doesn't answer; the core mock in ../mockIpc.ts tries the
 * extras before its own switch.
 *
 * estimate_export answers with the backend's arithmetic, roughly; start_export
 * runs a fake job that emits `export-progress` for about 5 s and then adds a
 * row; list/delete/reveal work on an in-memory index seeded with three
 * exports of the fixture sessions.
 */
import { UNHANDLED, type MockCtx, type MockResult } from "./ctx";
import type { SessionManifest } from "../../types";
import { sessionBounds } from "../../session/engine";
import { sessionHoles } from "../../session/cuts";
import { rangeBytes } from "../../player/timeline";
import type { ExportEstimate, ExportProgress, ExportRecord, Layout, SizeMode, Speed } from "../../export/types";

const JOB_MS = 5_000;
const TICK_MS = 250;
const SHARE_BYTES = 25e6;

const records: ExportRecord[] = [];
let seeded = false;
const timers = new Map<string, number>();

/* ---- argument readers ------------------------------------------------------- */

function obj(v: unknown): Record<string, unknown> {
  return typeof v === "object" && v !== null ? Object.fromEntries(Object.entries(v)) : {};
}
function str(a: Record<string, unknown>, k: string): string {
  const v = a[k];
  if (typeof v !== "string") throw new Error(`mock IPC: ${k} must be a string`);
  return v;
}
function num(a: Record<string, unknown>, k: string): number {
  const v = a[k];
  if (typeof v !== "number") throw new Error(`mock IPC: ${k} must be a number`);
  return v;
}
function one<T extends string>(list: readonly T[], v: unknown, d: T): T {
  return list.find((x) => x === v) ?? d;
}

interface MockReq {
  sessionId: string;
  fromMs: number;
  toMs: number;
  layout: Layout;
  speed: Speed;
  size: SizeMode;
  skipIdle: boolean;
  screen: string | null;
  micDb: number | null;
  rawTracks: string[] | null;
  title: string | null;
}

function readReq(a: Record<string, unknown>): MockReq {
  const r = obj(a.req);
  const sound = obj(r.sound);
  const raw = r.rawTracks;
  return {
    sessionId: str(r, "sessionId"),
    fromMs: num(r, "fromMs"),
    toMs: num(r, "toMs"),
    layout: one(["devlog", "short", "side-by-side", "desk"] as const, r.layout, "devlog"),
    speed: one(["1x", "2x", "4x", "timelapse"] as const, r.speed, "1x"),
    size: one(["original", "1080p", "share"] as const, r.size, "1080p"),
    skipIdle: r.skipIdle === true,
    screen: typeof r.screen === "string" ? r.screen : null,
    micDb: typeof sound.micDb === "number" ? sound.micDb : null,
    rawTracks: Array.isArray(raw) ? raw.filter((x): x is string => typeof x === "string") : null,
    title: typeof r.title === "string" && r.title.trim() ? r.title : null,
  };
}

/* ---- the estimate ------------------------------------------------------------ */

const FACTOR: Record<Speed, number> = { "1x": 1, "2x": 2, "4x": 4, timelapse: 30 };
const LAYOUT_TITLE: Record<Layout, string> = { devlog: "Devlog", short: "Short", "side-by-side": "Side by side", desk: "Desk" };

/** Kept time in [from, to]: minus whole-session cuts, minus idle when skipping. */
function keptMs(m: SessionManifest, from: number, to: number, skipIdle: boolean): number {
  const b = sessionBounds(m);
  const f = Math.max(from, b.start);
  const t = Math.min(to, b.end);
  let ms = Math.max(0, t - f);
  const minus = (a: number, z: number) => {
    ms -= Math.max(0, Math.min(z, t) - Math.max(a, f));
  };
  for (const h of sessionHoles(m)) minus(h.from, h.to);
  if (skipIdle) {
    const lead = m.tracks.find((x) => x.kind === "webcam") ?? m.tracks.find((x) => x.kind === "screen");
    for (const g of lead?.gaps ?? []) if (/^idle|^paused/.test(g.reason)) minus(g.from_epoch_ms, g.to_epoch_ms);
  }
  return Math.max(0, ms);
}

function fmtMinutes(ms: number): string {
  const m = Math.round(ms / 60_000);
  return m < 1 ? "under a minute" : `${m} min`;
}

function estimate(ctx: MockCtx, r: MockReq): ExportEstimate {
  const m = ctx.session(r.sessionId);
  const sourceMs = keptMs(m, r.fromMs, r.toMs, r.skipIdle);
  const outputMs = Math.round(sourceMs / FACTOR[r.speed]);
  const base: ExportEstimate = { outputMs, sourceMs, bytes: 0, width: 1920, height: 1080, encoder: "x264", encodeSeconds: 0, notes: [], error: null };
  if (r.toMs - r.fromMs < 1_000) return { ...base, error: "Pick at least a second of this session to export" };
  if (sourceMs <= 0) return { ...base, error: "Nothing is left in this range once deleted time is taken out" };

  if (r.rawTracks) {
    if (!r.rawTracks.length) return { ...base, error: "Pick at least one track" };
    return { ...base, bytes: rangeBytes(m, r.fromMs, r.toMs, r.rawTracks), encoder: "copy", encodeSeconds: Math.ceil(sourceMs / 600_000) };
  }

  const tallest = Math.max(480, ...m.tracks.filter((t) => t.kind === "screen" || t.kind === "webcam").map((t) => t.height));
  let p = r.size === "original" ? Math.min(2160, tallest) : 1080;
  const secs = outputMs / 1000;
  let kbps = p >= 1080 ? 6000 : p >= 720 ? 3500 : 1500;
  if (r.size === "share") {
    const budget = (SHARE_BYTES * 8 * 0.95) / 1000 / Math.max(1, secs) - 96;
    const tiers: [number, number][] = [
      [1080, 1800],
      [720, 800],
      [480, 280],
    ];
    const fit = tiers.find(([, min]) => budget >= min);
    if (!fit) {
      const longest = ((SHARE_BYTES * 8 * 0.95) / ((280 + 96) * 1000)) * 1000;
      return { ...base, error: `Too long for 25 MB: ${fmtMinutes(outputMs)} would drop below 480p. Share fits about ${fmtMinutes(longest)}; pick a shorter range or a faster speed` };
    }
    p = fit[0];
    kbps = Math.min(6000, budget);
  }
  const long = Math.round((p * 16) / 9 / 2) * 2;
  const [width, height] = r.layout === "short" ? [p, long] : [long, p];
  const notes: string[] = [];
  if (r.speed === "timelapse") notes.push("Timelapse has no sound");
  const audio = r.speed === "timelapse" ? 0 : 160;
  return {
    ...base,
    width,
    height,
    bytes: Math.round(((kbps + audio) * 1000 * secs) / 8),
    encodeSeconds: Math.max(1, Math.round(sourceMs / 1000 / (r.speed === "timelapse" ? 60 : 6))),
    notes,
  };
}

/* ---- the index ------------------------------------------------------------- */

function stamp(epochMs: number): string {
  const d = new Date(epochMs);
  const p2 = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())} ${p2(d.getHours())}.${p2(d.getMinutes())}`;
}

function record(id: string, m: SessionManifest, from: number, to: number, layout: string, title: string, durationMs: number, bytes: number, createdEpochMs: number): ExportRecord {
  const raw = layout === "raw";
  return {
    id,
    sessionId: m.id,
    from,
    to,
    layout,
    path: `exports/${stamp(from)} ${title}${raw ? "" : ".mp4"}`,
    bytes,
    durationMs,
    createdEpochMs,
    title,
    thumb: null,
  };
}

function seed(ctx: MockCtx) {
  if (seeded) return;
  seeded = true;
  const now = Date.now();
  // the three longest fixture sessions, so the rows have pictures
  const length = (id: string) => {
    const b = sessionBounds(ctx.session(id));
    return b.end - b.start;
  };
  const ids = [...ctx.sessionIds()].sort((x, y) => length(y) - length(x));
  const picks: [number, number, number, string, string, number][] = [
    // session index, start offset min, length min, layout, title, MB
    [0, 12, 24, "devlog", "Devlog", 1_870],
    [1, 4, 6, "short", "Short", 96],
    [2, 2, 18, "raw", "Raw tracks", 2_410],
  ];
  picks.forEach(([si, off, len, layout, title, mb], i) => {
    const id = ids[si] ?? ids[0];
    if (!id) return;
    const m = ctx.session(id);
    const b = sessionBounds(m);
    const from = Math.min(b.end - 60_000, b.start + off * 60_000);
    const to = Math.min(b.end, from + len * 60_000);
    records.push(record(`exp-seed-${i}`, m, from, to, layout, title, to - from, mb * 1e6, now - (i + 1) * 26 * 3_600_000));
  });
}

/* ---- the fake job ---------------------------------------------------------- */

function startJob(ctx: MockCtx, r: MockReq): string {
  const est = estimate(ctx, r);
  if (est.error) throw new Error(est.error);
  const jobId = `exp-${Date.now()}`;
  const started = Date.now();
  const emit = (p: ExportProgress) => ctx.emit("export-progress", p);
  emit({ jobId, fraction: 0, etaSeconds: null, state: "running" });
  const t = window.setInterval(() => {
    const f = Math.min(1, (Date.now() - started) / JOB_MS);
    if (f < 1) {
      emit({ jobId, fraction: f, etaSeconds: (JOB_MS * (1 - f)) / 1000, state: "running" });
      return;
    }
    window.clearInterval(t);
    timers.delete(jobId);
    const m = ctx.session(r.sessionId);
    const title = r.title ?? (r.rawTracks ? "Raw tracks" : LAYOUT_TITLE[r.layout]);
    const rec = record(jobId, m, r.fromMs, r.toMs, r.rawTracks ? "raw" : r.layout, title, r.rawTracks ? est.sourceMs : est.outputMs, est.bytes, Date.now());
    records.unshift(rec);
    emit({ jobId, fraction: 1, etaSeconds: 0, state: "done", outputPath: `C:\\Users\\you\\Videos\\ChirpTime\\${rec.path.replace(/\//g, "\\")}` });
  }, TICK_MS);
  timers.set(jobId, t);
  return jobId;
}

export function handleExport(cmd: string, a: Record<string, unknown>, ctx: MockCtx): MockResult {
  switch (cmd) {
    case "estimate_export":
      return estimate(ctx, readReq(a));
    case "start_export":
      return startJob(ctx, readReq(a));
    case "cancel_export": {
      const jobId = str(a, "jobId");
      const t = timers.get(jobId);
      if (t === undefined) throw new Error("That export is not running");
      window.clearInterval(t);
      timers.delete(jobId);
      ctx.emit("export-progress", { jobId, fraction: 0, etaSeconds: null, state: "cancelled" } satisfies ExportProgress);
      return null;
    }
    case "list_exports":
      seed(ctx);
      return records.map((r) => ({ ...r }));
    case "delete_export": {
      seed(ctx);
      const id = str(a, "id");
      const i = records.findIndex((r) => r.id === id);
      if (i < 0) throw new Error("That export is already gone");
      records.splice(i, 1);
      return null;
    }
    case "reveal_export": {
      seed(ctx);
      const id = str(a, "id");
      if (!records.some((r) => r.id === id)) throw new Error("That export is gone");
      return null;
    }
    default:
      return UNHANDLED;
  }
}
