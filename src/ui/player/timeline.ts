/**
 * Pure geometry and facts for the player: the collapsed time axis, idle and
 * outage spans, snapping, and size estimates. No DOM, no React, so a bun script
 * can check every rule here against a manifest.
 */
import type { SessionManifest, SessionSegment, SessionTrack, TrackGap } from "../types";
import { collapse, expand, sessionHoles, trackHoles } from "../session/cuts";
import type { Hole } from "../session/cuts";
import { sessionBounds } from "../session/engine";

/* ---- the axis ----------------------------------------------------------- */

/**
 * Maps wall-clock epoch-ms onto [0, 1] across a window, with whole-session cuts
 * collapsed to nothing. A cut becomes a seam at a single position.
 */
export interface Axis {
  from: number;
  to: number;
  /** collapsed length of the window, ms */
  span: number;
  holes: readonly Hole[];
}

export function makeAxis(holes: readonly Hole[], from: number, to: number): Axis {
  const span = Math.max(1, collapse(holes, to) - collapse(holes, from));
  return { from, to, span, holes };
}

/** epoch → fraction of the window (may fall outside [0, 1]) */
export function frac(a: Axis, epochMs: number): number {
  return (collapse(a.holes, epochMs) - collapse(a.holes, a.from)) / a.span;
}

/** fraction → epoch; a seam maps to the side that plays */
export function epochAt(a: Axis, f: number): number {
  return expand(a.holes, collapse(a.holes, a.from) + f * a.span);
}

/* ---- session facts ------------------------------------------------------ */

export const OUTAGE_MIN_MS = 60_000;
/** stops within this of each other read as one event: "Screens 1–3 stopped" */
export const GROUP_WINDOW_MS = 120_000;

export function isIdleReason(reason: string): boolean {
  return reason.trim().toLowerCase().startsWith("idle");
}

/** The recorder's Pause writes this reason on every track it stopped. */
export const PAUSED_REASON = "paused";

export function isPausedReason(reason: string): boolean {
  return reason.trim().toLowerCase().startsWith(PAUSED_REASON);
}

/** A hole the person chose (away, paused), not a failure. */
export function isExpectedGap(reason: string): boolean {
  return isIdleReason(reason) || isPausedReason(reason);
}

/** `screen-2` → 3. Screens are numbered from 1 for people. */
export function screenNumber(t: SessionTrack): number {
  const m = /(\d+)$/.exec(t.id);
  return m ? Number(m[1]) + 1 : 0;
}

export function screenTracks(m: SessionManifest): SessionTrack[] {
  return m.tracks.filter((t) => t.kind === "screen").sort((a, b) => screenNumber(a) - screenNumber(b));
}

/** `1–3`, `1, 3`, `2` */
export function numberList(ns: number[]): string {
  const s = [...new Set(ns)].sort((a, b) => a - b);
  const runs: string[] = [];
  for (let i = 0; i < s.length; ) {
    let j = i;
    const at = (k: number) => s[k] ?? NaN;
    while (j + 1 < s.length && at(j + 1) === at(j) + 1) j++;
    runs.push(j - i >= 2 ? `${at(i)}–${at(j)}` : j > i ? `${at(i)}, ${at(j)}` : `${at(i)}`);
    i = j + 1;
  }
  return runs.join(", ");
}

/** Gaps that touch or overlap (within `slackMs`) merge into one, first reason kept. */
function mergeGaps(gaps: TrackGap[], slackMs = 2_000): TrackGap[] {
  const out: TrackGap[] = [];
  for (const g of [...gaps].sort((a, b) => a.from_epoch_ms - b.from_epoch_ms)) {
    const last = out[out.length - 1];
    if (last && g.from_epoch_ms <= last.to_epoch_ms + slackMs) {
      last.to_epoch_ms = Math.max(last.to_epoch_ms, g.to_epoch_ms);
    } else {
      out.push({ ...g });
    }
  }
  return out;
}

export interface Span {
  from: number;
  to: number;
}

