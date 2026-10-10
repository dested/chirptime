/**
 * The Library's facts and its time map, derived from the session summaries and
 * whatever manifests have loaded. Pure: no DOM, no React, so a bun script can
 * check the layout rules (`scripts/check-library.ts`).
 *
 * The map is days × hours in wall-clock time. A session that runs past
 * midnight is split, its tail drawn on the next day's row.
 */
import type { SessionManifest, SessionMarker, SessionSummary, SessionTrack } from "../types";
import { sessionHoles, trackHoles } from "../session/cuts";
import type { Hole } from "../session/cuts";
import { outageHeadline, quietSpans, screenTracks, sessionTotals } from "../player/timeline";
import type { QuietSpan } from "../player/timeline";

/** Under this, a session is offered for clean-up (a Start pressed twice). */
export const SHORT_SESSION_MS = 60_000;

export interface SessionFacts {
  id: string;
  start: number;
  /** wall-clock end; now for the live session */
  end: number;
  live: boolean;
  /** cuts taken out */
  lengthMs: number;
  bytes: number;
  short: boolean;
  quiet: QuietSpan[];
  holes: Hole[];
  /** a screen stopped for over a minute (not away, not paused) */
  outage: { screens: number[]; at: number } | null;
  webcam: SessionTrack | null;
  /** marks outside any cut */
  marks: SessionMarker[];
  manifest: SessionManifest | null;
}

export function sessionFacts(s: SessionSummary, m: SessionManifest | null, liveId: string | null, now: number): SessionFacts {
  const live = s.id === liveId;
  const summaryEnd = s.ended_epoch_ms ?? s.started_epoch_ms + s.duration_ms;
  if (!m) {
    return {
      id: s.id,
      start: s.started_epoch_ms,
      end: live ? Math.max(now, summaryEnd) : summaryEnd,
      live,
      lengthMs: live ? Math.max(now, summaryEnd) - s.started_epoch_ms : s.duration_ms,
      bytes: s.bytes,
      short: !live && s.duration_ms < SHORT_SESSION_MS,
      quiet: [],
      holes: [],
      outage: null,
      webcam: null,
      marks: [],
      manifest: null,
    };
  }
  const totals = sessionTotals(m);
  const holes = sessionHoles(m);
  const inHole = (t: number) => holes.some((h) => t >= h.from && t < h.to);
  const end = live ? Math.max(now, totals.end) : totals.end;
  return {
    id: s.id,
    start: totals.start,
    end,
    live,
    lengthMs: live ? totals.lengthMs + (end - totals.end) : totals.lengthMs,
    bytes: totals.bytes,
    short: !live && totals.lengthMs < SHORT_SESSION_MS,
    quiet: quietSpans(m),
    holes,
    outage: outageHeadline(m),
    webcam: m.tracks.find((t) => t.kind === "webcam") ?? null,
    marks: m.markers.filter((mk) => !inHole(mk.epoch_ms)),
    manifest: m,
  };
}

/* ---- the time map --------------------------------------------------------- */

