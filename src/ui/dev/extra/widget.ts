/**
 * Mock IPC handlers owned by `widget` (round 2). Return `UNHANDLED` for any
 * command this file doesn't answer; the core mock in ../mockIpc.ts tries the
 * extras before its own switch.
 */
import { UNHANDLED, type MockCtx, type MockResult } from "./ctx";
import type {
  Cut,
  DisplayOutputInfo,
  FfmpegSetup,
  SessionManifest,
  SessionMarker,
  SessionOptions,
  SessionStatus,
  Settings,
  TrackSummary,
} from "../../types";

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

/* ---- the recorder --------------------------------------------------------- *
 * A live session for the widget, built from the newest fixture so the desk
 * map's tiles have real thumbnails. `?scene=` opens it in a given state for
 * screenshots: `recording` (screen 4 restarting), `alert` (screens 1–3 down
 * for 14 min), `paused`, `firstrun` (not onboarded), `noffmpeg`.
 * -------------------------------------------------------------------------- */

type Scene = "recording" | "alert" | "paused" | "firstrun" | "noffmpeg" | null;

function sceneParam(): Scene {
  const v = new URLSearchParams(window.location.search).get("scene");
  return v === "recording" || v === "alert" || v === "paused" || v === "firstrun" || v === "noffmpeg" ? v : null;
}

interface Hotkeys {
  record: string;
  mark: string;
  pause: string;
}

interface MockLive {
  fixture: SessionManifest;
  started: number;
  pausedSince: number | null;
  /** track id → when its outage began */
  down: Map<string, number>;
  /** track id → restart count, for a track that is restarting */
  restarting: Map<string, number>;
  markers: SessionMarker[];
}

const CHANGED_EVENT = "recorder-changed";
const MARK_EVENT = "recorder-mark";
const SCENE_ELAPSED_MS = (2 * 3600 + 14 * 60 + 31) * 1000;
const FFMPEG = String.raw`C:\ffmpeg\bin\ffmpeg.exe`;

const scene = sceneParam();
let live: MockLive | null = null;
let liveBooted = false;
let onboarded = scene !== "firstrun";
let ffmpegFound = scene !== "noffmpeg";
let session: Partial<SessionOptions> | null = null;
let hotkeys: Hotkeys = { record: "Ctrl+Alt+R", mark: "Ctrl+Alt+M", pause: "Ctrl+Alt+P" };
let contentProtection = true;
let discardShort = false;

/** The fixture with the most screens: the "desk" the mock records from. */
function newest(ctx: MockCtx): SessionManifest | null {
  let best: SessionManifest | null = null;
  for (const id of ctx.sessionIds()) {
    const m = ctx.session(id);
    const n = m.tracks.filter((t) => t.kind === "screen").length;
    if (!best || n > best.tracks.filter((t) => t.kind === "screen").length) best = m;
  }
  return best;
}

function startLive(ctx: MockCtx, started: number): MockLive {
  const fixture = newest(ctx);
  if (!fixture) throw new Error("No fixture session to record into");
  return { fixture, started, pausedSince: null, down: new Map(), restarting: new Map(), markers: [] };
}

/** The scene's session, made on the first recorder call so the fixtures are loaded. */
function bootScene(ctx: MockCtx): void {
  if (liveBooted) return;
  liveBooted = true;
  if (scene !== "recording" && scene !== "alert" && scene !== "paused") return;
  const now = Date.now();
  live = startLive(ctx, now - SCENE_ELAPSED_MS);
  live.markers = [
    { epoch_ms: now - 22 * 60_000, label: "" },
    { epoch_ms: now - 6 * 60_000, label: "" },
  ];
  if (scene === "recording") live.restarting.set("screen-3", 2);
  if (scene === "alert") for (const id of ["screen-0", "screen-1", "screen-2"]) live.down.set(id, now - 14 * 60_000);
  if (scene === "paused") live.pausedSince = now - 3 * 60_000;
}

