/**
 * Writes synthetic sessions to `fixtures/sessions/<id>/` for the browser dev
 * harness (`scripts/fixture-server.ts` + `src/ui/dev/mockIpc.ts`). Never touches
 * a real output folder.
 *
 *   bun scripts/make-fixture.ts          # skips a session that already exists
 *   bun scripts/make-fixture.ts --force  # rebuilds everything
 *
 * The main session runs about two hours with the shape of a bad real day: four
 * screens (one ultrawide), a camera, mic and system audio; one idle gap on every
 * picture track; screen 4 dying first, screens 1–3 a few minutes later, and all
 * four coming back. Every frame burns in its track name and the wall clock, so
 * sync and seeks can be read straight off the picture.
 */
import { mkdir, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import type {
  DesktopRect,
  SessionManifest,
  SessionSegment,
  SessionSummary,
  SessionTrack,
  TrackGap,
  TrackKind,
  TrackMonitor,
} from "../src/ui/types";

const ROOT = resolve(import.meta.dir, "..", "fixtures", "sessions");
const FORCE = process.argv.includes("--force");
const FONT = "C\\:/Windows/Fonts/consola.ttf";
const VIDEO_SEGMENT_MS = 60_000;
const AUDIO_SEGMENT_MS = 600_000;
const FPS = 5;

const MIN = 60_000;
const IDLE_REASON = "idle — no keyboard or mouse activity";
const STALL_REASON =
  "ffmpeg stopped reporting progress: no stderr output — the process may have been killed externally";
const DARK_REASON = "Display was locked or asleep";

interface Run {
  from: number;
  to: number;
}

interface TrackPlan {
  id: string;
  kind: TrackKind;
  label: string;
  monitor: TrackMonitor | null;
  width: number;
  height: number;
  runs: Run[];
  gaps: TrackGap[];
  /** the picture's flat ground, so screens tell apart in a thumbnail */
  ground: string;
  /** what the picture says, or the sine pitch for audio */
  title: string;
  pitch: number;
}

interface SessionPlan {
  id: string;
  start: number;
  end: number;
  desktop: DesktopRect;
  tracks: TrackPlan[];
  markers: { epoch_ms: number; label: string }[];
}

const p2 = (n: number) => String(n).padStart(2, "0");

function sessionId(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())}_${p2(d.getHours())}-${p2(d.getMinutes())}-${p2(d.getSeconds())}`;
}

/** The recorder's `-strftime` name: `YYYYMMDD-HHMMSS.mp4`. */
function segmentName(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}${p2(d.getMonth() + 1)}${p2(d.getDate())}-${p2(d.getHours())}${p2(d.getMinutes())}${p2(d.getSeconds())}.mp4`;
}

/** Runs of footage: the session span minus the gaps. */
function runsBetween(start: number, end: number, gaps: TrackGap[]): Run[] {
  const runs: Run[] = [];
  let at = start;
  for (const g of [...gaps].sort((a, b) => a.from_epoch_ms - b.from_epoch_ms)) {
    if (g.from_epoch_ms > at) runs.push({ from: at, to: g.from_epoch_ms });
    at = Math.max(at, g.to_epoch_ms);
  }
  if (end > at) runs.push({ from: at, to: end });
  return runs;
}

/* ---- the plans ---------------------------------------------------------- */

const DESKTOP: DesktopRect = { x: -3840, y: -2160, w: 12800, h: 4320 };
const GROUNDS = ["0x2C2B27", "0x1F2E2A", "0x1E2A35", "0x33291F"];
const MONITORS: Record<string, TrackMonitor> = {
  "screen-0": { output_idx: 0, adapter: 0, x: 0, y: 0, w: 5120, h: 2160 },
  "screen-1": { output_idx: 1, adapter: 0, x: 5120, y: -15, w: 3840, h: 2160 },
  "screen-2": { output_idx: 2, adapter: 0, x: -3840, y: -15, w: 3840, h: 2160 },
  "screen-3": { output_idx: 0, adapter: 1, x: 729, y: -2160, w: 3840, h: 2160 },
};

