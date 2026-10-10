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
 * Persisted settings + first-run onboarding. Mirrors `settings.rs` and
 * `commands::EnvironmentInfo`.
 * ------------------------------------------------------------------ */

/** Mirrors `settings::Settings` — the whole of what survives a restart. */
export interface Settings {
  version: number;
  capture: CaptureConfig;
  /**
   * Last-used `SessionOptions`, stored opaquely by the backend. `null` means
   * never configured — merge over `DEFAULT_OPTIONS` rather than trusting it to
   * be complete.
   */
  session: Partial<SessionOptions> | null;
  onboarded: boolean;
  /** A folder the user pointed at by hand, or null for the normal lookup order. */
  ffmpeg_dir: string | null;
}

/**
 * Mirrors `commands::FfmpegSetup`. Everything the first-run panel needs to
 * decide whether to offer the download and what to say about it — the size is
 * here so the user is told before agreeing, never after.
 */
export interface FfmpegSetup {
  /** Non-null means a usable FFmpeg was found; nothing is offered. */
  ffmpeg_path: string | null;
  /** Resolved separately — the session recorder needs both halves. */
  ffprobe_path: string | null;
  managed: boolean;
  manual_dir: string | null;
  install_dir: string;
  download_url: string;
  download_bytes: number;
  version: string;
  installing: boolean;
}

/** Payload of the `ffmpeg-install-progress` event. */
export interface FfmpegProgress {
  phase: "downloading" | "verifying" | "extracting" | "checking" | "done";
  downloaded_bytes: number;
  total_bytes: number;
}

/** Mirrors `settings::LegacyOutput`. Reported only; nothing is ever moved. */
export interface LegacyOutput {
  path: string;
  day_count: number;
  session_count: number;
}

/** Mirrors `commands::EnvironmentInfo`. */
export interface EnvironmentInfo {
  /** resolved and absolute — never `%USERPROFILE%\...` */
  output_dir: string;
  output_dir_exists: boolean;
  disk_free_bytes: number | null;
  disk_total_bytes: number | null;
  /** null when ffmpeg is not installed — nothing records without it */
  ffmpeg_path: string | null;
  nvenc: boolean;
  legacy_output: LegacyOutput | null;
  settings_path: string;
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
  /** deleted ranges, pending or final (never undone) — holes playback skips */
  cuts: Cut[];
}

/** `session::CutState`, serialized kebab-case. */
export type CutState = "pending" | "final" | "undone";

/** Mirrors `session::Cut`: a deleted block of time, remembered forever. */
export interface Cut {
  /** `cut-<epoch_ms of creation>` */
  id: string;
  from_epoch_ms: number;
  to_epoch_ms: number;
  /** omitted by serde when the cut covers every track */
  tracks?: string[] | null;
  created_epoch_ms: number;
  /** pending = still undoable, files parked; final = disk freed */
  state: CutState;
  freed_bytes: number;
}

/** Mirrors `edit::CutReceipt`, returned by `delete_range`. */
export interface CutReceipt {
  cut: Cut;
  freed_bytes_estimate: number;
  /** after this, `undoCut` rejects */
  undo_until_epoch_ms: number;
}

/** Mirrors `edit::SessionTrashReceipt`, returned by `delete_session`. */
export interface SessionTrashReceipt {
  id: string;
  bytes: number;
  undo_until_epoch_ms: number;
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
  /** Why the track is not recording, as a sentence ("Display was locked or asleep"). Null while it records. */
  last_error_human: string | null;
  /** Set while a screen waits out an outage. It keeps retrying until capture works or the session stops. */
  waiting_since_epoch_ms: number | null;
  /** When the next attempt is due, while it is restarting or waiting. */
  next_attempt_epoch_ms: number | null;
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
  /**
   * Delete any segment that covered a stretch with no keyboard or mouse input,
   * once the idle stretch has run longer than this many seconds. `0` keeps
   * everything. Windows-only; ignored on other platforms.
   */
  drop_idle_after_seconds: number;
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
