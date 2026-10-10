/**
 * Checks the round-2 rules that live in pure modules: Focus follow with its
 * hysteresis (`player/follow.ts`), the Library's time map (`app/libraryModel.ts`)
 * and the paused / gap facts the log and stage use (`player/log.ts`,
 * `player/timeline.ts`). No app, no browser, no fixtures needed.
 *
 *   bun scripts/check-library.ts
 */
import type { CursorSample, SessionManifest, SessionTrack, TrackGap } from "../src/ui/types";
import { followScreen, mergeWindows, screenUnderCursor, windowsFor, CURSOR_WINDOW_MS } from "../src/ui/player/follow";
import { hourRange, layoutDays, nearestFootage, sessionFacts, thumbInstants } from "../src/ui/app/libraryModel";
import { deriveLog } from "../src/ui/player/log";
import { gapReasonAt, lastFootageBefore, outageHeadline, screenOutages } from "../src/ui/player/timeline";

let failed = 0;
const check = (ok: boolean, what: string) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failed++;
};

const MIN = 60_000;
const T0 = new Date(2026, 7, 1, 10, 0, 0).getTime();

function track(id: string, kind: SessionTrack["kind"], x: number, gaps: TrackGap[] = [], segs = 60, start = T0): SessionTrack {
  return {
    id,
    kind,
    label: id,
    monitor: kind === "screen" ? { output_idx: 0, adapter: 0, x, y: 0, w: 1920, h: 1080 } : null,
    codec: "h264",
    width: 1920,
    height: 1080,
    fps: 30,
    start_epoch_ms: start,
    dir: id,
    segments: Array.from({ length: segs }, (_, i) => ({ file: `${i}.mp4`, start_epoch_ms: start + i * MIN, duration_ms: MIN, bytes: 1000 }))
      .filter((s) => !gaps.some((g) => s.start_epoch_ms >= g.from_epoch_ms && s.start_epoch_ms < g.to_epoch_ms)),
    gaps,
    status: "stopped",
    restarts: 0,
  };
}

function manifest(tracks: SessionTrack[], start = T0, minutes = 60): SessionManifest {
  return {
    version: 1,
    id: "s",
    started_epoch_ms: start,
    ended_epoch_ms: start + minutes * MIN,
    segment_seconds: 60,
    desktop: { x: 0, y: 0, w: 3840, h: 1080 },
    tracks,
    markers: [],
    rebuilt: false,
    cuts: [],
  };
}

/* ---- follow ---------------------------------------------------------------- */

const screens = [track("screen-0", "screen", 0), track("screen-1", "screen", 1920)];
// on screen-0 until t=10 s, crosses screen-1 for 1 s, back to 0, then over to 1 for good at 20 s
const samples: CursorSample[] = [];
for (let t = 0; t <= 40_000; t += 100) {
  const onOne = (t >= 10_000 && t < 11_000) || t >= 20_000;
  samples.push({ t: T0 + t, x: onOne ? 2500 : 500, y: 500 });
}
const at = (s: number) => T0 + s * 1000;
check(screenUnderCursor(samples, screens, at(5)) === "screen-0", "cursor at 5 s is on screen 1");
check(followScreen(samples, screens, at(10.5), "screen-0") === "screen-0", "a 1 s crossing does not switch (hysteresis)");
check(followScreen(samples, screens, at(20.5), "screen-0") === "screen-0", "0.5 s on the new screen is not enough");
check(followScreen(samples, screens, at(21.6), "screen-0") === "screen-1", "1.5 s on the new screen switches");
check(followScreen([], screens, at(5), "screen-0") === "screen-0", "no data keeps the static screen");
check(followScreen(samples, screens, at(60), "screen-1") === "screen-1", "a stale sample keeps the current screen");
check(followScreen(samples, screens, at(5), null) === "screen-0", "no current screen takes the one under the cursor");
const w = Math.floor(at(5) / CURSOR_WINDOW_MS);
check(windowsFor(w * CURSOR_WINDOW_MS + 1000).includes(w - 1), "near a window start the previous window is fetched too");
check(
  mergeWindows(new Map([[2, [{ t: 5, x: 0, y: 0 }]], [1, [{ t: 1, x: 0, y: 0 }, { t: 5, x: 0, y: 0 }]]])).map((s) => s.t).join() === "1,5",
  "windows merge ascending without boundary duplicates",
);

