import { invoke } from "@tauri-apps/api/core";
import type {
  DateEntries,
  DisplayOutputInfo,
  DshowDevices,
  MonitorConfig,
  CaptureConfig,
  CaptureStatus,
  CutReceipt,
  EnvironmentInfo,
  FfmpegSetup,
  SessionManifest,
  SessionMarker,
  SessionOptions,
  SessionStatus,
  SessionSummary,
  SessionTrashReceipt,
  Settings,
} from "../types";
import { mediaUrl } from "./mediaUrl";

export async function fetchDates(): Promise<string[]> {
  return invoke<string[]>("get_dates");
}

export async function fetchDateEntries(date: string): Promise<DateEntries> {
  return invoke<DateEntries>("get_date_entries", { date });
}

export async function fetchMonitorConfig(): Promise<MonitorConfig> {
  return invoke<MonitorConfig>("get_monitor_config");
}

export async function fetchOutputDir(): Promise<string> {
  return invoke<string>("get_output_dir");
}

export async function deleteEntry(date: string, timestamp: string): Promise<void> {
  return invoke("delete_entry", { date, timestamp });
}

export async function getCaptureConfig(): Promise<CaptureConfig> {
  return invoke<CaptureConfig>("get_capture_config");
}

export async function setCaptureConfig(config: CaptureConfig): Promise<void> {
  return invoke("set_capture_config", { config });
}

export async function startCapture(): Promise<void> {
  return invoke("start_capture");
}

export async function stopCapture(): Promise<void> {
  return invoke("stop_capture");
}

export async function getCaptureStatus(): Promise<CaptureStatus> {
  return invoke<CaptureStatus>("get_capture_status");
}

export async function runSingleCapture(): Promise<string> {
  return invoke<string>("run_single_capture");
}

export async function listDevices(): Promise<DshowDevices> {
  return invoke<DshowDevices>("list_devices");
}

/** Point recording at `dir` and persist it. Returns the resolved absolute path. */
export async function setOutputDir(dir: string): Promise<string> {
  return invoke<string>("set_output_dir", { dir });
}

/* ---- settings + onboarding -------------------------------------------- */

/** Everything that survives a restart. */
export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/** Persist the session recorder's options. Rejects if they are not valid. */
export async function setSessionOptions(options: SessionOptions): Promise<void> {
  return invoke("set_session_options", { options });
}

/** Dismiss the first-run panel for good. */
export async function setOnboarded(value: boolean): Promise<void> {
  return invoke("set_onboarded", { value });
}

/** Output path, free space, ffmpeg/NVENC presence, and any legacy `output/`. */
export async function getEnvironment(): Promise<EnvironmentInfo> {
  return invoke<EnvironmentInfo>("get_environment");
}

/**
 * Native folder picker, applied and persisted by the backend. Resolves to
 * `null` when the user cancelled — that is not an error.
 */
export async function pickOutputDir(): Promise<string | null> {
  return invoke<string | null>("pick_output_dir");
}

/** Where FFmpeg is (or is not), and what a download would cost. */
export async function getFfmpegSetup(): Promise<FfmpegSetup> {
  return invoke<FfmpegSetup>("get_ffmpeg_setup");
}

/**
 * Download, verify and install FFmpeg. Only ever call this from an explicit
 * button — it spends ~105 MB of the user's bandwidth. Progress arrives as
 * `ffmpeg-install-progress` events; the promise resolves to the ffmpeg path.
 */
export async function installFfmpeg(): Promise<string> {
  return invoke<string>("install_ffmpeg");
}

/**
 * Point ChirpTime at an FFmpeg the user already has — the way out when the
 * download is blocked. Rejects a folder without both runnable binaries.
 * Resolves to `null` when the picker was cancelled.
 */
export async function pickFfmpegDir(): Promise<string | null> {
  return invoke<string | null>("pick_ffmpeg_dir");
}

/** Forget the hand-picked folder and go back to the normal lookup order. */
export async function clearFfmpegDir(): Promise<void> {
  return invoke("clear_ffmpeg_dir");
}

/** Show a folder in Explorer. No argument means the configured output folder. */
export async function openFolder(path?: string): Promise<void> {
  return invoke("open_folder", { path: path ?? null });
}

/**
 * Open (or re-point) the viewer window. `ts` is a clip's `HH-MM-SS` alongside
 * `date`, and an epoch-ms instant alongside `session`.
 */
export interface ViewerLink {
  date?: string;
  timestamp?: string;
  session?: string;
  tab?: "clips" | "sessions";
}

export async function openViewer(link: ViewerLink = {}): Promise<void> {
  return invoke("open_viewer", {
    date: link.date ?? null,
    timestamp: link.timestamp ?? null,
    session: link.session ?? null,
    tab: link.tab ?? null,
  });
}