/** Local midnight at or before `t`. */
export function dayStart(t: number): number {
  const d = new Date(t);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** The next local midnight (DST-safe: built from the calendar, not +24 h). */
export function nextDay(dayMs: number): number {
  const d = new Date(dayMs);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime();
}

/** Hours since that day's local midnight, fractional. */
export function hourOfDay(t: number, day: number): number {
  return (t - day) / 3_600_000;
}

/** One session's stretch on one day's row. */
export interface Piece {
  facts: SessionFacts;
  from: number;
  to: number;
  /** the session continues on the next row */
  continues: boolean;
  /** the session began on an earlier row */
  continued: boolean;
}

export interface DayRow {
  /** local midnight */
  day: number;
  pieces: Piece[];
}

/** Rows newest day first; pieces in a row oldest first. */
export function layoutDays(facts: readonly SessionFacts[]): DayRow[] {
  const rows = new Map<number, Piece[]>();
  for (const f of facts) {
    let from = f.start;
    // one piece per calendar day the session touches
    for (let guard = 0; guard < 400 && from < f.end; guard++) {
      const day = dayStart(from);
      const to = Math.min(f.end, nextDay(day));
      const list = rows.get(day) ?? [];
      list.push({ facts: f, from, to, continues: to < f.end, continued: from > f.start });
      rows.set(day, list);
      from = to;
    }
    if (f.end <= f.start) {
      const day = dayStart(f.start);
      const list = rows.get(day) ?? [];
      list.push({ facts: f, from: f.start, to: f.start, continues: false, continued: false });
      rows.set(day, list);
    }
  }
  return [...rows.entries()]
    .sort((a, b) => b[0] - a[0])
    .map(([day, pieces]) => ({ day, pieces: pieces.sort((a, b) => a.from - b.from) }));
}

/**
 * The hours the map spans: every piece fits, rounded out to even hours, at
 * least ten hours wide, clamped to the day. Empty → 8 am to 8 pm. The tail of
 * a session that ran past midnight does not stretch the map back to 12 am; it
 * is drawn as a stub at the left edge instead.
 */
export function hourRange(rows: readonly DayRow[]): { h0: number; h1: number } {
  let lo = Infinity;
  let hi = -Infinity;
  for (const r of rows) {
    for (const p of r.pieces) {
      if (p.continued) continue;
      lo = Math.min(lo, hourOfDay(p.from, r.day));
      hi = Math.max(hi, hourOfDay(p.to, r.day));
    }
  }
  if (!Number.isFinite(lo)) return { h0: 8, h1: 20 };
  let h0 = Math.max(0, Math.floor((lo - 0.25) / 2) * 2);
  let h1 = Math.min(24, Math.ceil((hi + 0.25) / 2) * 2);
  while (h1 - h0 < 10) {
    if (h0 > 0) h0 -= 1;
    if (h1 - h0 < 10 && h1 < 24) h1 += 1;
  }
  return { h0, h1 };
}

/**
 * Instants for a block's camera thumbnails: about one per hour of the piece,
 * as many as fit at `minPx` each, centred in equal slices.
 */
export function thumbInstants(from: number, to: number, widthPx: number, minPx = 22): number[] {
  if (to <= from || widthPx < minPx + 4) return [];
  const hours = (to - from) / 3_600_000;
  const n = Math.max(1, Math.min(Math.round(hours), Math.floor(widthPx / minPx)));
  return Array.from({ length: n }, (_, i) => from + ((i + 0.5) / n) * (to - from));
}

/* ---- the inspector -------------------------------------------------------- */

/** A screen to show as the session's hero still: screen-0, else the largest. */
export function heroScreen(m: SessionManifest): SessionTrack | null {
  const screens = screenTracks(m);
  return (
    screens.find((t) => t.id === "screen-0") ??
    [...screens].sort((a, b) => (b.monitor ? b.monitor.w * b.monitor.h : 0) - (a.monitor ? a.monitor.w * a.monitor.h : 0))[0] ??
    null
  );
}

/** Footage on a track with every hole (its gaps, cuts) taken out, ms. */
export function footageMs(m: SessionManifest, t: SessionTrack): number {
  const holes = trackHoles(m, t.id);
  let ms = 0;
  for (const s of t.segments) {
    let len = s.duration_ms;
    for (const h of holes) {
      const a = Math.max(h.from, s.start_epoch_ms);
      const b = Math.min(h.to, s.start_epoch_ms + s.duration_ms);
      if (b > a) len -= b - a;
    }
    ms += Math.max(0, len);
  }
  return ms;
}

/** Library header: `12 sessions · 41 hours · 174 GB on disk` */
export function libraryTotals(facts: readonly SessionFacts[]): { sessions: number; hours: number; bytes: number } {
  return {
    sessions: facts.length,
    hours: Math.round(facts.reduce((a, f) => a + f.lengthMs, 0) / 3_600_000),
    bytes: facts.reduce((a, f) => a + f.bytes, 0),
  };
}

/**
 * An instant on `track` with footage behind it, as close to `prefer` as the
 * footage allows: `prefer` itself, else the middle of the nearest segment that
 * no cut hides. Null when the track has nothing.
 */
export function nearestFootage(m: SessionManifest, track: SessionTrack, prefer: number): number | null {
  const holes = trackHoles(m, track.id);
  const hidden = (t: number) => holes.some((h) => t >= h.from && t < h.to);
  let best: number | null = null;
  for (const s of track.segments) {
    const inside = prefer >= s.start_epoch_ms && prefer < s.start_epoch_ms + s.duration_ms;
    const t = inside ? prefer : s.start_epoch_ms + s.duration_ms / 2;
    if (hidden(t)) continue;
    if (best === null || Math.abs(t - prefer) < Math.abs(best - prefer)) best = t;
    if (inside) break;
  }
  return best;
}
