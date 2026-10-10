/**
 * Browser dev harness: answers `invoke()` from the fixture sessions so the app
 * window runs in a plain browser. Loaded only when `import.meta.env.DEV` and
 * `?mock=1` (see main.tsx); never part of a build.
 *
 * Manifests come from `scripts/fixture-server.ts` (port 9120), which also serves
 * the media and thumbnails. Deletes and undos mutate an in-memory copy with the
 * backend's timing (undo for 10 s, final at 12 s), so the whole delete flow is
 * clickable; a reload starts clean.
 */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { InvokeArgs } from "@tauri-apps/api/core";
import type {
  Cut,
  CutReceipt,
  EnvironmentInfo,
  SessionManifest,
  SessionSegment,
  SessionStatus,
  SessionSummary,
  SessionTrashReceipt,
  Settings,
} from "../types";
import { mediaUrl } from "../hooks/mediaUrl";
import { summarizeSession } from "../hooks/useApi";

const UNDO_WINDOW_MS = 10_000;
const FINALIZE_AFTER_MS = 12_000;
const LIVE_SESSION_ERR = "Stop recording before deleting from this session";
const UNDO_EXPIRED_ERR = "That delete can no longer be undone";

/** What a pending cut parked, so undo can put it back. */
interface Parked {
  sessionId: string;
  segments: { trackId: string; segment: SessionSegment }[];
}

const manifests = new Map<string, SessionManifest>();
const parked = new Map<string, Parked>();
const trashed = new Map<string, { manifest: SessionManifest; until: number }>();
let order: string[] = [];
let recording: { id: string; started: number } | null = null;

async function fetchJson<T>(rel: string): Promise<T> {
  const res = await fetch(mediaUrl(rel), { cache: "no-store" });
  if (!res.ok) throw new Error(`${res.status} ${rel} — is scripts/fixture-server.ts running?`);
  return (await res.json()) as T;
}

async function load(): Promise<void> {
  const index = await fetchJson<SessionSummary[]>("sessions/index.json");
  order = index.map((s) => s.id);
  await Promise.all(
    index.map(async (s) => {
      const m = await fetchJson<SessionManifest>(`sessions/${s.id}/session.json`);
      manifests.set(s.id, { ...m, cuts: m.cuts ?? [] });
    }),
  );
}

/* ---- argument narrowing --------------------------------------------------- */

function args(payload: InvokeArgs | undefined): Record<string, unknown> {
  if (!payload || Array.isArray(payload) || payload instanceof ArrayBuffer || payload instanceof Uint8Array) return {};
  return payload;
}

function str(a: Record<string, unknown>, k: string): string {
  const v = a[k];
  if (typeof v !== "string") throw new Error(`mock: ${k} must be a string`);
  return v;
}

function int(a: Record<string, unknown>, k: string): number {
  const v = a[k];
  if (typeof v !== "number" || !Number.isFinite(v)) throw new Error(`mock: ${k} must be a number`);
  return Math.round(v);
}

function strList(a: Record<string, unknown>, k: string): string[] | null {
  const v = a[k];
  if (v === null || v === undefined) return null;
  if (!Array.isArray(v) || !v.every((x): x is string => typeof x === "string")) throw new Error(`mock: ${k} must be a list of strings`);
  return v;
}

function session(id: string): SessionManifest {
  const m = manifests.get(id);
  if (!m) throw new Error(`No session named ${id}`);
  return m;
}

const clone = <T,>(v: T): T => structuredClone(v);

/* ---- edits ------------------------------------------------------------------ */

function bytesInside(s: SessionSegment, from: number, to: number): number {
  const a = Math.max(from, s.start_epoch_ms);
  const b = Math.min(to, s.start_epoch_ms + s.duration_ms);
  if (b <= a || s.duration_ms <= 0) return 0;
  return Math.round((s.bytes * (b - a)) / s.duration_ms);
}

function deleteRange(id: string, from: number, to: number, only: string[] | null): CutReceipt {
  if (recording?.id === id) throw new Error(LIVE_SESSION_ERR);
  const m = session(id);
  if (to <= from) throw new Error("The end of the range must come after its start");
  if (only && only.length === 0) throw new Error("Pick at least one track to delete from");
  for (const t of only ?? []) {
    if (!m.tracks.some((x) => x.id === t)) throw new Error(`This session has no track named "${t}"`);
  }
  const end = m.ended_epoch_ms ?? Date.now();
  if (to <= m.started_epoch_ms || from >= end) throw new Error("That range is outside this session");

  const now = Date.now();
  const cutId = `cut-${now}`;
  const park: Parked = { sessionId: id, segments: [] };
  let estimate = 0;
  for (const t of m.tracks) {
    if (only && !only.includes(t.id)) continue;
    const keep: SessionSegment[] = [];
    for (const s of t.segments) {
      const covered = s.start_epoch_ms >= from && s.start_epoch_ms + s.duration_ms <= to;
      if (covered) {
        park.segments.push({ trackId: t.id, segment: s });
        estimate += s.bytes;
      } else {
        estimate += bytesInside(s, from, to);
        keep.push(s);
      }
    }
    t.segments = keep;
  }
  const allTracks = !only || m.tracks.every((t) => only.includes(t.id));
  const cut: Cut = {
    id: cutId,
    from_epoch_ms: from,
    to_epoch_ms: to,
    tracks: allTracks ? null : only,
    created_epoch_ms: now,
    state: "pending",
    freed_bytes: 0,
  };
  m.cuts.push(cut);
  parked.set(cutId, park);
  window.setTimeout(() => finalize(id, cutId, estimate), FINALIZE_AFTER_MS);
  return { cut: clone(cut), freed_bytes_estimate: estimate, undo_until_epoch_ms: now + UNDO_WINDOW_MS };
}