/**
 * Away stretches: idle gaps on the picture tracks. The camera's are the truth
 * when there is a camera (it records the whole day); otherwise every picture
 * track's idle gaps, merged.
 */
export function idleSpans(m: SessionManifest): Span[] {
  const cam = m.tracks.find((t) => t.kind === "webcam");
  const sources = cam ? [cam] : m.tracks.filter((t) => t.kind === "screen");
  const gaps = sources.flatMap((t) => t.gaps.filter((g) => isIdleReason(g.reason)));
  return mergeGaps(gaps).map((g) => ({ from: g.from_epoch_ms, to: g.to_epoch_ms }));
}

/** Paused stretches: the recorder stops every track, so any track's paused gaps say it. */
export function pausedSpans(m: SessionManifest): Span[] {
  const gaps = m.tracks.flatMap((t) => t.gaps.filter((g) => isPausedReason(g.reason)));
  return mergeGaps(gaps).map((g) => ({ from: g.from_epoch_ms, to: g.to_epoch_ms }));
}

/** Time nobody meant to record, veiled and labelled on the ribbons. */
export interface QuietSpan extends Span {
  label: "Away" | "Paused";
}

export function quietSpans(m: SessionManifest): QuietSpan[] {
  return [
    ...idleSpans(m).map((s): QuietSpan => ({ ...s, label: "Away" })),
    ...pausedSpans(m).map((s): QuietSpan => ({ ...s, label: "Paused" })),
  ].sort((a, b) => a.from - b.from);
}

export interface Outage {
  trackId: string;
  screen: number;
  from: number;
  to: number;
  reason: string;
}

/** Non-idle screen gaps over a minute: a screen stopped recording. */
export function screenOutages(m: SessionManifest): Outage[] {
  const out: Outage[] = [];
  for (const t of screenTracks(m)) {
    const failures = mergeGaps(t.gaps.filter((g) => !isExpectedGap(g.reason)));
    for (const g of failures) {
      if (g.to_epoch_ms - g.from_epoch_ms <= OUTAGE_MIN_MS) continue;
      out.push({ trackId: t.id, screen: screenNumber(t), from: g.from_epoch_ms, to: g.to_epoch_ms, reason: g.reason });
    }
  }
  return out.sort((a, b) => a.from - b.from || a.screen - b.screen);
}

/** Outages that start within GROUP_WINDOW_MS of the group's first one. */
export function groupOutages(list: Outage[], key: "from" | "to"): [Outage, ...Outage[]][] {
  const sorted = [...list].sort((a, b) => a[key] - b[key] || a.screen - b.screen);
  const groups: [Outage, ...Outage[]][] = [];
  for (const o of sorted) {
    const g = groups[groups.length - 1];
    if (g && o[key] - g[0][key] <= GROUP_WINDOW_MS) g.push(o);
    else groups.push([o]);
  }
  return groups;
}

/** Recorder sentences that pass through to people as written. */
const READABLE = [
  "Display was locked or asleep",
  "Display slept, locked or changed mode",
  "Monitor disconnected or switched off",
  "Graphics driver restarted",
  "Graphics card ran out of memory",
  "This FFmpeg has no encoder for this screen",
  "This FFmpeg cannot capture screens",
  "This FFmpeg is too old for screen recording",
  "Monitor moved to a different graphics card",
];

/** A gap's reason, as a person would say it. */
export function humanReason(reason: string): string {
  const r = reason.trim();
  const lower = r.toLowerCase();
  if (lower.includes("887a0026") || lower.includes("access lost") || lower.includes("stopped reporting")) {
    return "The display went dark";
  }
  if (READABLE.includes(r)) return r;
  // The new recorder writes sentences; ffmpeg writes stderr. A sentence starts
  // with a capital and carries no exit codes, pipes or hex.
  if (/^[A-Z][^|]*$/.test(r) && !/0x[0-9a-f]+|ffmpeg|exit code/i.test(r) && r.length <= 90) return r;
  return "Recording failed";
}

