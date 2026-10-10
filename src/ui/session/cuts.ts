/**
 * Deleted time ("cuts") as holes in a session. Pure — no DOM, no media — so the
 * engine, the timeline and a plain `bun` script can all use it.
 *
 * A cut that covers every track removes that time from the session: playback
 * jumps over it and the timeline collapses it. A cut over only some tracks
 * (say the mic during a private call) leaves the session's time alone; those
 * tracks render a gap inside it while the rest keep playing.
 *
 * Holes are half-open `[from, to)` in epoch-ms. Up to one GOP of footage inside
 * a cut can still be on disk (video edges snap to keyframes), which is why the
 * hole, not the segment list, decides what plays.
 */
import type { Cut, SessionManifest } from "../types";

export interface Hole {
  from: number;
  to: number;
}

function liveCuts(m: SessionManifest): Cut[] {
  // `?? []`: a manifest read straight off disk may predate cuts.
  return (m.cuts ?? []).filter((c) => c.state !== "undone");
}

/** Does this cut remove the time from the whole session? */
export function coversAllTracks(m: SessionManifest, cut: Cut): boolean {
  const only = cut.tracks;
  if (!only) return true;
  return m.tracks.every((t) => only.includes(t.id));
}

/** Sort and merge overlapping or touching ranges. */
export function mergeHoles(holes: Hole[]): Hole[] {
  const sorted = holes.filter((h) => h.to > h.from).sort((a, b) => a.from - b.from);
  const out: Hole[] = [];
  for (const h of sorted) {
    const last = out[out.length - 1];
    if (last && h.from <= last.to) last.to = Math.max(last.to, h.to);
    else out.push({ from: h.from, to: h.to });
  }
  return out;
}

/** Time removed from the whole session: the clock skips these. */
export function sessionHoles(m: SessionManifest): Hole[] {
  return mergeHoles(
    liveCuts(m)
      .filter((c) => coversAllTracks(m, c))
      .map((c) => ({ from: c.from_epoch_ms, to: c.to_epoch_ms })),
  );
}

/** Every hole one track has, whole-session cuts included: it renders a gap here. */
export function trackHoles(m: SessionManifest, trackId: string): Hole[] {
  return mergeHoles(
    liveCuts(m)
      .filter((c) => !c.tracks || c.tracks.includes(trackId))
      .map((c) => ({ from: c.from_epoch_ms, to: c.to_epoch_ms })),
  );
}

/** The (merged) hole containing epochMs, or null. Binary search. */
export function holeAt(holes: readonly Hole[], epochMs: number): Hole | null {
  let lo = 0;
  let hi = holes.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const h = holes[mid];
    if (epochMs < h.from) hi = mid - 1;
    else if (epochMs >= h.to) lo = mid + 1;
    else return h;
  }
  return null;
}

/** epochMs, or the end of the hole it sits in. Holes are merged, so one jump lands clear. */
export function skipHoles(holes: readonly Hole[], epochMs: number): number {
  const h = holeAt(holes, epochMs);
  return h ? h.to : epochMs;
}

/** First hole starting at or after epochMs, or null. */
export function nextHole(holes: readonly Hole[], epochMs: number): Hole | null {
  for (const h of holes) if (h.from >= epochMs) return h;
  return null;
}

/** `collapsedTime` over precomputed (merged) holes. */
export function collapse(holes: readonly Hole[], epochMs: number): number {
  let removed = 0;
  for (const h of holes) {
    if (h.from >= epochMs) break;
    removed += Math.min(h.to, epochMs) - h.from;
  }
  return epochMs - removed;
}

/** `expandedTime` over precomputed (merged) holes. */
export function expand(holes: readonly Hole[], collapsedMs: number): number {
  let removed = 0;
  for (const h of holes) {
    // Exactly at a hole's start maps to its end: the side that plays.
    if (collapsedMs < h.from - removed) break;
    removed += h.to - h.from;
  }
  return collapsedMs + removed;
}

/**
 * Wall-clock epoch → "session time with cuts removed", in the same units: the
 * epoch minus every whole-session hole before it. Inside a hole it pins to the
 * hole's start. A timeline position is `collapsedTime(m, t) - collapsedTime(m, start)`.
 */
export function collapsedTime(m: SessionManifest, epochMs: number): number {
  return collapse(sessionHoles(m), epochMs);
}

/** Inverse of `collapsedTime`. A hole's boundary maps to the hole's end. */
export function expandedTime(m: SessionManifest, collapsedMs: number): number {
  return expand(sessionHoles(m), collapsedMs);
}