/* ---- paused gaps ----------------------------------------------------------- */

const paused: TrackGap = { from_epoch_ms: T0 + 20 * MIN, to_epoch_ms: T0 + 32 * MIN, reason: "paused" };
const pm = manifest([track("screen-0", "screen", 0, [paused]), track("webcam", "webcam", 0, [paused])]);
const lines = deriveLog(pm, { live: false, now: Date.now() });
check(lines.some((l) => l.title === "Paused 12m"), "a paused gap reads Paused 12m");
check(screenOutages(pm).length === 0 && outageHeadline(pm) === null, "a pause is not an outage");
const cam = pm.tracks[1];
if (cam) {
  check(gapReasonAt(pm, cam, T0 + 25 * MIN) === "Paused", "the stage names a pause");
  const last = lastFootageBefore(pm, cam, T0 + 25 * MIN);
  check(last !== null && last < T0 + 20 * MIN && last >= T0 + 19 * MIN, "the last frame before a pause is just before it");
  check(nearestFootage(pm, cam, T0 + 25 * MIN) !== null, "nearest footage skips the pause");
}

/* ---- the time map ---------------------------------------------------------- */

const lateStart = new Date(2026, 7, 3, 23, 20).getTime();
const late = manifest([track("webcam", "webcam", 0, [], 75, lateStart)], lateStart, 75);
const morningStart = new Date(2026, 7, 5, 9, 30).getTime();
const morning = manifest([track("webcam", "webcam", 0, [], 60, morningStart)], morningStart, 60);
const summary = (m: SessionManifest) => ({
  id: m.id,
  started_epoch_ms: m.started_epoch_ms,
  ended_epoch_ms: m.ended_epoch_ms,
  duration_ms: (m.ended_epoch_ms ?? m.started_epoch_ms) - m.started_epoch_ms,
  track_count: m.tracks.length,
  segment_count: 0,
  bytes: 0,
  has_manifest: true,
});
const facts = [late, morning].map((m) => sessionFacts(summary(m), m, null, Date.now()));
const rows = layoutDays(facts);
check(rows.length === 3, `a session past midnight lands on two rows (${rows.length} rows)`);
check(rows[0]?.day === new Date(2026, 7, 5).getTime(), "rows run newest day first");
const tail = rows.find((r) => r.day === new Date(2026, 7, 4).getTime())?.pieces[0];
check(tail?.continued === true && tail.to - tail.from === 35 * MIN, "the tail after midnight is 35 minutes, marked continued");
const range = hourRange(rows);
check(range.h0 === 8 && range.h1 === 24, `hour range ignores the after-midnight tail (got ${range.h0}–${range.h1})`);
check(thumbInstants(T0, T0 + 3 * 3_600_000, 300).length === 3, "about one thumbnail per hour");
check(thumbInstants(T0, T0 + 8 * 3_600_000, 50).length === 2, "no more thumbnails than fit");
const short = sessionFacts({ ...summary(morning), duration_ms: 30_000, ended_epoch_ms: morning.started_epoch_ms + 30_000 }, null, null, Date.now());
check(short.short, "a 30 s session is offered for clean-up");
check(!sessionFacts({ ...summary(morning), duration_ms: 30_000, ended_epoch_ms: null }, null, "s", Date.now()).short, "the live session never is");

if (failed) {
  console.error(`${failed} check(s) failed`);
  process.exit(1);
}