/* ---- continuous sessions ---------------------------------------------- *
 * The Tauri commands are the primary source. Both fall back to reading the
 * on-disk JSON straight off the file server, which keeps the viewer usable
 * outside the Tauri shell and against a session directory the backend has not
 * indexed yet — the manifest is the contract, not the command.
 * ---------------------------------------------------------------------- */

async function fetchJson<T>(relPath: string): Promise<T> {
  const res = await fetch(mediaUrl(relPath), { cache: "no-store" });
  if (!res.ok) throw new Error(`${res.status} ${relPath}`);
  return (await res.json()) as T;
}

export function summarizeSession(m: SessionManifest): SessionSummary {
  let bytes = 0;
  let segment_count = 0;
  let end = m.ended_epoch_ms ?? m.started_epoch_ms;
  for (const t of m.tracks) {
    segment_count += t.segments.length;
    for (const s of t.segments) {
      bytes += s.bytes;
      end = Math.max(end, s.start_epoch_ms + s.duration_ms);
    }
  }
  return {
    id: m.id,
    started_epoch_ms: m.started_epoch_ms,
    ended_epoch_ms: m.ended_epoch_ms,
    duration_ms: end - m.started_epoch_ms,
    track_count: m.tracks.length,
    segment_count,
    bytes,
    has_manifest: true,
  };
}

export async function listSessions(): Promise<SessionSummary[]> {
  try {
    return await invoke<SessionSummary[]>("list_sessions");
  } catch {
    return fetchJson<SessionSummary[]>("sessions/index.json");
  }
}

export async function fetchSession(id: string): Promise<SessionManifest> {
  try {
    return await invoke<SessionManifest>("get_session", { id });
  } catch {
    const m = await fetchJson<SessionManifest>(`sessions/${id}/session.json`);
    // A manifest written before cuts existed has no `cuts` key.
    return { ...m, cuts: m.cuts ?? [] };
  }
}

/**
 * Begin continuous recording. Rejects with the backend's message if a session is
 * already running, or if no track could be started at all — every one of these
 * strings is written to be shown to the user verbatim.
 */
export async function startSession(options?: Partial<SessionOptions>): Promise<SessionStatus> {
  return invoke<SessionStatus>("start_session", { options: options ?? null });
}

export async function stopSession(): Promise<SessionStatus> {
  return invoke<SessionStatus>("stop_session");
}

export async function getSessionStatus(): Promise<SessionStatus> {
  return invoke<SessionStatus>("get_session_status");
}

/** Flag the current instant. Only valid while a session is recording. */
export async function addMarker(label: string): Promise<SessionMarker> {
  return invoke<SessionMarker>("add_marker", { label });
}

/** Reconstruct a manifest by scanning the session directory and probing every segment. */
export async function rebuildSession(id: string): Promise<SessionManifest> {
  return invoke<SessionManifest>("rebuild_session", { id });
}

/* ---- deleting time ------------------------------------------------------ *
 * Every delete is instant and undoable until `undo_until_epoch_ms` (10s); the
 * backend frees the disk ~12s after. All of these reject with a message meant
 * for the user, e.g. "Stop recording before deleting from this session".
 * ---------------------------------------------------------------------- */

/** Delete `[fromEpochMs, toEpochMs)` from a session; `tracks` limits it (default: all). */
export async function deleteRange(
  id: string,
  fromEpochMs: number,
  toEpochMs: number,
  tracks?: string[],
): Promise<CutReceipt> {
  return invoke<CutReceipt>("delete_range", {
    id,
    fromEpochMs,
    toEpochMs,
    tracks: tracks ?? null,
  });
}

/** Put a pending delete back. Resolves to the restored manifest. */
export async function undoCut(id: string, cutId: string): Promise<SessionManifest> {
  return invoke<SessionManifest>("undo_cut", { id, cutId });
}

/** Free the disk for every pending delete in a session now, giving up undo. */
export async function finalizeCuts(id: string): Promise<SessionManifest> {
  return invoke<SessionManifest>("finalize_cuts", { id });
}

/** Delete a whole session; it drops out of `listSessions` at once. */
export async function deleteSession(id: string): Promise<SessionTrashReceipt> {
  return invoke<SessionTrashReceipt>("delete_session", { id });
}

export async function undoDeleteSession(id: string): Promise<void> {
  return invoke("undo_delete_session", { id });
}

/** DXGI outputs with the recorder's own `screen_index` and a real capturable flag. */
export async function listDisplayOutputs(): Promise<DisplayOutputInfo[]> {
  return invoke<DisplayOutputInfo[]>("list_display_outputs");
}