/**
 * The header's amber pill, or null when nothing broke. One screen: `Screen 4
 * stopped at 1:45 PM`; several: `Screens stopped at <when the last of them had
 * stopped>`.
 */
export function outageHeadline(m: SessionManifest): { screens: number[]; at: number } | null {
  const list = screenOutages(m).filter((o) => !inHoles(sessionHoles(m), o.from));
  if (!list.length) return null;
  const firstByScreen = new Map<number, number>();
  for (const o of list) if (!firstByScreen.has(o.screen)) firstByScreen.set(o.screen, o.from);
  return { screens: [...firstByScreen.keys()].sort((a, b) => a - b), at: Math.max(...firstByScreen.values()) };
}

function inHoles(holes: readonly Hole[], t: number): boolean {
  return holes.some((h) => t >= h.from && t < h.to);
}

/* ---- sizes -------------------------------------------------------------- */

/** Bytes of `s` inside [from, to), prorated by time. */
export function bytesInside(s: SessionSegment, from: number, to: number): number {
  const a = Math.max(from, s.start_epoch_ms);
  const b = Math.min(to, s.start_epoch_ms + s.duration_ms);
  if (b <= a || s.duration_ms <= 0) return 0;
  return (s.bytes * (b - a)) / s.duration_ms;
}

/** What deleting [from, to) on these tracks would free. */
export function rangeBytes(m: SessionManifest, from: number, to: number, trackIds?: readonly string[]): number {
  let n = 0;
  for (const t of m.tracks) {
    if (trackIds && !trackIds.includes(t.id)) continue;
    for (const s of t.segments) n += bytesInside(s, from, to);
  }
  return Math.round(n);
}

/** The session's length and size with every cut taken out. */
export function sessionTotals(m: SessionManifest): { start: number; end: number; lengthMs: number; bytes: number } {
  const { start, end } = sessionBounds(m);
  const holes = sessionHoles(m);
  const lengthMs = collapse(holes, end) - collapse(holes, start);
  let bytes = 0;
  for (const t of m.tracks) {
    const th = trackHoles(m, t.id);
    for (const s of t.segments) {
      bytes += s.bytes;
      for (const h of th) bytes -= bytesInside(s, h.from, h.to);
    }
  }
  return { start, end, lengthMs, bytes: Math.max(0, Math.round(bytes)) };
}

/* ---- snapping ----------------------------------------------------------- */

export type SnapKind = "gap" | "mark" | "cut" | "start" | "end" | "segment";

export interface SnapTarget {
  t: number;
  kind: SnapKind;
}

const SNAP_LABEL: Record<SnapKind, string> = {
  gap: "Snapped to a gap",
  mark: "Snapped to a mark",
  cut: "Snapped to a cut",
  start: "Snapped to the session start",
  end: "Snapped to the session end",
  segment: "Snapped to a segment edge",
};

export function snapLabel(k: SnapKind): string {
  return SNAP_LABEL[k];
}

/** Everything an edge may snap to. Segment edges come from the picture with the most footage. */
export function snapTargets(m: SessionManifest): SnapTarget[] {
  const { start, end } = sessionBounds(m);
  const out: SnapTarget[] = [
    { t: start, kind: "start" },
    { t: end, kind: "end" },
  ];
  for (const s of quietSpans(m)) out.push({ t: s.from, kind: "gap" }, { t: s.to, kind: "gap" });
  for (const o of screenOutages(m)) out.push({ t: o.from, kind: "gap" }, { t: o.to, kind: "gap" });
  for (const mk of m.markers) out.push({ t: mk.epoch_ms, kind: "mark" });
  for (const h of sessionHoles(m)) out.push({ t: h.from, kind: "cut" }, { t: h.to, kind: "cut" });
  const pictures = m.tracks.filter((t) => t.kind === "webcam" || t.kind === "screen");
  const richest = pictures.sort((a, b) => b.segments.length - a.segments.length)[0];
  if (richest) {
    for (const s of richest.segments) out.push({ t: s.start_epoch_ms, kind: "segment" });
  }
  return out;
}

