import { invoke } from "@tauri-apps/api/core";
import type {
  DateEntries,
  DisplayOutputInfo,
  DshowDevices,
  MonitorConfig,
  CaptureConfig,
  CaptureStatus,
  SessionManifest,
  SessionMarker,
  SessionOptions,
  SessionStatus,
  SessionSummary,
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

export async function setOutputDir(dir: string): Promise<void> {
  return invoke("set_output_dir", { dir });
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
    return fetchJson<SessionManifest>(`sessions/${id}/session.json`);
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

/** DXGI outputs with the recorder's own `screen_index` and a real capturable flag. */
export async function listDisplayOutputs(): Promise<DisplayOutputInfo[]> {
  return invoke<DisplayOutputInfo[]>("list_display_outputs");
}