function trackSummary(l: MockLive, t: SessionManifest["tracks"][number]): TrackSummary {
  const last = t.segments[t.segments.length - 1];
  const downSince = l.down.get(t.id) ?? null;
  const restarts = l.restarting.get(t.id) ?? 0;
  const troubled = downSince !== null || restarts > 0;
  return {
    id: t.id,
    kind: t.kind,
    label: t.label,
    status: troubled ? "restarting" : "recording",
    codec: t.codec,
    width: t.width,
    height: t.height,
    fps: t.fps,
    start_epoch_ms: l.started,
    segment_count: t.segments.length,
    bytes: t.segments.reduce((a, s) => a + s.bytes, 0),
    last_segment_end_ms: last ? last.start_epoch_ms + last.duration_ms : null,
    gap_count: t.gaps.length,
    restarts,
    error: null,
    last_error_human: downSince !== null ? "Display was locked or asleep" : restarts > 0 ? "FFmpeg stopped unexpectedly" : null,
    waiting_since_epoch_ms: downSince,
    next_attempt_epoch_ms: troubled ? Date.now() + 8_000 : null,
  };
}

function liveStatus(l: MockLive | null): SessionStatus {
  const now = Date.now();
  const tracks = l ? l.fixture.tracks.map((t) => trackSummary(l, t)) : [];
  return {
    recording: l !== null,
    session_id: l?.fixture.id ?? null,
    session_dir: null,
    started_epoch_ms: l?.started ?? null,
    elapsed_ms: l ? now - l.started : 0,
    segment_seconds: 60,
    tracks,
    total_bytes: tracks.reduce((a, t) => a + t.bytes, 0),
    segment_count: tracks.reduce((a, t) => a + t.segment_count, 0),
    warnings: [],
    error: null,
    disk_free_bytes: 812e9,
    min_free_bytes: 2e9,
  };
}

function control(): { recording: boolean; session_id: string | null; paused_since_epoch_ms: number | null } {
  return { recording: live !== null, session_id: live?.fixture.id ?? null, paused_since_epoch_ms: live?.pausedSince ?? null };
}

function announce(ctx: MockCtx): void {
  ctx.emit(CHANGED_EVENT, control());
}

function displayOutputs(ctx: MockCtx): DisplayOutputInfo[] {
  const m = newest(ctx);
  if (!m) return [];
  return m.tracks.flatMap((t) => {
    const n = /^screen-(\d+)$/.exec(t.id);
    if (!n || !t.monitor) return [];
    const idx = Number(n[1]);
    const out: DisplayOutputInfo = {
      adapter: t.monitor.adapter,
      output_idx: t.monitor.output_idx,
      device_name: String.raw`\\.\DISPLAY` + String(idx + 1),
      adapter_name: "NVIDIA GeForce RTX 4090",
      x: t.monitor.x,
      y: t.monitor.y,
      w: t.monitor.w,
      h: t.monitor.h,
      attached: true,
      capturable: true,
      screen_index: idx,
    };
    return [out];
  });
}

function ffmpegSetup(): FfmpegSetup {
  return {
    ffmpeg_path: ffmpegFound ? FFMPEG : null,
    ffprobe_path: ffmpegFound ? FFMPEG.replace("ffmpeg.exe", "ffprobe.exe") : null,
    managed: false,
    manual_dir: null,
    install_dir: String.raw`%LOCALAPPDATA%\com.chirptime.app\ffmpeg`,
    download_url: "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
    download_bytes: 88_000_000,
    version: "7.1",
    installing: false,
  };
}

function settings(): Settings & { hotkeys: Hotkeys; content_protection: boolean; discard_short_sessions: boolean } {
  return {
    version: 2,
    capture: {
      interval_minutes: 10,
      video_mode: true,
      video_duration_seconds: 5,
      video_framerate: 30,
      video_preset: "ultrafast",
      record_audio: true,
      audio_device: null,
      pip_overlay: true,
      webcam_device: null,
      output_dir: String.raw`G:\code\chirptime\fixtures`,
    },
    session,
    onboarded,
    ffmpeg_dir: null,
    hotkeys: { ...hotkeys },
    content_protection: contentProtection,
    discard_short_sessions: discardShort,
  };
}