function mainDay(): SessionPlan {
  const start = new Date(2026, 7, 1, 10, 30, 12).getTime();
  const at = (m: number) => start + m * MIN;
  const end = at(122);
  const idle: TrackGap = { from_epoch_ms: at(40), to_epoch_ms: at(60), reason: IDLE_REASON };
  const screens: TrackPlan[] = [0, 1, 2, 3].map((i) => {
    const id = `screen-${i}`;
    const mon = MONITORS[id];
    const down: TrackGap =
      i === 3
        ? { from_epoch_ms: at(75), to_epoch_ms: at(100), reason: STALL_REASON }
        : { from_epoch_ms: at(82), to_epoch_ms: at(100), reason: DARK_REASON };
    const gaps = [idle, down];
    return {
      id,
      kind: "screen",
      label: `DISPLAY${i + 1} (${mon.w}x${mon.h})`,
      monitor: mon,
      width: 1280,
      // keep each picture the shape of its monitor: the ultrawide is 21:9
      height: Math.round((1280 * mon.h) / mon.w / 2) * 2,
      runs: runsBetween(start, end, gaps),
      gaps,
      title: `Screen ${i + 1}`,
      ground: GROUNDS[i],
      pitch: 0,
    };
  });
  const webcam: TrackPlan = {
    id: "webcam",
    kind: "webcam",
    label: "Fixture camera",
    monitor: null,
    width: 1280,
    height: 720,
    runs: runsBetween(start, end, [idle]),
    gaps: [idle],
    title: "Camera",
    ground: "0x3A3833",
    pitch: 0,
  };
  const audio = (id: string, kind: TrackKind, label: string, pitch: number): TrackPlan => ({
    id,
    kind,
    label,
    monitor: null,
    width: 0,
    height: 0,
    runs: [{ from: start, to: end }],
    gaps: [],
    title: label,
    ground: "",
    pitch,
  });
  return {
    id: sessionId(start),
    start,
    end,
    desktop: DESKTOP,
    tracks: [
      ...screens,
      webcam,
      audio("mic", "mic", "Fixture microphone", 220),
      audio("sysaudio", "system-audio", "System audio (WASAPI loopback)", 330),
    ],
    markers: [
      { epoch_ms: at(22), label: "" },
      { epoch_ms: at(108), label: "Bug repro" },
    ],
  };
}

/** A short second session so the Library has more than one row. */
function shortDay(): SessionPlan {
  const start = new Date(2026, 7, 2, 9, 4, 40).getTime();
  const end = start + 3 * MIN;
  const run = [{ from: start, to: end }];
  return {
    id: sessionId(start),
    start,
    end,
    desktop: { x: 0, y: 0, w: 5120, h: 2160 },
    tracks: [
      {
        id: "screen-0",
        kind: "screen",
        label: "DISPLAY1 (5120x2160)",
        monitor: { output_idx: 0, adapter: 0, x: 0, y: 0, w: 5120, h: 2160 },
        width: 1280,
        height: 540,
        runs: run,
        gaps: [],
        title: "Screen 1",
        ground: GROUNDS[0],
        pitch: 0,
      },
      {
        id: "webcam",
        kind: "webcam",
        label: "Fixture camera",
        monitor: null,
        width: 1280,
        height: 720,
        runs: run,
        gaps: [],
        title: "Camera",
        ground: "0x3A3833",
        pitch: 0,
      },
      {
        id: "mic",
        kind: "mic",
        label: "Fixture microphone",
        monitor: null,
        width: 0,
        height: 0,
        runs: run,
        gaps: [],
        title: "Mic",
        ground: "",
        pitch: 220,
      },
    ],
    markers: [],
  };
}

/**
 * A small session for the Library's time map: one 1440p screen, a camera and a
 * mic, at a light 640 px. `gaps` land on every track (paused, idle).
 */
function smallDay(
  start: number,
  minutes: number,
  gaps: { from: number; to: number; reason: string }[] = [],
  markers: { at: number; label: string }[] = [],
): SessionPlan {
  const end = start + minutes * MIN;
  const g: TrackGap[] = gaps.map((x) => ({ from_epoch_ms: start + x.from * MIN, to_epoch_ms: start + x.to * MIN, reason: x.reason }));
  const runs = runsBetween(start, end, g);
  const picture = (id: string, kind: TrackKind, title: string, ground: string, monitor: TrackMonitor | null): TrackPlan => ({
    id,
    kind,
    label: kind === "screen" ? "DISPLAY1 (2560x1440)" : "Fixture camera",
    monitor,
    width: 640,
    height: 360,
    runs,
    gaps: g,
    title,
    ground,
    pitch: 0,
  });
  return {
    id: sessionId(start),
    start,
    end,
    desktop: { x: 0, y: 0, w: 2560, h: 1440 },
    tracks: [
      picture("screen-0", "screen", "Screen 1", GROUNDS[2], { output_idx: 0, adapter: 0, x: 0, y: 0, w: 2560, h: 1440 }),
      picture("webcam", "webcam", "Camera", "0x3A3833", null),
      {
        id: "mic",
        kind: "mic",
        label: "Fixture microphone",
        monitor: null,
        width: 0,
        height: 0,
        runs,
        gaps: g,
        title: "Mic",
        ground: "",
        pitch: 220,
      },
    ],
    markers: markers.map((m) => ({ epoch_ms: start + m.at * MIN, label: m.label })),
  };
}

