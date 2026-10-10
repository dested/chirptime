/**
 * The recorder's own commands (controls.rs, hotkeys.rs) and the settings
 * fields that came with them. Shared by the widget, Settings and first run.
 */
import { invoke } from "@tauri-apps/api/core";
import type { SessionMarker, SessionOptions, SessionStatus, SessionTrashReceipt, Settings } from "../types";

/** Mirrors `controls::RecorderControl`, also the `recorder-changed` payload. */
export interface RecorderControl {
  recording: boolean;
  session_id: string | null;
  paused_since_epoch_ms: number | null;
}

/** Mirrors `controls::StopOutcome`. */
export interface StopOutcome {
  status: SessionStatus;
  /** set when a session under a minute was discarded (it has the usual undo) */
  discarded: SessionTrashReceipt | null;
}

/** Mirrors `settings::Hotkeys`: accelerators like `Ctrl+Alt+R`, empty = off. */
export interface Hotkeys {
  record: string;
  mark: string;
  pause: string;
}

export type HotkeyAction = keyof Hotkeys;

/** Mirrors `hotkeys::HotkeyStatus`. */
export interface HotkeyStatus {
  action: HotkeyAction;
  accelerator: string;
  registered: boolean;
  error: string | null;
}

/** The v2 fields of `settings::Settings`, on top of what `types.ts` has. */
export interface RecorderPrefs {
  hotkeys: Hotkeys;
  content_protection: boolean;
  discard_short_sessions: boolean;
}

export const CHANGED_EVENT = "recorder-changed";
export const MARK_EVENT = "recorder-mark";
/** A stop discarded a session under a minute; the payload is its receipt, for Undo. */
export const DISCARDED_EVENT = "recorder-discarded";

export const DEFAULT_HOTKEYS: Hotkeys = { record: "Ctrl+Alt+R", mark: "Ctrl+Alt+M", pause: "Ctrl+Alt+P" };

/** Mirrors `recorder::SessionOptions::default()`. */
export const DEFAULT_OPTIONS: SessionOptions = {
  segment_seconds: 60,
  screen_fps: 30,
  record_screens: true,
  screens: [],
  record_webcam: true,
  webcam_device: null,
  webcam_size: "1920x1080",
  webcam_fps: 30,
  record_mic: true,
  mic_device: null,
  record_system_audio: true,
  mic_sidecar_16k: true,
  drop_idle_after_seconds: 60,
};

/** What a recorded hour of four screens costs on disk, for "about N hours". */
export const BYTES_PER_HOUR = 6.5e9;

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const str = (v: unknown, d: string): string => (typeof v === "string" ? v : d);
const bool = (v: unknown, d: boolean): boolean => (typeof v === "boolean" ? v : d);

/**
 * The v2 fields out of a `get_settings` reply, each defaulted when missing or
 * malformed (an older backend, or the dev mock).
 */
export function readPrefs(s: Settings): RecorderPrefs {
  const raw: unknown = s;
  const r = isRecord(raw) ? raw : {};
  const h = isRecord(r.hotkeys) ? r.hotkeys : {};
  return {
    hotkeys: {
      record: str(h.record, DEFAULT_HOTKEYS.record),
      mark: str(h.mark, DEFAULT_HOTKEYS.mark),
      pause: str(h.pause, DEFAULT_HOTKEYS.pause),
    },
    content_protection: bool(r.content_protection, true),
    discard_short_sessions: bool(r.discard_short_sessions, false),
  };
}

/** The saved options merged over the defaults, never trusted whole. */
export function readOptions(s: Settings): SessionOptions {
  return { ...DEFAULT_OPTIONS, ...(s.session ?? {}) };
}

/** `Ctrl+Alt+R` → `Ctrl Alt R`, the way the mocks print a hotkey. */
export function keyLabel(accelerator: string): string {
  return accelerator.split("+").map((k) => k.trim()).filter(Boolean).join(" ");
}

export const getRecorderControl = () => invoke<RecorderControl>("get_recorder_control");
export const startRecording = () => invoke<SessionStatus>("start_recording");
export const stopRecording = () => invoke<StopOutcome>("stop_recording");
export const pauseSession = () => invoke<RecorderControl>("pause_session");
export const resumeSession = () => invoke<RecorderControl>("resume_session");
/** Retry every screen that is waiting out an outage or gave up; resolves to how many were down. */
export const restartScreens = () => invoke<number>("restart_screens");
export const markNow = (label?: string) => invoke<SessionMarker>("mark_now", { label: label ?? null });
export const setLiveMarkerLabel = (epochMs: number, label: string) =>
  invoke<SessionMarker>("set_live_marker_label", { epochMs, label });
/** Open the app window on a route such as `#/settings` or `#/session/<id>`. */
export const openApp = (route: string) => invoke<void>("open_app", { route });
export const showRecorderWindow = () => invoke<void>("show_recorder_window");
export const setContentProtection = (enabled: boolean) => invoke<void>("set_content_protection", { enabled });
export const setDiscardShortSessions = (enabled: boolean) => invoke<void>("set_discard_short_sessions", { enabled });
export const getHotkeys = () => invoke<HotkeyStatus[]>("get_hotkeys");
export const setHotkeys = (hotkeys: Hotkeys) => invoke<HotkeyStatus[]>("set_hotkeys", { hotkeys });
