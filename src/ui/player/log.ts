/**
 * The day log beside the stage, derived from the manifest alone. Pure: the
 * same manifest and `now` always give the same lines, so a bun script can
 * check it (`scripts/check-log.ts`).
 *
 * Presence lines ("Left the desk", "with company") need away detection, which
 * does not exist yet. They are not invented here.
 */
import type { SessionManifest } from "../types";
import { sessionHoles } from "../session/cuts";
import { sessionBounds } from "../session/engine";
import { fmtSpan } from "../session/format";
import { groupOutages, humanReason, idleSpans, numberList, screenOutages } from "./timeline";

export type LogKind = "start" | "mark" | "away" | "stopped" | "back" | "cut" | "end" | "interrupted";

export interface LogLine {
  /** stable across re-derivations, for React keys */
  key: string;
  kind: LogKind;
  /** where a click seeks */
  at: number;
  title: string;
  sub: string | null;
  /** the sub-line is a prompt, not content ("Add a note") */
  subMuted?: boolean;
  /** a range the line can select for deletion (away stretches) */
  range?: { from: number; to: number };
  /** for a pending cut: its id and when undo runs out */
  cutId?: string;
  undoUntil?: number;
}

export interface LogOptions {
  /** the session is still recording */
  live: boolean;
  /** wall clock now, for "Undo for N s" */
  now: number;
}

const UNDO_WINDOW_MS = 10_000;

/** `4 screens, camera, mic and system audio` */
export function trackSummary(m: SessionManifest): string {
  const screens = m.tracks.filter((t) => t.kind === "screen").length;
  const parts: string[] = [];
  if (screens) parts.push(screens === 1 ? "1 screen" : `${screens} screens`);
  if (m.tracks.some((t) => t.kind === "webcam")) parts.push("camera");
  if (m.tracks.some((t) => t.kind === "mic")) parts.push("mic");
  if (m.tracks.some((t) => t.kind === "system-audio")) parts.push("system audio");
  const last = parts.pop();
  if (last === undefined) return "No tracks";
  return parts.length ? `${parts.join(", ")} and ${last}` : last;
}

function screensLabel(ns: number[]): string {
  return ns.length === 1 ? `Screen ${ns[0]}` : `Screens ${numberList(ns)}`;
}

/** The order lines take when they share an instant. */
const ORDER: Record<LogKind, number> = {
  start: 0,
  back: 1,
  cut: 2,
  away: 3,
  stopped: 4,
  mark: 5,
  end: 6,
  interrupted: 6,
};

export function deriveLog(m: SessionManifest, opts: LogOptions): LogLine[] {
  const holes = sessionHoles(m);
  const deleted = (t: number) => holes.some((h) => t >= h.from && t < h.to);
  const { end } = sessionBounds(m);
  const lines: LogLine[] = [];

  lines.push({
    key: "start",
    kind: "start",
    at: m.started_epoch_ms,
    title: "Started recording",
    sub: trackSummary(m),
  });

  m.markers.forEach((mk, i) => {
    if (deleted(mk.epoch_ms)) return;
    const label = mk.label.trim();
    lines.push({
      key: `mark-${mk.epoch_ms}-${i}`,
      kind: "mark",
      at: mk.epoch_ms,
      title: "Mark",
      sub: label || "Add a note",
      subMuted: !label,
    });
  });

  for (const s of idleSpans(m)) {
    if (deleted(s.from)) continue;
    lines.push({
      key: `away-${s.from}`,
      kind: "away",
      at: s.from,
      title: `Away ${fmtSpan(s.to - s.from)}`,
      sub: "Not recorded, no keyboard or mouse",
      range: { from: s.from, to: s.to },
    });
  }

  const outages = screenOutages(m).filter((o) => !deleted(o.from));
  for (const g of groupOutages(outages, "from")) {
    const reasons = [...new Set(g.map((o) => humanReason(o.reason)))];
    lines.push({
      key: `stopped-${g[0].from}-${g[0].screen}`,
      kind: "stopped",
      at: g[0].from,
      title: `${screensLabel(g.map((o) => o.screen))} stopped recording`,
      sub: reasons.join(". "),
    });
  }
  // A screen comes back where its segments resume; one that never did has no line.
  const resumed = outages.filter((o) => o.to < end - 1_000 && !deleted(o.to));
  for (const g of groupOutages(resumed, "to")) {
    lines.push({
      key: `back-${g[0].to}-${g[0].screen}`,
      kind: "back",
      at: g[0].to,
      title: `${screensLabel(g.map((o) => o.screen))} back`,
      sub: null,
    });
  }

  for (const c of m.cuts) {
    if (c.state === "undone") continue;
    // A cut on only some tracks leaves the time in place; say which.
    const partial = c.tracks && !m.tracks.every((t) => c.tracks?.includes(t.id));
    const undoUntil = c.created_epoch_ms + UNDO_WINDOW_MS;
    const left = Math.ceil((undoUntil - opts.now) / 1000);
    const pending = c.state === "pending" && left > 0;
    const subParts: string[] = [];
    if (partial && c.tracks) subParts.push(`${c.tracks.length === 1 ? "1 track" : `${c.tracks.length} tracks`}`);
    if (pending) subParts.push(`Undo for ${left} s`);
    lines.push({
      key: `cut-${c.id}`,
      kind: "cut",
      at: c.from_epoch_ms,
      title: `Cut ${fmtSpan(c.to_epoch_ms - c.from_epoch_ms)}`,
      sub: subParts.length ? subParts.join(" · ") : null,
      ...(pending ? { cutId: c.id, undoUntil } : {}),
    });
  }

  if (m.ended_epoch_ms !== null) {
    lines.push({ key: "end", kind: "end", at: m.ended_epoch_ms, title: "Stopped recording", sub: null });
  } else if (!opts.live) {
    lines.push({
      key: "end",
      kind: "interrupted",
      at: end,
      title: "Interrupted",
      sub: "The recorder stopped without finishing this session",
    });
  }

  return lines.sort((a, b) => a.at - b.at || ORDER[a.kind] - ORDER[b.kind]);
}

/** The line the playhead is in: the last one at or before it. */
export function currentLineIndex(lines: readonly LogLine[], playhead: number): number {
  let idx = -1;
  for (const [i, l] of lines.entries()) {
    if (l.at <= playhead + 500) idx = i;
    else break;
  }
  return idx;
}