/** Sessions for the Library: more days, a pause, a midnight crossing, two starts pressed twice. */
function libraryDays(): SessionPlan[] {
  return [
    smallDay(new Date(2026, 6, 30, 9, 12, 5).getTime(), 150, [{ from: 70, to: 95, reason: IDLE_REASON }], [{ at: 31, label: "Standup notes" }]),
    smallDay(new Date(2026, 6, 31, 13, 5, 40).getTime(), 75, [{ from: 35, to: 47, reason: "paused" }], [{ at: 60, label: "" }]),
    smallDay(new Date(2026, 7, 2, 14, 10, 2).getTime(), 0.5),
    smallDay(new Date(2026, 7, 2, 14, 11, 9).getTime(), 1 / 3),
    smallDay(new Date(2026, 7, 3, 23, 20, 30).getTime(), 75, [], [{ at: 50, label: "After midnight" }]),
  ];
}

/* ---- ffmpeg ------------------------------------------------------------- */

/** Retries twice: lavfi under load occasionally dies with ENOMEM on Windows. */
async function ffmpeg(args: string[], attempt = 1): Promise<void> {
  const p = Bun.spawn(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", ...args], {
    stdout: "ignore",
    stderr: "pipe",
  });
  const err = await new Response(p.stderr).text();
  if ((await p.exited) === 0) return;
  if (attempt < 3) return ffmpeg(args, attempt + 1);
  throw new Error(`ffmpeg failed: ${err.slice(-400)}`);
}

/** One continuous run → `-f segment` pieces in `tmp`, oldest first. */
async function encodeRun(t: TrackPlan, run: Run, tmp: string): Promise<string[]> {
  await mkdir(tmp, { recursive: true });
  const seconds = (run.to - run.from) / 1000;
  const isAudio = t.kind === "mic" || t.kind === "system-audio";
  const pattern = join(tmp, "%04d.mp4");
  if (isAudio) {
    await ffmpeg([
      "-f", "lavfi", "-i", `sine=frequency=${t.pitch}:sample_rate=48000`,
      "-t", String(seconds),
      "-af", "volume=0.15",
      "-c:a", "aac", "-b:a", "24k",
      "-f", "segment", "-segment_time", String(AUDIO_SEGMENT_MS / 1000), "-reset_timestamps", "1",
      pattern,
    ]);
  } else {
    // A small moving testsrc2 on a flat ground keeps the bytes tiny; the label
    // and the wall clock are drawn large enough to read off a thumbnail.
    const epochS = Math.floor(run.from / 1000);
    const pattern2 = t.kind === "webcam" ? `testsrc2=size=320x180:rate=${FPS},hue=s=0.2` : `testsrc2=size=320x180:rate=${FPS}`;
    // localtime's default format: `2026-08-01 10:30:12`
    const text = `${t.title}   %{pts\\:localtime\\:${epochS}}`;
    await ffmpeg([
      "-f", "lavfi", "-i", `color=c=${t.ground}:s=${t.width}x${t.height}:r=${FPS}`,
      "-f", "lavfi", "-i", pattern2,
      "-t", String(seconds),
      "-filter_complex",
      [
        "[0][1]overlay=x=(W-w)/2:y=H*0.12",
        `drawtext=fontfile='${FONT}':text='${text}':fontsize=${Math.round(t.height / 11)}:fontcolor=white:x=(w-tw)/2:y=h*0.64`,
      ].join(","),
      "-c:v", "libx264", "-preset", "veryfast", "-crf", "44",
      "-g", String(FPS * 2), "-pix_fmt", "yuv420p",
      "-f", "segment", "-segment_time", String(VIDEO_SEGMENT_MS / 1000), "-reset_timestamps", "1",
      pattern,
    ]);
  }
  return (await readdir(tmp)).filter((f) => f.endsWith(".mp4")).sort().map((f) => join(tmp, f));
}

