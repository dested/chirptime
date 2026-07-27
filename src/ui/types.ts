export interface CaptureEntry {
  timestamp: string;
  display_time: string;
  screen_video: string | null;
  webcam_video: string | null;
  combined_video: string | null;
  screenshot: string | null;
}

export interface MonitorRegion {
  id: number;
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface MonitorConfig {
  totalWidth: number;
  totalHeight: number;
  monitors: MonitorRegion[];
}

export interface DateEntries {
  date: string;
  entries: CaptureEntry[];
  full_day_video: string | null;
}

export interface CaptureConfig {
  interval_minutes: number;
  video_mode: boolean;
  video_duration_seconds: number;
  video_framerate: number;
  video_preset: string;
  record_audio: boolean;
  audio_device: string | null;
  pip_overlay: boolean;
  webcam_device: string | null;
  output_dir: string;
}

export interface CaptureStatus {
  running: boolean;
  last_capture_time: string | null;
  next_capture_time: string | null;
  captures_today: number;
}

/* ------------------------------------------------------------------ *
 * Continuous session recording — mirrors the manifest schema in
 * plans/2026-07-27-continuous-recording.md §3. Backend writes, we read.
 * ------------------------------------------------------------------ */

export type TrackKind = "screen" | "webcam" | "mic" | "system-audio";

/** `session::TrackStatus`, serialized kebab-case. */
export type TrackStatus = "starting" | "recording" | "restarting" | "stopped" | "failed";

export interface SessionSegment {
  /** file name only; full path is <session>/<track.dir>/<file> */
  file: string;
  start_epoch_ms: number;
  duration_ms: number;
  bytes: number;
}

export interface TrackMonitor {
  output_idx: number;
  adapter: number;
  /** physical desktop pixels */
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface TrackGap {
  from_epoch_ms: number;
  to_epoch_ms: number;
  reason: string;
}

export interface SessionTrack {
  id: string;
  kind: TrackKind;
  label: string;
  monitor: TrackMonitor | null;
  codec: string;
  width: number;
  height: number;
  fps: number;
  /** measured, per track — never assumed equal across tracks */
  start_epoch_ms: number;
  dir: string;
  segments: SessionSegment[];
  gaps: TrackGap[];
  /** additive to the §3 sketch — lets the UI tell "never started" from "fine" */
  status: TrackStatus;
  /** omitted by serde when there is nothing wrong */
  error?: string | null;
  restarts: number;
}

export interface SessionMarker {
  epoch_ms: number;
  label: string;
}

export interface DesktopRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface SessionManifest {
  version: number;
  id: string;
  started_epoch_ms: number;
  /** null while still recording */
  ended_epoch_ms: number | null;
  segment_seconds: number;
  desktop: DesktopRect;
  tracks: SessionTrack[];
  markers: SessionMarker[];
  /** set when the session stopped for a reason other than the user asking */
  error?: string | null;
  /** true when this manifest was reconstructed by scanning the directory */
  rebuilt: boolean;
}

/**
 * Row in the session browser. Mirrors `recorder::SessionSummary` exactly —
 * `list_sessions` returns this, so nothing here may be invented on our side.
 * "Is it still recording" is derived from `ended_epoch_ms === null`, not a field.
 */
export interface SessionSummary {
  id: string;
  started_epoch_ms: number;
  ended_epoch_ms: number | null;
  duration_ms: number;
  track_count: number;
  segment_count: number;
  bytes: number;
  /** false when the directory has no readable session.json (rebuild candidate) */
  has_manifest: boolean;
}

/**
 * Live health of one track while recording. Mirrors `commands::TrackSummary`.
 * This is the *status* view — `SessionTrack` is the manifest view, and the two
 * deliberately carry different fields.
 */
export interface TrackSummary {
  id: string;
  kind: TrackKind;
  label: string;
  status: TrackStatus;
  codec: string;
  width: number;
  height: number;
  fps: number;
  start_epoch_ms: number;
  segment_count: number;
  bytes: number;
  last_segment_end_ms: number | null;
  gap_count: number;
  restarts: number;
  error: string | null;
}

/** Mirrors `commands::SessionStatus`. Returned by start/stop/get_session_status. */
export interface SessionStatus {
  recording: boolean;
  session_id: string | null;
  session_dir: string | null;
  started_epoch_ms: number | null;
  elapsed_ms: number;
  segment_seconds: number;
  tracks: TrackSummary[];
  total_bytes: number;
  segment_count: number;
  /** device selection and monitor resolution notes — never silently dropped */
  warnings: string[];
  error: string | null;
  disk_free_bytes: number | null;
  /** the recorder stops cleanly below this; colour the free number against it */
  min_free_bytes: number;
}

/**
 * Mirrors `recorder::SessionOptions`. Rust applies `#[serde(default)]`, so a
 * partial object is legal — we always send a whole one so what the settings
 * panel shows is exactly what gets recorded.
 */
export interface SessionOptions {
  segment_seconds: number;
  screen_fps: number;
  record_screens: boolean;
  /** screen indices (the N in `screen-N`); empty = every capturable screen */
  screens: number[];
  record_webcam: boolean;
  webcam_device: string | null;
  webcam_size: string;
  webcam_fps: number;
  record_mic: boolean;
  /** naming a Bluetooth device here is the only way to record one */
  mic_device: string | null;
  record_system_audio: boolean;
  mic_sidecar_16k: boolean;
}

/** Mirrors `commands::DisplayOutputInfo` (a flattened `recorder::DisplayOutput`). */
export interface DisplayOutputInfo {
  adapter: number;
  /** per adapter, not global — never use it to name a track */
  output_idx: number;
  device_name: string;
  adapter_name: string;
  x: number;
  y: number;
  w: number;
  h: number;
  attached: boolean;
  /** false when there is no working capture+encode path for this output */
  capturable: boolean;
  /** the N in `screen-N`; null for an unattached output */
  screen_index: number | null;
}

export interface DshowDevices {
  video: string[];
  audio: string[];
}

export interface AppState {
  dates: string[];
  selectedDate: string | null;
  dateEntry: DateEntries | null;
  selectedTimestamp: string | null;
  monitorConfig: MonitorConfig | null;
  zoomedMonitor: number | null;
  loading: boolean;
  captureStatus: CaptureStatus;
  outputDir: string;
}