function finalize(id: string, cutId: string, freed: number): void {
  const m = manifests.get(id);
  const c = m?.cuts.find((x) => x.id === cutId);
  if (!c || c.state !== "pending") return;
  c.state = "final";
  c.freed_bytes = freed;
  parked.delete(cutId);
}

function undoCut(id: string, cutId: string): SessionManifest {
  const m = session(id);
  const c = m.cuts.find((x) => x.id === cutId);
  const p = parked.get(cutId);
  if (!c || c.state !== "pending" || !p || Date.now() > c.created_epoch_ms + UNDO_WINDOW_MS) {
    throw new Error(UNDO_EXPIRED_ERR);
  }
  for (const { trackId, segment } of p.segments) {
    const t = m.tracks.find((x) => x.id === trackId);
    if (!t) continue;
    t.segments.push(segment);
    t.segments.sort((a, b) => a.start_epoch_ms - b.start_epoch_ms);
  }
  m.cuts = m.cuts.filter((x) => x.id !== cutId);
  parked.delete(cutId);
  return clone(m);
}

/* ---- status ----------------------------------------------------------------- */

function status(): SessionStatus {
  const now = Date.now();
  return {
    recording: recording !== null,
    session_id: recording?.id ?? null,
    session_dir: null,
    started_epoch_ms: recording?.started ?? null,
    elapsed_ms: recording ? now - recording.started : 0,
    segment_seconds: 60,
    tracks: [],
    total_bytes: 0,
    segment_count: 0,
    warnings: [],
    error: null,
    disk_free_bytes: 812e9,
    min_free_bytes: 2e9,
  };
}

const ENVIRONMENT: EnvironmentInfo = {
  output_dir: "G:\\code\\chirptime\\fixtures",
  output_dir_exists: true,
  disk_free_bytes: 812e9,
  disk_total_bytes: 2e12,
  ffmpeg_path: "C:\\ffmpeg\\bin\\ffmpeg.exe",
  nvenc: true,
  legacy_output: null,
  settings_path: "%APPDATA%\\com.chirptime.app\\settings.json",
};

const SETTINGS: Settings = {
  version: 1,
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
    output_dir: ENVIRONMENT.output_dir,
  },
  session: null,
  onboarded: true,
  ffmpeg_dir: null,
};

function handle(cmd: string, payload: InvokeArgs | undefined): unknown {
  const a = args(payload);
  switch (cmd) {
    case "list_sessions":
      return order.filter((id) => manifests.has(id)).map((id) => summarizeSession(session(id)));
    case "get_session":
      return clone(session(str(a, "id")));
    case "get_session_status":
      return status();
    case "get_environment":
      return { ...ENVIRONMENT };
    case "get_settings":
      return clone(SETTINGS);
    case "start_session":
      if (recording) throw new Error("A session is already recording");
      recording = { id: "mock-live", started: Date.now() };
      return status();
    case "stop_session":
      recording = null;
      return status();
    case "delete_range":
      return deleteRange(str(a, "id"), int(a, "fromEpochMs"), int(a, "toEpochMs"), strList(a, "tracks"));
    case "undo_cut":
      return undoCut(str(a, "id"), str(a, "cutId"));
    case "finalize_cuts": {
      const id = str(a, "id");
      for (const c of session(id).cuts) if (c.state === "pending") finalize(id, c.id, 0);
      return clone(session(id));
    }
    case "delete_session": {
      const id = str(a, "id");
      const m = session(id);
      if (recording?.id === id) throw new Error("Stop recording before deleting this session");
      const until = Date.now() + UNDO_WINDOW_MS;
      trashed.set(id, { manifest: m, until });
      manifests.delete(id);
      const receipt: SessionTrashReceipt = { id, bytes: summarizeSession(m).bytes, undo_until_epoch_ms: until };
      return receipt;
    }
    case "undo_delete_session": {
      const id = str(a, "id");
      const t = trashed.get(id);
      if (!t || Date.now() > t.until) throw new Error(UNDO_EXPIRED_ERR);
      manifests.set(id, t.manifest);
      trashed.delete(id);
      return null;
    }
  }
  throw new Error(`mock IPC: ${cmd} is not mocked`);
}

export async function installMockIpc(windowLabel: string): Promise<void> {
  mockWindows(windowLabel);
  try {
    await load();
  } catch (e) {
    console.error("[mockIpc] could not load fixtures:", e);
  }
  mockIPC((cmd, payload) => {
    // A small delay so loading states are real, and a rejection is a real
    // rejected promise rather than a synchronous throw.
    return new Promise((resolve, reject) => {
      window.setTimeout(() => {
        try {
          resolve(handle(cmd, payload));
        } catch (e) {
          reject(e instanceof Error ? e.message : String(e));
        }
      }, 40);
    });
  });
  console.info(`[mockIpc] answering invoke() from ${manifests.size} fixture sessions`);
}
