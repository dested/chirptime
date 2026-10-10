/**
 * Mock IPC handlers owned by `widget` (round 2). Return `UNHANDLED` for any
 * command this file doesn't answer; the core mock in ../mockIpc.ts tries the
 * extras before its own switch.
 */
import { UNHANDLED, type MockCtx, type MockResult } from "./ctx";
import type { Cut, SessionManifest } from "../../types";

/* ---- cursor track --------------------------------------------------------- *
 * `get_cursor_track` synthesised for the fixture sessions, shaped like the
 * real thing (cursor.rs): ascending, cut ranges excluded, a record every
 * 100 ms while the mouse moves and once a second while it holds still. The
 * mouse dwells on one screen for a few minutes at a time, then crosses to
 * another, so Player follow and export Follow mouse have switches to make.
 * -------------------------------------------------------------------------- */

interface CursorSample {
  t: number;
  x: number;
  y: number;
}

const DWELL_MS = 4 * 60_000;
const MOVE_MS = 100;
const STILL_MS = 1_000;

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

function cutHidesCursor(c: Cut, t: number): boolean {
  return (
    c.state !== "undone" &&
    t >= c.from_epoch_ms &&
    t < c.to_epoch_ms &&
    (!c.tracks || c.tracks.some((id) => id.startsWith("screen-")))
  );
}

function cursorTrack(m: SessionManifest, from: number, to: number): CursorSample[] {
  const screens = m.tracks.filter((t) => t.kind === "screen" && t.monitor);
  if (screens.length === 0) return [];
  const end = m.ended_epoch_ms ?? Date.now();
  const lo = Math.max(from, m.started_epoch_ms);
  const hi = Math.min(to, end);
  // Nobody moves the mouse while away; the cursor holds where it was left.
  const idle = screens[0].gaps.filter((g) => g.reason.startsWith("idle"));
  const seed = hash(m.id);
  const out: CursorSample[] = [];
  let t = lo - (lo % MOVE_MS);
  let last: { x: number; y: number } | null = null;
  while (t <= hi) {
    const away = idle.find((g) => t >= g.from_epoch_ms && t < g.to_epoch_ms);
    const at = away ? away.from_epoch_ms : t;
    const block = Math.floor((at - m.started_epoch_ms) / DWELL_MS);
    const mon = screens[hash(`${seed}:${block}`) % screens.length].monitor;
    if (mon) {
      const phase = (at - m.started_epoch_ms) / 1000;
      const x = Math.round(mon.x + mon.w * (0.5 + 0.35 * Math.sin(phase / 7 + block)));
      const y = Math.round(mon.y + mon.h * (0.5 + 0.35 * Math.cos(phase / 11 + block)));
      if (t >= lo && !m.cuts.some((c) => cutHidesCursor(c, t))) {
        if (!away || !last || t % STILL_MS === 0) out.push({ t, x, y });
      }
      last = { x, y };
    }
    t += MOVE_MS;
  }
  return out;
}

/* ---- dispatcher ----------------------------------------------------------- */

function num(a: Record<string, unknown>, k: string): number {
  const v = a[k];
  if (typeof v !== "number" || !Number.isFinite(v)) throw new Error(`mock: ${k} must be a number`);
  return Math.round(v);
}

function text(a: Record<string, unknown>, k: string): string {
  const v = a[k];
  if (typeof v !== "string") throw new Error(`mock: ${k} must be a string`);
  return v;
}

export function handleWidget(cmd: string, a: Record<string, unknown>, ctx: MockCtx): MockResult {
  switch (cmd) {
    case "get_cursor_track":
      return cursorTrack(ctx.session(text(a, "sessionId")), num(a, "fromMs"), num(a, "toMs"));
    default:
      return UNHANDLED;
  }
}
