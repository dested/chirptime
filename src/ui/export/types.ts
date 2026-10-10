/** Mirrors of the `export.rs` request, estimate, progress and index types. Only the export code reads these. */

export type Layout = "devlog" | "short" | "side-by-side" | "desk";
export type Corner = "top-left" | "top-right" | "bottom-left" | "bottom-right";
export type Shape = "rounded" | "circle";
export type Speed = "1x" | "2x" | "4x" | "timelapse";
export type SizeMode = "original" | "1080p" | "share";

export interface CameraOpts {
  enabled: boolean;
  corner: Corner;
  shape: Shape;
  /** the corner camera's width as a fraction of the frame's, 0.1–0.5 */
  size: number;
}

export interface SoundOpts {
  /** gain in dB; null leaves the mic out */
  micDb: number | null;
  /** gain in dB; null leaves system audio out */
  systemDb: number | null;
  /** "Lower system audio while I talk" */
  duck: boolean;
}

export interface ExportRequest {
  sessionId: string;
  fromMs: number;
  toMs: number;
  layout: Layout;
  /** a screen track id, or null to follow the mouse */
  screen: string | null;
  camera: CameraOpts;
  sound: SoundOpts;
  speed: Speed;
  skipIdle: boolean;
  size: SizeMode;
  /** track ids to copy as they are into a folder instead of composing */
  rawTracks: string[] | null;
  title: string | null;
}

export interface ExportEstimate {
  outputMs: number;
  sourceMs: number;
  bytes: number;
  width: number;
  height: number;
  encoder: string;
  encodeSeconds: number;
  /** said in words under the estimate, e.g. the follow fallback */
  notes: string[];
  /** why this export can't run, in words */
  error: string | null;
}

export type ExportJobState = "running" | "done" | "failed" | "cancelled";

/** The `export-progress` event. */
export interface ExportProgress {
  jobId: string;
  fraction: number;
  etaSeconds: number | null;
  state: ExportJobState;
  error?: string | null;
  outputPath?: string | null;
}

/** One line of `exports/index.jsonl`. `path` and `thumb` are relative to the recordings folder. */
export interface ExportRecord {
  id: string;
  sessionId: string;
  from: number;
  to: number;
  layout: string;
  path: string;
  bytes: number;
  durationMs: number;
  createdEpochMs: number;
  title: string;
  thumb: string | null;
}

/** What Save preset keeps: everything but the range and the session's own screens. */
export interface ExportPreset {
  layout: Layout;
  follow: boolean;
  camera: CameraOpts;
  sound: SoundOpts;
  speed: Speed;
  skipIdle: boolean;
  size: SizeMode;
}