/**
 * The nearest target within `radiusPx`, landmarks first: a segment edge only
 * wins when no gap, mark, cut or session edge is in reach.
 */
export function snap(
  targets: readonly SnapTarget[],
  t: number,
  pxPerMs: number,
  radiusPx = 8,
): { t: number; kind: SnapKind | null } {
  let best: SnapTarget | null = null;
  let bestSeg: SnapTarget | null = null;
  for (const c of targets) {
    const d = Math.abs(c.t - t) * pxPerMs;
    if (d > radiusPx) continue;
    if (c.kind === "segment") {
      if (!bestSeg || Math.abs(c.t - t) < Math.abs(bestSeg.t - t)) bestSeg = c;
    } else if (!best || Math.abs(c.t - t) < Math.abs(best.t - t)) {
      best = c;
    }
  }
  const pick = best ?? bestSeg;
  return pick ? { t: pick.t, kind: pick.kind } : { t, kind: null };
}

/* ---- footage coverage ------------------------------------------------------ */

/** Is there footage on this track at t (cuts included)? */
export function hasFootage(m: SessionManifest, track: SessionTrack, t: number): boolean {
  if (trackHoles(m, track.id).some((h) => t >= h.from && t < h.to)) return false;
  const segs = track.segments;
  let lo = 0;
  let hi = segs.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const s = segs[mid];
    if (!s) return false;
    if (t < s.start_epoch_ms) hi = mid - 1;
    else if (t >= s.start_epoch_ms + s.duration_ms + 250) lo = mid + 1;
    else return true;
  }
  return false;
}

/** The thumbnail instant for t: rounded down to 10 s so the cache hits. */
export function thumbInstant(t: number, stepMs = 10_000): number {
  return Math.floor(t / stepMs) * stepMs;
}

/**
 * A thumbnail instant with footage behind it: t rounded down to `stepMs`, or the
 * next 10 s boundary when rounding down lands before the segment (a session
 * start, a screen coming back). Null when the track has nothing there.
 */
export function thumbFor(m: SessionManifest, track: SessionTrack, t: number, stepMs = 10_000): number | null {
  const near = thumbInstant(t);
  for (const c of [thumbInstant(t, stepMs), near, near + 10_000]) {
    if (hasFootage(m, track, c)) return c;
  }
  return null;
}

/* ---- what a stopped track shows ------------------------------------------- */

/**
 * The last instant this track had footage before `t`, skipping its holes
 * (gaps and cuts), or null when nothing came before. The stage shows that
 * frame, dimmed, while the playhead sits in a gap.
 */
export function lastFootageBefore(m: SessionManifest, track: SessionTrack, t: number): number | null {
  const holes = trackHoles(m, track.id);
  const segs = track.segments;
  let probe = t;
  // each pass steps back over one hole or one segment, so this ends
  for (let guard = 0; guard < segs.length + holes.length + 2; guard++) {
    const hole = holes.find((h) => probe > h.from && probe <= h.to);
    if (hole) {
      probe = hole.from;
      continue;
    }
    let seg: SessionSegment | null = null;
    for (const s of segs) {
      if (s.start_epoch_ms < probe) seg = s;
      else break;
    }
    if (!seg) return null;
    const end = Math.min(probe, seg.start_epoch_ms + seg.duration_ms);
    // the last frame sits a moment before the end
    const at = Math.max(seg.start_epoch_ms, end - 1_000);
    if (!holes.some((h) => at >= h.from && at < h.to)) return at;
    probe = at;
  }
  return null;
}

/** Why this track has no footage at `t`, in a few words. */
export function gapReasonAt(m: SessionManifest, track: SessionTrack, t: number): string | null {
  if (trackHoles(m, track.id).some((h) => t >= h.from && t < h.to)) return "Cut";
  const g = track.gaps.find((x) => t >= x.from_epoch_ms && t < x.to_epoch_ms);
  if (!g) return null;
  if (isIdleReason(g.reason)) return "Away";
  if (isPausedReason(g.reason)) return "Paused";
  return humanReason(g.reason);
}