function hotkeyStatus() {
  return (["record", "mark", "pause"] as const).map((action) => ({
    action,
    accelerator: hotkeys[action],
    registered: hotkeys[action] !== "",
    error: null,
  }));
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** The install, with the real progress events, at mock speed. */
async function installFfmpeg(ctx: MockCtx): Promise<string> {
  const total = 88_000_000;
  for (let i = 1; i <= 10; i++) {
    await new Promise((r) => window.setTimeout(r, 150));
    ctx.emit("ffmpeg-install-progress", { phase: "downloading", downloaded_bytes: (total * i) / 10, total_bytes: total });
  }
  for (const phase of ["verifying", "extracting", "checking", "done"] as const) {
    await new Promise((r) => window.setTimeout(r, 250));
    ctx.emit("ffmpeg-install-progress", { phase, downloaded_bytes: total, total_bytes: total });
  }
  ffmpegFound = true;
  return FFMPEG;
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

    // Window calls from the widget: nothing to do in a browser tab.
    case "plugin:window|set_size":
    case "plugin:window|start_dragging":
    case "plugin:window|minimize":
    case "plugin:window|close":
    case "plugin:window|hide":
      return null;
    case "plugin:app|version":
      return "1.0.0";
  }

  bootScene(ctx);
  switch (cmd) {
    case "get_settings":
      return settings();
    case "set_session_options":
      if (isRecord(a.options)) session = { ...a.options };
      return null;
    case "set_onboarded":
      onboarded = a.value === true;
      return null;
    case "list_display_outputs":
      return displayOutputs(ctx);
    case "list_devices":
      return { video: ["Logitech BRIO"], audio: ["Microphone (Logitech BRIO)", "Headset (WH-1000XM5 Hands-Free)"] };
    case "get_ffmpeg_setup":
      return ffmpegSetup();
    case "install_ffmpeg":
      return installFfmpeg(ctx);
    case "pick_ffmpeg_dir":
    case "pick_output_dir":
    case "clear_ffmpeg_dir":
    case "open_folder":
    case "open_app":
    case "open_viewer":
    case "show_recorder_window":
      return null;

    case "get_recorder_control":
      return control();
    case "get_session_status":
      return live ? liveStatus(live) : UNHANDLED;
    case "start_recording":
      if (live) throw new Error("A session is already recording");
      live = startLive(ctx, Date.now());
      announce(ctx);
      return liveStatus(live);
    case "stop_recording": {
      const out = { status: { ...liveStatus(live), recording: false }, discarded: null };
      live = null;
      announce(ctx);
      return out;
    }
    case "pause_session":
      if (!live) throw new Error("No session is recording");
      live.pausedSince = Date.now();
      announce(ctx);
      return control();
    case "resume_session":
      if (!live) throw new Error("No session is recording");
      live.pausedSince = null;
      announce(ctx);
      return control();
    case "restart_screens": {
      if (!live) throw new Error("No session is recording");
      const n = live.down.size;
      live.down.clear();
      live.restarting.clear();
      announce(ctx);
      return n;
    }
    case "mark_now": {
      if (!live) throw new Error("No session is recording");
      if (live.pausedSince !== null) throw new Error("Paused: resume to mark");
      const m: SessionMarker = { epoch_ms: Date.now(), label: typeof a.label === "string" ? a.label : "" };
      live.markers.push(m);
      ctx.emit(MARK_EVENT, m);
      return m;
    }
    case "set_live_marker_label": {
      const at = num(a, "epochMs");
      const mk = live?.markers.find((m) => m.epoch_ms === at);
      if (!mk) throw new Error("That mark is no longer in this session");
      mk.label = text(a, "label").trim();
      return { ...mk };
    }
    case "get_session":
      // The live session's marks are the widget's own, not the fixture's.
      if (live && a.id === live.fixture.id) {
        return { ...live.fixture, ended_epoch_ms: null, markers: live.markers.map((m) => ({ ...m })) };
      }
      return UNHANDLED;

    case "get_hotkeys":
      return hotkeyStatus();
    case "set_hotkeys":
      if (isRecord(a.hotkeys)) {
        const h = a.hotkeys;
        hotkeys = {
          record: typeof h.record === "string" ? h.record : hotkeys.record,
          mark: typeof h.mark === "string" ? h.mark : hotkeys.mark,
          pause: typeof h.pause === "string" ? h.pause : hotkeys.pause,
        };
      }
      return hotkeyStatus();
    case "set_content_protection":
      contentProtection = a.enabled === true;
      return null;
    case "set_discard_short_sessions":
      discardShort = a.enabled === true;
      return null;
    default:
      return UNHANDLED;
  }
}
