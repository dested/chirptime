/**
 * Focus follows the mouse: which screen the cursor was on, with hysteresis so
 * a mouse crossing a screen on its way somewhere else does not flick the stage.
 * Pure: no DOM, no React, so `scripts/check-follow.ts` can check every rule.
 *
 * Samples come from `get_cursor_track`: ascending, in virtual-desktop pixels
 * (the space of `track.monitor`), a record on every move and at least once a
 * second while the mouse holds still, none while paused or inside a cut.
 */
import type { CursorSample, SessionTrack } from "../types";

/** The cursor must stay on a new screen this long before Focus switches to it. */
export const FOLLOW_HOLD_MS = 1_500;
/** Cursor data is fetched in windows this long. */
export const CURSOR_WINDOW_MS = 10 * 60_000;
/** A sample older than this says nothing about now (a pause, a cut, a gap). */
export const CURSOR_STALE_MS = 5_000;

/** Index of the last sample at or before `t`, or -1. Binary search. */
export function sampleIndexAt(samples: readonly CursorSample[], t: number): number {
  let lo = 0;
  let hi = samples.length - 1;
  let best = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const s = samples[mid];
    if (!s) break;
    if (s.t <= t) {
      best = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return best;
}

/** The screen track whose monitor holds this point, or null (between screens, off the desk). */
export function screenAtPoint(screens: readonly SessionTrack[], x: number, y: number): string | null {
  for (const t of screens) {
    const m = t.monitor;
    if (m && x >= m.x && x < m.x + m.w && y >= m.y && y < m.y + m.h) return t.id;
  }
  return null;
}

/** The screen under the cursor at `t`, or null when the last sample is stale or off every screen. */
export function screenUnderCursor(samples: readonly CursorSample[], screens: readonly SessionTrack[], t: number): string | null {
  const s = samples[sampleIndexAt(samples, t)];
  if (!s || t - s.t > CURSOR_STALE_MS) return null;
  return screenAtPoint(screens, s.x, s.y);
}

/**
 * The screen Focus should show at `t`, given the one it shows now. It moves to
 * another screen only once the cursor has been on that screen for the whole of
 * `[t - hold, t]`; with no evidence either way it stays where it is.
 */
export function followScreen(
  samples: readonly CursorSample[],
  screens: readonly SessionTrack[],
  t: number,
  current: string | null,
  holdMs = FOLLOW_HOLD_MS,
): string | null {
  const under = screenUnderCursor(samples, screens, t);
  if (under === null || under === current) return current;
  if (current === null) return under;
  for (let k = sampleIndexAt(samples, t); k >= 0; k--) {
    const s = samples[k];
    if (!s) break;
    if (screenAtPoint(screens, s.x, s.y) !== under) return current;
    // this sample held from s.t on, so it covers the start of the hold window
    if (s.t <= t - holdMs) return under;
  }
  return current;
}

/** The window index holding `t`. */
export function windowOf(t: number): number {
  return Math.floor(t / CURSOR_WINDOW_MS);
}

/**
 * Windows worth having for a playhead at `t`: its own, the next one (playback
 * runs into it), and the previous one near a boundary, so the hold window
 * never starts before the data does.
 */
export function windowsFor(t: number): number[] {
  const w = windowOf(t);
  const out = [w, w + 1];
  if (t - w * CURSOR_WINDOW_MS < FOLLOW_HOLD_MS + CURSOR_STALE_MS) out.push(w - 1);
  return out;
}

/** Samples from every fetched window, ascending, without the boundary duplicates. */
export function mergeWindows(windows: ReadonlyMap<number, readonly CursorSample[]>): CursorSample[] {
  const out: CursorSample[] = [];
  for (const k of [...windows.keys()].sort((a, b) => a - b)) {
    for (const s of windows.get(k) ?? []) {
      const last = out[out.length - 1];
      if (last && s.t <= last.t) continue;
      out.push(s);
    }
  }
  return out;
}
