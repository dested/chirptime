/**
 * What the next recording will use: the saved options, the monitors and
 * devices it can pick from, the folder and its free space, and FFmpeg. One
 * hook for the widget's options, Settings and first run, so all three read and
 * write the same way.
 *
 * The probes shell out to FFmpeg and DXGI, so they run once on mount and again
 * only when asked (`reload*`), never on a poll.
 */
import { useCallback, useEffect, useState } from "react";
import {
  getEnvironment,
  getFfmpegSetup,
  getSettings,
  listDevices,
  listDisplayOutputs,
  setSessionOptions,
} from "../hooks/useApi";
import type { DisplayOutputInfo, DshowDevices, EnvironmentInfo, FfmpegSetup, SessionOptions, Settings } from "../types";
import { BYTES_PER_HOUR, DEFAULT_OPTIONS, readOptions, readPrefs, type RecorderPrefs } from "./api";

/** One monitor the recorder can capture, in virtual-desktop pixels. */
export interface DeskScreen {
  /** the N in `screen-N` */
  index: number;
  x: number;
  y: number;
  w: number;
  h: number;
  capturable: boolean;
  name: string;
}

export function deskScreens(outputs: DisplayOutputInfo[]): DeskScreen[] {
  const out: DeskScreen[] = [];
  for (const o of outputs) {
    if (!o.attached || o.screen_index === null) continue;
    out.push({
      index: o.screen_index,
      x: o.x,
      y: o.y,
      w: o.w,
      h: o.h,
      capturable: o.capturable,
      name: o.device_name.replace(/^\\\\\.\\/, ""),
    });
  }
  return out.sort((a, b) => a.index - b.index);
}

/** Whether screen `index` is in the next recording. An empty list means every capturable screen. */
export function screenOn(options: SessionOptions, screens: DeskScreen[], index: number): boolean {
  const s = screens.find((x) => x.index === index);
  if (!s?.capturable || !options.record_screens) return false;
  return options.screens.length === 0 || options.screens.includes(index);
}

/** Leave a screen out, or put it back. "Every screen" is stored as the empty list. */
export function toggleScreen(options: SessionOptions, screens: DeskScreen[], index: number): SessionOptions {
  const all = screens.filter((s) => s.capturable).map((s) => s.index);
  const current = !options.record_screens ? [] : options.screens.length === 0 ? all : options.screens;
  const next = current.includes(index) ? current.filter((i) => i !== index) : [...current, index].sort((a, b) => a - b);
  if (next.length === 0) return { ...options, record_screens: false, screens: [] };
  return { ...options, record_screens: true, screens: next.length === all.length ? [] : next };
}

const BLUETOOTH_MARKERS = ["bluetooth", "hands-free", "handsfree", "headset", "airpods", "buds"];

/** A Bluetooth headset mic forces the headset into hands-free mode while recording. */
export function looksBluetooth(name: string): boolean {
  const n = name.toLowerCase();
  return BLUETOOTH_MARKERS.some((m) => n.includes(m));
}

/** `812 GB free · about 125 hours` */
export function freeLine(env: EnvironmentInfo | null, fmt: (n: number) => string): string | null {
  if (!env) return null;
  if (env.disk_free_bytes === null) return "Free space unknown";
  const hours = Math.round(env.disk_free_bytes / BYTES_PER_HOUR);
  return `${fmt(env.disk_free_bytes)} free · about ${hours} ${hours === 1 ? "hour" : "hours"}`;
}

/** `Skips footage after 1 min with no keyboard or mouse` */
export function idleLine(options: SessionOptions): string {
  const s = options.drop_idle_after_seconds;
  if (s <= 0) return "Keeps everything, even while you are away";
  const m = Math.round(s / 60);
  return `Skips footage after ${m} min with no keyboard or mouse`;
}

export const IDLE_CHOICES: { seconds: number; label: string }[] = [
  { seconds: 0, label: "Never skip" },
  { seconds: 60, label: "1 minute idle" },
  { seconds: 120, label: "2 minutes idle" },
  { seconds: 300, label: "5 minutes idle" },
];

export const FPS_CHOICES = [10, 15, 24, 30, 60];

export interface RecordingSetup {
  settings: Settings | null;
  options: SessionOptions;
  prefs: RecorderPrefs | null;
  outputs: DisplayOutputInfo[] | null;
  screens: DeskScreen[];
  devices: DshowDevices | null;
  env: EnvironmentInfo | null;
  ffmpeg: FfmpegSetup | null;
  error: string | null;
  setOptions: (next: SessionOptions) => void;
  reloadSettings: () => void;
  reloadEnv: () => void;
  reloadFfmpeg: () => void;
  reloadOutputs: () => void;
}

export function useRecordingSetup({ probeDevices = true }: { probeDevices?: boolean } = {}): RecordingSetup {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [options, setOptionsState] = useState<SessionOptions>(DEFAULT_OPTIONS);
  const [outputs, setOutputs] = useState<DisplayOutputInfo[] | null>(null);
  const [devices, setDevices] = useState<DshowDevices | null>(null);
  const [env, setEnv] = useState<EnvironmentInfo | null>(null);
  const [ffmpeg, setFfmpeg] = useState<FfmpegSetup | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reloadSettings = useCallback(() => {
    getSettings()
      .then((s) => {
        setSettings(s);
        setOptionsState(readOptions(s));
      })
      .catch((e) => setError(String(e)));
  }, []);
  const reloadEnv = useCallback(() => {
    getEnvironment().then(setEnv).catch(() => setEnv(null));
  }, []);
  const reloadFfmpeg = useCallback(() => {
    getFfmpegSetup().then(setFfmpeg).catch(() => setFfmpeg(null));
  }, []);
  const reloadOutputs = useCallback(() => {
    listDisplayOutputs()
      .then(setOutputs)
      .catch((e) => {
        setOutputs([]);
        setError(String(e));
      });
  }, []);

  useEffect(() => {
    reloadSettings();
    reloadEnv();
    reloadFfmpeg();
    reloadOutputs();
    if (probeDevices) {
      listDevices()
        .then(setDevices)
        .catch(() => setDevices({ video: [], audio: [] }));
    }
  }, [reloadSettings, reloadEnv, reloadFfmpeg, reloadOutputs, probeDevices]);

  const setOptions = useCallback((next: SessionOptions) => {
    setOptionsState(next);
    setSessionOptions(next).catch((e) => setError(String(e)));
  }, []);

  return {
    settings,
    options,
    prefs: settings ? readPrefs(settings) : null,
    outputs,
    screens: deskScreens(outputs ?? []),
    devices,
    env,
    ffmpeg,
    error,
    setOptions,
    reloadSettings,
    reloadEnv,
    reloadFfmpeg,
    reloadOutputs,
  };
}