async function buildTrack(dir: string, t: TrackPlan): Promise<SessionTrack> {
  const trackDir = join(dir, t.id);
  await mkdir(trackDir, { recursive: true });
  const segLen = t.kind === "mic" || t.kind === "system-audio" ? AUDIO_SEGMENT_MS : VIDEO_SEGMENT_MS;
  const segments: SessionSegment[] = [];
  for (const [i, run] of t.runs.entries()) {
    const tmp = join(dir, `.tmp-${t.id}-${i}`);
    const pieces = await encodeRun(t, run, tmp);
    for (const [k, piece] of pieces.entries()) {
      const startMs = run.from + k * segLen;
      const file = segmentName(startMs);
      await rename(piece, join(trackDir, file));
      const { size } = await stat(join(trackDir, file));
      segments.push({
        file,
        start_epoch_ms: startMs,
        duration_ms: Math.min(segLen, run.to - startMs),
        bytes: size,
      });
    }
    await rm(tmp, { recursive: true, force: true });
  }
  const isVideo = t.kind === "screen" || t.kind === "webcam";
  return {
    id: t.id,
    kind: t.kind,
    label: t.label,
    monitor: t.monitor,
    codec: isVideo ? "h264" : "aac",
    width: t.width,
    height: t.height,
    fps: isVideo ? FPS : 0,
    start_epoch_ms: t.runs[0]?.from ?? 0,
    dir: t.id,
    segments,
    gaps: t.gaps,
    status: "stopped",
    restarts: t.gaps.filter((g) => g.reason !== IDLE_REASON && g.reason !== "paused").length,
  };
}

function summarize(m: SessionManifest): SessionSummary {
  let bytes = 0;
  let segment_count = 0;
  for (const t of m.tracks) {
    segment_count += t.segments.length;
    for (const s of t.segments) bytes += s.bytes;
  }
  return {
    id: m.id,
    started_epoch_ms: m.started_epoch_ms,
    ended_epoch_ms: m.ended_epoch_ms,
    duration_ms: (m.ended_epoch_ms ?? m.started_epoch_ms) - m.started_epoch_ms,
    track_count: m.tracks.length,
    segment_count,
    bytes,
    has_manifest: true,
  };
}

async function buildSession(plan: SessionPlan): Promise<SessionManifest> {
  const dir = join(ROOT, plan.id);
  const manifestPath = join(dir, "session.json");
  if (!FORCE && existsSync(manifestPath)) {
    console.log(`kept     ${plan.id} (pass --force to rebuild)`);
    return (await Bun.file(manifestPath).json()) as SessionManifest;
  }
  await rm(dir, { recursive: true, force: true });
  await mkdir(dir, { recursive: true });
  const t0 = performance.now();
  // Three encoders at a time: all seven at once can run ffmpeg out of memory.
  const tracks: SessionTrack[] = [];
  for (let i = 0; i < plan.tracks.length; i += 3) {
    tracks.push(...(await Promise.all(plan.tracks.slice(i, i + 3).map((t) => buildTrack(dir, t)))));
  }
  const manifest: SessionManifest = {
    version: 1,
    id: plan.id,
    started_epoch_ms: plan.start,
    ended_epoch_ms: plan.end,
    segment_seconds: VIDEO_SEGMENT_MS / 1000,
    desktop: plan.desktop,
    tracks,
    markers: plan.markers,
    rebuilt: false,
    cuts: [],
  };
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2));
  const s = summarize(manifest);
  console.log(
    `wrote    ${plan.id}: ${s.segment_count} segments, ${(s.bytes / 1e6).toFixed(0)} MB in ${((performance.now() - t0) / 1000).toFixed(1)}s`,
  );
  return manifest;
}

const manifests: SessionManifest[] = [];
for (const plan of [mainDay(), shortDay(), ...libraryDays()]) manifests.push(await buildSession(plan));
// The same list `list_sessions` returns, newest first; `listSessions()` reads it
// when there is no Tauri backend.
await writeFile(
  join(ROOT, "index.json"),
  JSON.stringify(manifests.map(summarize).sort((a, b) => b.started_epoch_ms - a.started_epoch_ms), null, 2),
);
console.log(`fixtures in ${ROOT}`);
