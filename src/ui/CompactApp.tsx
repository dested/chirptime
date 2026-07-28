import React, { useState, useEffect, useCallback } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { listen } from "@tauri-apps/api/event";
import {
  startCapture,
  stopCapture,
  runSingleCapture,
  getCaptureStatus,
  fetchDates,
  fetchDateEntries,
  deleteEntry,
  openViewer,
  startSession,
  stopSession,
  getSessionStatus,
  addMarker,
  listDevices,
  listDisplayOutputs,
  setOutputDir,
  getSettings,
  setSessionOptions,
  setOnboarded,
  getEnvironment,
  pickOutputDir,
  openFolder,
  getFfmpegSetup,
  installFfmpeg,
  pickFfmpegDir,
} from "./hooks/useApi";
import type {
  CaptureStatus,
  CaptureEntry,
  DisplayOutputInfo,
  EnvironmentInfo,
  FfmpegProgress,
  FfmpegSetup,
  SessionOptions,
  SessionStatus,
  TrackSummary,
} from "./types";
import { fmtBytes, fmtClock, fmtElapsed } from "./session/format";

const appWindow = getCurrentWindow();

type Mode = "clips" | "session";

/** Mirrors `recorder::SessionOptions::default()`. */
const DEFAULT_OPTIONS: SessionOptions = {
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
};

const IDLE_SESSION: SessionStatus = {
  recording: false,
  session_id: null,
  session_dir: null,
  started_epoch_ms: null,
  elapsed_ms: 0,
  segment_seconds: 0,
  tracks: [],
  total_bytes: 0,
  segment_count: 0,
  warnings: [],
  error: null,
  disk_free_bytes: null,
  min_free_bytes: 1024 * 1024 * 1024,
};

/**
 * Mirrors `recorder::is_bluetooth_audio_device`. Used only to *label* a device in
 * the picker — the backend still makes the actual selection, and picking one here
 * is an explicit choice that it will honour.
 */
const BLUETOOTH_MARKERS = [
  "bluetooth", "hands-free", "handsfree", "hands free", "hfp",
  "headset", "airpods", "earbuds", "earphone", "galaxy buds",
];
function looksBluetooth(name: string): boolean {
  const n = name.toLowerCase();
  return BLUETOOTH_MARKERS.some((m) => n.includes(m));
}

/* ---------------------------------------------------------------- pieces -- */

function Eyebrow({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[11px] font-semibold text-slate-400 uppercase tracking-wide">{children}</div>
  );
}

const SECONDARY_BTN =
  "flex items-center justify-center gap-2 py-2.5 rounded-[10px] text-[13px] font-medium cursor-pointer border border-slate-200 bg-white text-slate-600 hover:bg-slate-50 hover:border-slate-300 hover:text-slate-900 transition-all disabled:opacity-50 disabled:cursor-not-allowed";

const FIELD =
  "w-full px-2 py-1.5 rounded-lg border border-slate-200 bg-white text-[12px] text-slate-900 cursor-pointer hover:border-slate-300 focus:outline-none focus:border-sky-500 transition-all disabled:opacity-50 disabled:cursor-not-allowed";

function Stat({ value, label, tone }: { value: string; label: string; tone?: string }) {
  return (
    <div className="flex flex-col items-center gap-0.5 min-w-0">
      <span className={`text-[15px] font-bold tabular-nums font-mono ${tone ?? "text-slate-900"}`}>
        {value}
      </span>
      <span className="text-[10px] font-medium text-slate-400 uppercase tracking-wide">{label}</span>
    </div>
  );
}

/**
 * Advisory line. Amber means "this needs your attention but nothing is broken" —
 * the Bluetooth-mic refusal is the case this exists for. Red is reserved for a
 * track or session that actually failed.
 */
function Notice({ tone, children }: { tone: "warn" | "error"; children: React.ReactNode }) {
  const cls =
    tone === "error"
      ? "bg-red-50 text-red-700 border-red-100"
      : "bg-amber-50 text-amber-800 border-amber-100";
  return (
    <div className={`flex gap-2 px-2.5 py-2 rounded-lg border text-[11px] leading-snug ${cls}`}>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className="shrink-0 mt-px">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 8v5M12 16v.5" />
      </svg>
      <span className="min-w-0">{children}</span>
    </div>
  );
}

/**
 * Midpoint of the 5–8 GB/hour a full session actually writes. Only ever used to
 * turn free space into a number of hours, always hedged with "about" — it is an
 * order-of-magnitude answer to "how long can I record", not a promise.
 */
const GB_PER_HOUR = 6.5;

function fmtRecordingHours(freeBytes: number): string {
  const hours = freeBytes / (GB_PER_HOUR * 1e9);
  if (hours < 1) return "under an hour of recording";
  return `about ${Math.round(hours)} hours of recording`;
}

/** One line of "here is what you are signing up for" on the first-run panel. */
function Fact({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-2.5">
      <span className="shrink-0 mt-0.5 text-slate-300">{icon}</span>
      <div className="min-w-0">
        <div className="text-[12px] font-semibold text-slate-900 leading-snug">{title}</div>
        <div className="text-[11px] text-slate-400 leading-snug">{children}</div>
      </div>
    </div>
  );
}

/** One requirement, muted when satisfied and amber when it is not. */
function Requirement({ ok, label, value }: { ok: boolean; label: string; value: string }) {
  return (
    <div className="flex items-center gap-2 text-[11px]">
      <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${ok ? "bg-slate-300" : "bg-amber-500"}`} />
      <span className="text-slate-600 shrink-0">{label}</span>
      <span
        className={`ml-auto min-w-0 truncate font-mono ${ok ? "text-slate-400" : "text-amber-700"}`}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

/**
 * The output folder, its free space, and the two things anyone ever wants to do
 * with it. Used by the first-run panel and by settings, because "where are my
 * recordings" is a question that does not stop being asked after day one.
 */
function OutputFolder({
  env,
  onChanged,
  disabled = false,
}: {
  env: EnvironmentInfo | null;
  onChanged: () => void;
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const choose = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      // null means the user cancelled the dialog — nothing to do.
      if (await pickOutputDir()) onChanged();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }, [onChanged]);

  const open = useCallback(async () => {
    setError(null);
    try {
      await openFolder();
    } catch (e) {
      setError(String(e));
    }
  }, []);

  return (
    <div className="flex flex-col gap-1.5">
      <div className="px-2.5 py-2 rounded-[10px] border border-slate-200 bg-white">
        <div className="text-[11px] font-mono text-slate-900 break-all leading-snug">
          {env ? env.output_dir : "Loading..."}
        </div>
        {env && (
          <div
            className={`mt-1 text-[10px] tabular-nums font-mono ${
              // Under two hours of recording is worth reading twice — advisory,
              // not an error: it is still a perfectly valid folder.
              env.disk_free_bytes !== null && env.disk_free_bytes < 2 * GB_PER_HOUR * 1e9
                ? "text-amber-700"
                : "text-slate-400"
            }`}
          >
            {env.disk_free_bytes === null
              ? "Free space unknown"
              : `${fmtBytes(env.disk_free_bytes)} free · ${fmtRecordingHours(env.disk_free_bytes)}`}
          </div>
        )}
      </div>
      <div className="flex gap-2">
        <button
          className={`${SECONDARY_BTN} flex-1 py-1.5 text-[12px]`}
          onClick={() => void choose()}
          disabled={busy || disabled}
        >
          Choose Folder
        </button>
        <button
          className={`${SECONDARY_BTN} px-3 py-1.5 text-[12px]`}
          onClick={() => void open()}
          title="Open in Explorer"
        >
          Open
        </button>
      </div>
      {error && <Notice tone="error">{error}</Notice>}
    </div>
  );
}

/**
 * First run. The app is about to write hours of somebody's life to disk at
 * several GB an hour and never delete any of it, so where that lands — and what
 * it costs — is stated before the first recording, not discovered afterwards.
 *
 * Inline, never a modal: the window is 320px wide (see ui.md).
 */
function Onboarding({ onDone }: { onDone: () => void }) {
  const [env, setEnv] = useState<EnvironmentInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [legacyDismissed, setLegacyDismissed] = useState(false);
  const [busy, setBusy] = useState(false);

  const [ffmpeg, setFfmpeg] = useState<FfmpegSetup | null>(null);

  const load = useCallback(() => {
    getEnvironment()
      .then(setEnv)
      .catch((e) => setError(String(e)));
    // Kept separate from `env` so installing FFmpeg does not also re-probe disk
    // space and the legacy output folder, and so a failure in one does not
    // blank the other.
    getFfmpegSetup().then(setFfmpeg).catch(() => setFfmpeg(null));
  }, []);
  useEffect(load, [load]);

  const adoptLegacy = useCallback(
    async (path: string) => {
      setError(null);
      try {
        await setOutputDir(path);
        load();
      } catch (e) {
        setError(String(e));
      }
    },
    [load],
  );

  const finish = useCallback(async () => {
    setBusy(true);
    try {
      // Only dismiss once it is actually written: if settings cannot be saved,
      // the panel would come back next launch with no explanation.
      await setOnboarded(true);
      onDone();
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }, [onDone]);

  const legacy = env?.legacy_output ?? null;

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="flex-1 min-h-0 overflow-y-auto px-4 pt-3 pb-3">
        <Eyebrow>First run</Eyebrow>
        <h1 className="mt-1 text-[15px] font-bold tracking-tight text-slate-900">
          Where your recordings go
        </h1>
        <p className="mt-1 text-[12px] text-slate-500 leading-snug">
          Screenshots, clips and sessions are written to this folder on this machine.
          Nothing leaves it.
        </p>

        <div className="mt-3">
          <Eyebrow>Saving to</Eyebrow>
          <div className="mt-1.5">
            <OutputFolder env={env} onChanged={load} />
          </div>
        </div>

        {error && (
          <div className="mt-2">
            <Notice tone="error">{error}</Notice>
          </div>
        )}

        {legacy && !legacyDismissed && (
          <div className="mt-2">
            <Notice tone="warn">
              <span className="block">
                Earlier recordings are still in{" "}
                <span className="font-mono break-all">{legacy.path}</span> —{" "}
                {legacy.day_count} {legacy.day_count === 1 ? "day" : "days"} and{" "}
                {legacy.session_count}{" "}
                {legacy.session_count === 1 ? "session" : "sessions"}. ChirpTime is not
                reading that folder anymore. Nothing there has been moved or deleted.
              </span>
              <span className="flex gap-2 mt-1.5">
                <button
                  className={`${SECONDARY_BTN} flex-1 py-1 text-[11px]`}
                  onClick={() => void adoptLegacy(legacy.path)}
                >
                  Use This Folder
                </button>
                <button
                  className={`${SECONDARY_BTN} px-2 py-1 text-[11px]`}
                  onClick={() => setLegacyDismissed(true)}
                >
                  Ignore
                </button>
              </span>
            </Notice>
          </div>
        )}

        <div className="flex flex-col gap-2.5 mt-4">
          <Fact
            icon={
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <ellipse cx="12" cy="6" rx="8" ry="3" />
                <path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6" />
                <path d="M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3" />
              </svg>
            }
            title="5 to 8 GB per hour"
          >
            A continuous session records every screen, the webcam and audio at once. Check
            the free space above before a long day.
          </Fact>
          <Fact
            icon={
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <rect x="3" y="4" width="18" height="4" rx="1" />
                <path d="M5 8v11a1 1 0 001 1h12a1 1 0 001-1V8" />
                <path d="M10 12h4" />
              </svg>
            }
            title="Nothing is deleted"
          >
            ChirpTime never removes a recording, however old. The folder grows until you
            clear it out yourself.
          </Fact>
          <Fact
            icon={
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <rect x="3" y="4" width="18" height="12" rx="2" />
                <path d="M8 20h8M12 16v4" />
              </svg>
            }
            title="Windows only"
          >
            Capture runs on FFmpeg. An NVIDIA GPU gets the fast encoder path; without one
            recording still works but costs more CPU.
          </Fact>
        </div>

        <div className="flex flex-col gap-1.5 mt-4">
          <Eyebrow>This machine</Eyebrow>
          {env === null ? (
            <div className="text-[11px] text-slate-400">Loading...</div>
          ) : (
            <>
              <Requirement
                ok={ffmpeg?.ffmpeg_path != null}
                label="FFmpeg"
                value={ffmpeg?.ffmpeg_path ?? "Not found"}
              />
              <Requirement
                ok={env.nvenc}
                label="NVIDIA encoder"
                value={env.nvenc ? "Available" : "Not available"}
              />
            </>
          )}
        </div>

        {/* The one thing on this panel that stops the app working at all, so it
            is offered here rather than discovered later on a failed capture. */}
        {ffmpeg && ffmpeg.ffmpeg_path === null && (
          <div className="mt-2">
            <Notice tone="error">
              <span className="block font-semibold mb-1">FFmpeg is missing</span>
              <FfmpegSetupCard setup={ffmpeg} onResolved={load} />
            </Notice>
          </div>
        )}

        {env && (
          <div className="mt-3 text-[10px] text-slate-300 font-mono break-all leading-snug">
            Settings: {env.settings_path}
          </div>
        )}
      </div>

      <div className="px-4 py-2.5 pb-3.5 border-t border-slate-100">
        <button
          className="flex items-center justify-center gap-2.5 w-full py-3 rounded-xl text-sm font-semibold tracking-tight cursor-pointer border-none bg-sky-500 text-white shadow-[0_1px_3px_rgba(14,165,233,0.3),0_4px_12px_rgba(14,165,233,0.15)] hover:bg-sky-600 hover:shadow-[0_2px_6px_rgba(14,165,233,0.4),0_6px_16px_rgba(14,165,233,0.2)] hover:-translate-y-px transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          onClick={() => void finish()}
          disabled={busy}
        >
          <span>Continue</span>
        </button>
      </div>
    </div>
  );
}

/** Where someone who would rather install it themselves is sent. */
const FFMPEG_DOWNLOAD_URL = "https://www.gyan.dev/ffmpeg/builds/";

const PHASE_LABEL: Record<FfmpegProgress["phase"], string> = {
  downloading: "Downloading FFmpeg",
  verifying: "Checking the download",
  extracting: "Unpacking",
  checking: "Making sure it runs",
  done: "Done",
};

/**
 * The missing-FFmpeg recovery, shared by the first-run panel and the persistent
 * banner so there is exactly one flow rather than two that drift apart.
 *
 * ChirpTime records through FFmpeg but does not ship it, so someone who
 * installed the .msi has never been told they need a second program. This says
 * what is missing, offers to fetch it — **stating the size first, and only on a
 * button press** — and, for the person whose network blocks the download, takes
 * a folder they already have.
 *
 * Progress is not optional: a silent 105 MB transfer is indistinguishable from
 * a hang.
 */
function FfmpegSetupCard({
  setup,
  onResolved,
  // The banner sits above the panes in a 540px-tall window, so it drops the
  // install path and the long explanation. The first-run panel scrolls and has
  // room to say the whole thing.
  compact = false,
}: {
  setup: FfmpegSetup;
  onResolved: () => void;
  compact?: boolean;
}) {
  const [progress, setProgress] = useState<FfmpegProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The backend owns the install; this window only reports it. Subscribing for
  // the life of the card (not just while installing) means a panel reopened
  // mid-install rejoins the progress instead of showing a dead bar.
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let live = true;
    listen<FfmpegProgress>("ffmpeg-install-progress", (e) => setProgress(e.payload))
      .then((un) => {
        if (live) dispose = un;
        else un();
      })
      .catch(() => {});
    return () => {
      live = false;
      dispose?.();
    };
  }, []);

  const download = useCallback(async () => {
    setBusy(true);
    setError(null);
    setProgress({ phase: "downloading", downloaded_bytes: 0, total_bytes: setup.download_bytes });
    try {
      await installFfmpeg();
      onResolved();
    } catch (e) {
      setError(String(e));
      setProgress(null);
    } finally {
      setBusy(false);
    }
  }, [setup.download_bytes, onResolved]);

  const locate = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const picked = await pickFfmpegDir();
      // null is a cancelled picker, which is not a failure and says nothing.
      if (picked) onResolved();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [onResolved]);

  const recheck = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      onResolved();
    } finally {
      setBusy(false);
    }
  }, [onResolved]);

  if (progress && busy) {
    const pct =
      progress.total_bytes > 0
        ? Math.min(100, Math.round((progress.downloaded_bytes / progress.total_bytes) * 100))
        : 0;
    return (
      // Spans, not divs: this renders inside `Notice`, whose body is a <span>.
      <span className="flex flex-col gap-1.5">
        <span className="flex items-baseline justify-between gap-2">
          <span className="text-[11px] font-semibold text-slate-900">
            {PHASE_LABEL[progress.phase]}
          </span>
          {progress.phase === "downloading" && (
            <span className="text-[10px] text-slate-400 tabular-nums font-mono">
              {fmtBytes(progress.downloaded_bytes)} / {fmtBytes(progress.total_bytes)}
            </span>
          )}
        </span>
        <span className="block h-1.5 w-full rounded-full bg-slate-100 overflow-hidden">
          <span
            className={`block h-full rounded-full bg-sky-500 transition-[width] duration-200 ${
              progress.phase === "downloading" ? "" : "animate-pulse"
            }`}
            style={{ width: progress.phase === "downloading" ? `${pct}%` : "100%" }}
          />
        </span>
        <span className="text-[10px] text-slate-400 leading-snug">
          Leave this open. Nothing is installed until the download has been checked
          against its expected checksum.
        </span>
      </span>
    );
  }

  return (
    <span className="flex flex-col gap-1.5">
      <span className="text-[11px] leading-snug text-slate-600">
        {compact ? (
          <>
            Nothing can be captured without it. ChirpTime can fetch it for you —{" "}
            {fmtBytes(setup.download_bytes)}, checksum-verified, no admin needed.
          </>
        ) : (
          <>
            ChirpTime records through FFmpeg and cannot capture anything without it.
            ChirpTime can fetch it for you — about {fmtBytes(setup.download_bytes)},
            verified against a known checksum and installed just for you, no admin
            needed.
          </>
        )}
      </span>
      {!compact && (
        <span className="text-[10px] text-slate-400 font-mono break-all leading-snug">
          {setup.install_dir}
        </span>
      )}

      {error && (
        <span className="block text-[10px] leading-snug text-red-600 break-words">{error}</span>
      )}

      <span className="flex flex-col gap-1.5 mt-0.5">
        <button
          className="w-full py-1.5 rounded-lg text-[11px] font-semibold cursor-pointer border-none bg-sky-500 text-white hover:bg-sky-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          onClick={() => void download()}
          disabled={busy}
        >
          {error ? "Try Again" : `Download FFmpeg (${fmtBytes(setup.download_bytes)})`}
        </button>
        <span className="flex gap-1.5">
          <button
            className={`${SECONDARY_BTN} flex-1 py-1 text-[11px]`}
            onClick={() => void locate()}
            disabled={busy}
            title="Choose a folder containing ffmpeg.exe and ffprobe.exe"
          >
            I Already Have It
          </button>
          <button
            className={`${SECONDARY_BTN} px-2 py-1 text-[11px]`}
            onClick={() => void recheck()}
            disabled={busy}
          >
            Re-check
          </button>
        </span>
      </span>

      <button
        className="self-start text-[10px] text-slate-400 hover:text-slate-600 bg-transparent border-none p-0 cursor-pointer underline"
        onClick={() => void openUrl(FFMPEG_DOWNLOAD_URL).catch(() => {})}
      >
        Or install it yourself
      </button>
    </span>
  );
}

/**
 * The persistent form of the above, above the mode switch so it applies to both
 * panes. Not dismissible — nothing records until it is resolved.
 */
function FfmpegBanner({ setup, onResolved }: { setup: FfmpegSetup; onResolved: () => void }) {
  return (
    <div className="px-4 pt-3">
      <Notice tone="error">
        <span className="block font-semibold mb-1">FFmpeg is not installed</span>
        <FfmpegSetupCard setup={setup} onResolved={onResolved} compact />
      </Notice>
    </div>
  );
}

const KIND_BADGE: Record<string, string> = {
  screen: "bg-sky-50 text-sky-600",
  webcam: "bg-purple-50 text-purple-500",
  mic: "bg-slate-100 text-slate-500",
  "system-audio": "bg-slate-100 text-slate-500",
};

const KIND_LABEL: Record<string, string> = {
  screen: "Screen",
  webcam: "Cam",
  mic: "Mic",
  "system-audio": "Sys",
};

function TrackRow({ track }: { track: TrackSummary }) {
  const dot =
    track.status === "recording"
      ? "bg-sky-500 shadow-[0_0_0_3px_rgba(14,165,233,0.15)] dot-active"
      : track.status === "restarting"
        ? "bg-amber-500"
        : track.status === "failed"
          ? "bg-red-500"
          : track.status === "starting"
            ? "bg-slate-300"
            : "bg-slate-400";

  return (
    <div className="flex flex-col gap-1 px-2 py-1.5 rounded-lg hover:bg-slate-50 transition-colors">
      <div className="flex items-center gap-2">
        <span className={`w-2 h-2 rounded-full shrink-0 ${dot}`} />
        <span className="text-[12px] font-medium text-slate-900 truncate" title={track.label}>
          {track.label}
        </span>
        <span
          className={`shrink-0 text-[10px] font-semibold px-1.5 py-0.5 rounded-md ${
            KIND_BADGE[track.kind] ?? "bg-slate-100 text-slate-500"
          }`}
        >
          {KIND_LABEL[track.kind] ?? track.kind}
        </span>
        <span className="ml-auto shrink-0 text-[11px] text-slate-400 tabular-nums font-mono">
          {track.segment_count} · {fmtBytes(track.bytes)}
        </span>
      </div>
      {(track.restarts > 0 || track.gap_count > 0) && (
        <div className="pl-4 text-[10px] text-slate-400 tabular-nums font-mono">
          {track.restarts > 0 && <span>{track.restarts} restarts</span>}
          {track.restarts > 0 && track.gap_count > 0 && <span> · </span>}
          {track.gap_count > 0 && <span>{track.gap_count} gaps</span>}
        </div>
      )}
      {track.error && (
        <div className="pl-4 text-[10px] leading-snug text-red-600">{track.error}</div>
      )}
    </div>
  );
}

/* ------------------------------------------------------------- settings --- */

function SessionSettings({
  options,
  onChange,
  disabled,
}: {
  options: SessionOptions;
  onChange: (next: SessionOptions) => void;
  disabled: boolean;
}) {
  const [outputs, setOutputs] = useState<DisplayOutputInfo[] | null>(null);
  const [devices, setDevices] = useState<{ video: string[]; audio: string[] } | null>(null);
  const [env, setEnv] = useState<EnvironmentInfo | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);

  const loadEnv = useCallback(() => {
    getEnvironment().then(setEnv).catch(() => {});
  }, []);

  // All of these shell out to ffmpeg, so they run once when the panel opens and
  // never from a render path.
  useEffect(() => {
    let live = true;
    listDisplayOutputs()
      .then((o) => live && setOutputs(o))
      .catch((e) => live && (setOutputs([]), setProbeError(String(e))));
    listDevices()
      .then((d) => live && setDevices(d))
      .catch(() => live && setDevices({ video: [], audio: [] }));
    loadEnv();
    return () => {
      live = false;
    };
  }, [loadEnv]);

  const capturable = (outputs ?? []).filter((o) => o.attached && o.screen_index !== null);
  const allIndices = capturable.filter((o) => o.capturable).map((o) => o.screen_index as number);
  const isSelected = (idx: number) =>
    options.screens.length === 0 ? true : options.screens.includes(idx);

  const toggleScreen = (idx: number) => {
    const current = options.screens.length === 0 ? allIndices : options.screens;
    const next = current.includes(idx)
      ? current.filter((i) => i !== idx)
      : [...current, idx].sort((a, b) => a - b);
    // An explicit "everything" is stored as the empty list, which is also what a
    // fresh install means by it.
    onChange({
      ...options,
      screens: next.length === allIndices.length ? [] : next,
    });
  };

  return (
    <div className="flex flex-col gap-3 px-2.5 py-3 mt-2 rounded-[10px] border border-slate-200 bg-slate-50">
      {disabled && (
        <div className="text-[10px] text-slate-400 leading-snug">
          Settings are read while the session starts. Stop recording to change them.
        </div>
      )}

      {/* Monitors */}
      <div className="flex flex-col gap-1.5">
        <Eyebrow>Monitors</Eyebrow>
        {probeError && <Notice tone="error">{probeError}</Notice>}
        {outputs === null ? (
          <div className="text-[11px] text-slate-400">Loading...</div>
        ) : capturable.length === 0 ? (
          <div className="text-[11px] text-slate-400">No displays detected</div>
        ) : (
          capturable.map((o) => {
            const idx = o.screen_index as number;
            const usable = o.capturable && options.record_screens && !disabled;
            return (
              <label
                key={`${o.adapter}-${o.output_idx}`}
                className={`flex items-center gap-2 text-[12px] ${
                  usable ? "cursor-pointer text-slate-700" : "cursor-not-allowed text-slate-400"
                }`}
              >
                <input
                  type="checkbox"
                  className="accent-sky-500"
                  checked={o.capturable && options.record_screens && isSelected(idx)}
                  disabled={!usable}
                  onChange={() => toggleScreen(idx)}
                />
                <span className="truncate">
                  {o.device_name.replace(/^\\\\\.\\/, "")}
                </span>
                <span className="ml-auto shrink-0 text-[10px] text-slate-400 tabular-nums font-mono">
                  {o.w}x{o.h}
                </span>
              </label>
            );
          })
        )}
        {outputs?.some((o) => o.attached && !o.capturable) && (
          <Notice tone="warn">
            A monitor is on a secondary GPU with no h264_qsv encoder. That adapter has to
            encode its own capture, so it cannot be recorded.
          </Notice>
        )}
      </div>

      {/* Sources */}
      <div className="flex flex-col gap-1.5">
        <Eyebrow>Sources</Eyebrow>

        <label className="flex items-center gap-2 text-[12px] text-slate-700 cursor-pointer">
          <input
            type="checkbox"
            className="accent-sky-500"
            checked={options.record_screens}
            disabled={disabled}
            onChange={(e) => onChange({ ...options, record_screens: e.target.checked })}
          />
          <span>Screens</span>
        </label>

        <label className="flex items-center gap-2 text-[12px] text-slate-700 cursor-pointer">
          <input
            type="checkbox"
            className="accent-sky-500"
            checked={options.record_webcam}
            disabled={disabled}
            onChange={(e) => onChange({ ...options, record_webcam: e.target.checked })}
          />
          <span>Webcam</span>
        </label>
        {options.record_webcam && (
          <select
            className={FIELD}
            disabled={disabled}
            value={options.webcam_device ?? ""}
            onChange={(e) => onChange({ ...options, webcam_device: e.target.value || null })}
          >
            <option value="">Auto (first camera)</option>
            {(devices?.video ?? []).map((d) => (
              <option key={d} value={d}>{d}</option>
            ))}
          </select>
        )}

        <label className="flex items-center gap-2 text-[12px] text-slate-700 cursor-pointer">
          <input
            type="checkbox"
            className="accent-sky-500"
            checked={options.record_mic}
            disabled={disabled}
            onChange={(e) => onChange({ ...options, record_mic: e.target.checked })}
          />
          <span>Microphone</span>
        </label>
        {options.record_mic && (
          <>
            <select
              className={FIELD}
              disabled={disabled}
              value={options.mic_device ?? ""}
              onChange={(e) => onChange({ ...options, mic_device: e.target.value || null })}
            >
              <option value="">Auto (never Bluetooth)</option>
              {(devices?.audio ?? []).map((d) => (
                <option key={d} value={d}>
                  {looksBluetooth(d) ? `${d} — Bluetooth` : d}
                </option>
              ))}
            </select>
            {options.mic_device && looksBluetooth(options.mic_device) && (
              <Notice tone="warn">
                Recording this microphone forces the headset into hands-free mode. Everything
                you listen to drops to 16kHz mono until the session ends.
              </Notice>
            )}
          </>
        )}

        <label className="flex items-center gap-2 text-[12px] text-slate-700 cursor-pointer">
          <input
            type="checkbox"
            className="accent-sky-500"
            checked={options.record_system_audio}
            disabled={disabled}
            onChange={(e) => onChange({ ...options, record_system_audio: e.target.checked })}
          />
          <span>System audio</span>
        </label>
      </div>

      {/* Quality */}
      <div className="flex flex-col gap-1.5">
        <Eyebrow>Quality</Eyebrow>
        <div className="flex items-center gap-2">
          <span className="text-[12px] text-slate-600 w-16 shrink-0">Frame rate</span>
          <select
            className={FIELD}
            disabled={disabled}
            value={options.screen_fps}
            onChange={(e) => onChange({ ...options, screen_fps: Number(e.target.value) })}
          >
            {[10, 15, 24, 30, 60].map((f) => (
              <option key={f} value={f}>{f} fps</option>
            ))}
          </select>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-[12px] text-slate-600 w-16 shrink-0">Segments</span>
          <select
            className={FIELD}
            disabled={disabled}
            value={options.segment_seconds}
            onChange={(e) => onChange({ ...options, segment_seconds: Number(e.target.value) })}
          >
            {[15, 30, 60, 120, 300].map((s) => (
              <option key={s} value={s}>{s < 60 ? `${s} sec` : `${s / 60} min`}</option>
            ))}
          </select>
        </div>
      </div>

      {/* Output */}
      <div className="flex flex-col gap-1.5">
        <Eyebrow>Output folder</Eyebrow>
        <OutputFolder env={env} onChanged={loadEnv} disabled={disabled} />
        <div className="text-[10px] text-slate-400 leading-snug">
          Clips and sessions are written here and are never deleted automatically.
        </div>
        {env?.legacy_output && (
          <Notice tone="warn">
            Older recordings are still in{" "}
            <span className="font-mono break-all">{env.legacy_output.path}</span>. This
            app no longer reads that folder; nothing there has been moved.
          </Notice>
        )}
      </div>
    </div>
  );
}

/* -------------------------------------------------------------- session --- */

export function SessionPane({
  status,
  onChanged,
}: {
  status: SessionStatus;
  onChanged: (s: SessionStatus) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [options, setOptions] = useState<SessionOptions>(DEFAULT_OPTIONS);
  const [showSettings, setShowSettings] = useState(false);
  const [markerLabel, setMarkerLabel] = useState("");
  const [lastMarker, setLastMarker] = useState<string | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  // Survives the stop: `get_session_status` goes fully idle the moment the
  // recorder is dropped, and the session you just finished is the one you are
  // most likely to want to open.
  const [lastSessionId, setLastSessionId] = useState<string | null>(null);

  useEffect(() => {
    if (status.session_id) setLastSessionId(status.session_id);
  }, [status.session_id]);

  // Options survive a restart. The stored blob is merged over the defaults
  // rather than trusted whole, so a file written by an older build cannot leave
  // a field undefined on the way to `start_session`.
  useEffect(() => {
    let live = true;
    getSettings()
      .then((s) => {
        if (live && s.session) setOptions({ ...DEFAULT_OPTIONS, ...s.session });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const updateOptions = useCallback((next: SessionOptions) => {
    setOptions(next);
    void setSessionOptions(next).catch(() => {});
  }, []);

  const linkSessionId = status.session_id ?? lastSessionId;

  // Elapsed ticks locally off `started_epoch_ms` so the readout is smooth
  // between the 2s status polls.
  useEffect(() => {
    if (!status.recording) return;
    const t = setInterval(() => setNowMs(Date.now()), 1000);
    return () => clearInterval(t);
  }, [status.recording]);

  const elapsedMs =
    status.recording && status.started_epoch_ms !== null
      ? Math.max(0, nowMs - status.started_epoch_ms)
      : status.elapsed_ms;

  const handleStart = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      onChanged(await startSession(options));
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }, [options, onChanged]);

  const handleStop = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      onChanged(await stopSession());
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  }, [onChanged]);

  const handleMarker = useCallback(async () => {
    const label = markerLabel.trim() || "Marker";
    try {
      const m = await addMarker(label);
      setMarkerLabel("");
      setLastMarker(`${label} at ${fmtClock(m.epoch_ms)}`);
    } catch (e) {
      setError(String(e));
    }
  }, [markerLabel]);

  const diskTone =
    status.disk_free_bytes === null
      ? "text-slate-400"
      : status.disk_free_bytes < status.min_free_bytes
        ? "text-red-600"
        : status.disk_free_bytes < status.min_free_bytes * 5
          ? "text-amber-600"
          : "text-slate-900";

  const failed = status.tracks.filter((t) => t.status === "failed");

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="flex-1 min-h-0 overflow-y-auto px-4 pt-3 pb-2">
        {/* Status chip */}
        <div
          className={`flex items-center gap-2 px-3.5 py-2 rounded-[10px] text-[13px] font-semibold ${
            status.recording ? "bg-red-50 text-red-600" : "bg-slate-50 text-slate-500"
          }`}
        >
          <span
            className={`w-2 h-2 rounded-full shrink-0 ${
              status.recording
                ? "bg-red-600 shadow-[0_0_0_3px_rgba(220,38,38,0.15)] dot-active"
                : "bg-slate-300"
            }`}
          />
          <span>{status.recording ? "Recording" : "Idle"}</span>
          {status.recording && (
            <span className="ml-auto tabular-nums font-mono text-[13px]">{fmtElapsed(elapsedMs)}</span>
          )}
        </div>

        {/* Primary action */}
        <div className="flex flex-col gap-2 pt-3">
          {status.recording ? (
            <button
              className="flex items-center justify-center gap-2.5 py-3.5 rounded-xl text-sm font-semibold tracking-tight cursor-pointer border-none bg-red-600 text-white shadow-[0_1px_3px_rgba(220,38,38,0.3),0_4px_12px_rgba(220,38,38,0.15)] hover:bg-red-700 hover:shadow-[0_2px_6px_rgba(220,38,38,0.4),0_6px_16px_rgba(220,38,38,0.2)] hover:-translate-y-px transition-all disabled:opacity-50 disabled:cursor-not-allowed"
              onClick={handleStop}
              disabled={busy}
            >
              <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
                <rect x="4" y="4" width="12" height="12" rx="2" />
              </svg>
              <span>Stop Session</span>
            </button>
          ) : (
            <button
              className="flex items-center justify-center gap-2.5 py-3.5 rounded-xl text-sm font-semibold tracking-tight cursor-pointer border-none bg-sky-500 text-white shadow-[0_1px_3px_rgba(14,165,233,0.3),0_4px_12px_rgba(14,165,233,0.15)] hover:bg-sky-600 hover:shadow-[0_2px_6px_rgba(14,165,233,0.4),0_6px_16px_rgba(14,165,233,0.2)] hover:-translate-y-px transition-all disabled:opacity-50 disabled:cursor-not-allowed"
              onClick={handleStart}
              disabled={busy}
            >
              <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
                <circle cx="10" cy="10" r="7" />
              </svg>
              <span>Start Session</span>
            </button>
          )}

          {/* Marker */}
          {status.recording && (
            <div className="flex flex-col gap-1">
              <div className="flex gap-2">
                <input
                  className={`${FIELD} cursor-text flex-1`}
                  placeholder="What just happened?"
                  value={markerLabel}
                  onChange={(e) => setMarkerLabel(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && void handleMarker()}
                />
                <button
                  className={`${SECONDARY_BTN} shrink-0 px-3 py-1.5`}
                  onClick={() => void handleMarker()}
                  title="Flag this moment"
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="#F59E0B" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M5 21V4h13l-2.5 4L18 12H5" />
                  </svg>
                  <span>Flag</span>
                </button>
              </div>
              {lastMarker && (
                <span className="text-[10px] text-slate-400 tabular-nums font-mono">{lastMarker}</span>
              )}
            </div>
          )}
        </div>

        {/* Stats */}
        <div className="flex items-center justify-center py-2.5 mt-3 border-t border-b border-slate-100 gap-3">
          <Stat value={String(status.segment_count)} label="Segments" />
          <div className="w-px h-7 bg-slate-100" />
          <Stat value={fmtBytes(status.total_bytes)} label="Written" />
          <div className="w-px h-7 bg-slate-100" />
          <Stat
            value={status.disk_free_bytes === null ? "--" : fmtBytes(status.disk_free_bytes)}
            label="Disk free"
            tone={diskTone}
          />
        </div>

        {/* Errors and warnings */}
        <div className="flex flex-col gap-1.5 pt-2.5">
          {error && <Notice tone="error">{error}</Notice>}
          {status.error && <Notice tone="error">{status.error}</Notice>}
          {failed.map((t) => (
            <Notice key={t.id} tone="error">
              <span className="font-semibold">{t.label}</span> is not recording
              {t.error ? `: ${t.error}` : "."}
            </Notice>
          ))}
          {status.warnings.map((w) => (
            <Notice key={w} tone="warn">{w}</Notice>
          ))}
        </div>

        {/* Tracks */}
        {status.tracks.length > 0 && (
          <div className="flex flex-col pt-3">
            <div className="flex items-center justify-between mb-1">
              <Eyebrow>Tracks</Eyebrow>
              <span className="text-[11px] text-slate-300 font-mono">{status.session_id}</span>
            </div>
            {status.tracks.map((t) => (
              <TrackRow key={t.id} track={t} />
            ))}
          </div>
        )}

        {/* Settings */}
        <div className="pt-3">
          <button
            className="flex items-center gap-2 w-full py-1.5 bg-transparent border-none cursor-pointer text-slate-400 hover:text-slate-900 transition-all"
            onClick={() => setShowSettings((v) => !v)}
          >
            <Eyebrow>Settings</Eyebrow>
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              className={`ml-auto transition-transform ${showSettings ? "rotate-180" : ""}`}
            >
              <path d="M6 9l6 6 6-6" />
            </svg>
          </button>
          {showSettings && (
            <SessionSettings options={options} onChange={updateOptions} disabled={status.recording} />
          )}
        </div>
      </div>

      {/* Footer */}
      <div className="px-4 py-2.5 pb-3.5 border-t border-slate-100">
        <button
          className="flex items-center justify-center gap-2 w-full py-2.5 rounded-[10px] text-[13px] font-medium cursor-pointer border border-slate-200 bg-slate-50 text-slate-600 hover:bg-slate-100 hover:border-slate-300 hover:text-slate-900 transition-all"
          onClick={() =>
            void openViewer(linkSessionId ? { session: linkSessionId } : { tab: "sessions" })
          }
        >
          <span>
            {!linkSessionId
              ? "Browse Sessions"
              : status.recording
                ? "Open This Session"
                : "Open Last Session"}
          </span>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M5 12h14M12 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </div>
  );
}

/* ---------------------------------------------------------------- clips --- */

function ClipsPane({
  status,
  onChanged,
}: {
  status: CaptureStatus;
  onChanged: (s: CaptureStatus) => void;
}) {
  const [countdown, setCountdown] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [recentCaptures, setRecentCaptures] = useState<CaptureEntry[]>([]);
  const [todayDate, setTodayDate] = useState<string | null>(null);

  const refreshRecent = useCallback(async () => {
    try {
      const dates = await fetchDates();
      if (dates[0]) {
        setTodayDate(dates[0]);
        const entries = await fetchDateEntries(dates[0]);
        setRecentCaptures(entries.entries.slice(-6).reverse());
      } else {
        setRecentCaptures([]);
      }
    } catch {}
  }, []);

  // Refresh recent captures when a new capture lands
  useEffect(() => {
    void refreshRecent();
  }, [status.captures_today, refreshRecent]);

  // Countdown timer
  useEffect(() => {
    if (!status.running || !status.next_capture_time) {
      setCountdown(null);
      return;
    }
    const update = () => {
      const now = new Date();
      const [h, m, s] = status.next_capture_time!.split(":").map(Number);
      const target = new Date();
      target.setHours(h, m, s, 0);
      const diffMs = target.getTime() - now.getTime();
      if (diffMs <= 0) {
        setCountdown("now");
        return;
      }
      const mins = Math.floor(diffMs / 60000);
      const secs = Math.floor((diffMs % 60000) / 1000);
      setCountdown(`${mins}:${secs.toString().padStart(2, "0")}`);
    };
    update();
    const interval = setInterval(update, 1000);
    return () => clearInterval(interval);
  }, [status.running, status.next_capture_time]);

  const handleStart = useCallback(async () => {
    setBusy(true);
    try {
      await startCapture();
      onChanged(await getCaptureStatus());
    } catch {}
    setBusy(false);
  }, [onChanged]);

  const handleStop = useCallback(async () => {
    setBusy(true);
    try {
      await stopCapture();
      onChanged(await getCaptureStatus());
    } catch {}
    setBusy(false);
  }, [onChanged]);

  const handleSingle = useCallback(async () => {
    setBusy(true);
    try {
      await runSingleCapture();
      onChanged(await getCaptureStatus());
    } catch {}
    setBusy(false);
  }, [onChanged]);

  const handleRecentClick = useCallback((entry: CaptureEntry) => {
    if (todayDate) {
      void openViewer({ date: todayDate, timestamp: entry.timestamp });
    }
  }, [todayDate]);

  const handleRecentDelete = useCallback(async (e: React.MouseEvent, entry: CaptureEntry) => {
    e.stopPropagation();
    if (!todayDate) return;
    if (!confirm(`Delete capture at ${entry.display_time}?`)) return;
    await deleteEntry(todayDate, entry.timestamp);
    void refreshRecent();
  }, [todayDate, refreshRecent]);

  return (
    <>
      {/* Status section */}
      <div className="px-4 pt-3 pb-3">
        <div className={`flex items-center gap-2 px-3.5 py-2 rounded-[10px] text-[13px] font-semibold ${
          status.running ? "bg-red-50 text-red-600" : "bg-slate-50 text-slate-500"
        }`}>
          <span className={`w-2 h-2 rounded-full shrink-0 ${
            status.running
              ? "bg-red-600 shadow-[0_0_0_3px_rgba(220,38,38,0.15)] dot-active"
              : "bg-slate-300"
          }`} />
          <span>{status.running ? "Recording" : "Idle"}</span>
        </div>
        {status.running && countdown && (
          <div className="flex items-center justify-between mt-2 px-1">
            <span className="text-xs text-slate-400">Next capture in</span>
            <span className="text-sm font-bold tabular-nums font-mono text-slate-900">{countdown}</span>
          </div>
        )}
      </div>

      {/* Big action button */}
      <div className="px-4 pb-3 flex flex-col gap-2">
        {status.running ? (
          <button
            className="flex items-center justify-center gap-2.5 py-3.5 rounded-xl text-sm font-semibold tracking-tight cursor-pointer border-none bg-red-600 text-white shadow-[0_1px_3px_rgba(220,38,38,0.3),0_4px_12px_rgba(220,38,38,0.15)] hover:bg-red-700 hover:shadow-[0_2px_6px_rgba(220,38,38,0.4),0_6px_16px_rgba(220,38,38,0.2)] hover:-translate-y-px transition-all disabled:opacity-50 disabled:cursor-not-allowed"
            onClick={handleStop}
            disabled={busy}
          >
            <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
              <rect x="4" y="4" width="12" height="12" rx="2" />
            </svg>
            <span>Stop Recording</span>
          </button>
        ) : (
          <button
            className="flex items-center justify-center gap-2.5 py-3.5 rounded-xl text-sm font-semibold tracking-tight cursor-pointer border-none bg-sky-500 text-white shadow-[0_1px_3px_rgba(14,165,233,0.3),0_4px_12px_rgba(14,165,233,0.15)] hover:bg-sky-600 hover:shadow-[0_2px_6px_rgba(14,165,233,0.4),0_6px_16px_rgba(14,165,233,0.2)] hover:-translate-y-px transition-all disabled:opacity-50 disabled:cursor-not-allowed"
            onClick={handleStart}
            disabled={busy}
          >
            <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
              <circle cx="10" cy="10" r="7" />
            </svg>
            <span>Start Recording</span>
          </button>
        )}

        <button className={SECONDARY_BTN} onClick={handleSingle} disabled={busy}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <circle cx="12" cy="12" r="10" />
            <polyline points="12 6 12 12 16 14" />
          </svg>
          <span>Capture Now</span>
        </button>
      </div>

      {/* Stats */}
      <div className="flex items-center justify-center mx-4 py-2.5 border-t border-b border-slate-100 gap-4">
        <div className="flex flex-col items-center gap-0.5">
          <span className="text-[15px] font-bold text-slate-900 tabular-nums">{status.captures_today}</span>
          <span className="text-[10px] font-medium text-slate-400 uppercase tracking-wide">Today</span>
        </div>
        <div className="w-px h-7 bg-slate-100" />
        <div className="flex flex-col items-center gap-0.5">
          <span className="text-[15px] font-bold text-slate-900 tabular-nums">{status.last_capture_time ?? "--:--"}</span>
          <span className="text-[10px] font-medium text-slate-400 uppercase tracking-wide">Last capture</span>
        </div>
      </div>

      {/* Recent captures */}
      {recentCaptures.length > 0 && (
        <div className="flex-1 min-h-0 flex flex-col px-4 pt-2.5">
          <div className="flex items-center justify-between mb-1.5">
            <Eyebrow>Recent</Eyebrow>
            <span className="text-[11px] text-slate-300 font-mono">{todayDate}</span>
          </div>
          <div className="flex-1 overflow-y-auto">
            {recentCaptures.map((entry) => (
              <div
                key={entry.timestamp}
                className="group flex items-center gap-2.5 px-2 py-1.5 rounded-lg cursor-pointer hover:bg-slate-50 transition-colors"
                onClick={() => handleRecentClick(entry)}
              >
                <div className="shrink-0 w-5 flex items-center justify-center">
                  {entry.screen_video ? (
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#0EA5E9" strokeWidth="2.5">
                      <polygon points="5 3 19 12 5 21 5 3" fill="#0EA5E9" opacity="0.2" />
                    </svg>
                  ) : (
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#64748B" strokeWidth="2.5">
                      <rect x="3" y="3" width="18" height="18" rx="2" fill="#64748B" opacity="0.15" />
                      <circle cx="8.5" cy="8.5" r="1.5" fill="#64748B" />
                      <path d="M21 15l-5-5L5 21" />
                    </svg>
                  )}
                </div>
                <span className="text-[13px] font-medium text-slate-900 tabular-nums font-mono">{entry.display_time}</span>
                <span className="ml-auto text-[11px] text-slate-400">
                  {entry.screen_video ? "Video" : "Screenshot"}
                </span>
                <button
                  className="shrink-0 w-5 h-5 flex items-center justify-center border-none bg-transparent text-slate-300 rounded cursor-pointer opacity-0 group-hover:opacity-100 hover:text-red-500 hover:bg-red-50 transition-all"
                  onClick={(e) => handleRecentDelete(e, entry)}
                  title="Delete"
                >
                  <svg width="10" height="10" viewBox="0 0 10 10">
                    <line x1="2" y1="2" x2="8" y2="8" stroke="currentColor" strokeWidth="1.5" />
                    <line x1="8" y1="2" x2="2" y2="8" stroke="currentColor" strokeWidth="1.5" />
                  </svg>
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Browse button */}
      <div className="mt-auto px-4 py-2.5 pb-3.5">
        <button
          className="flex items-center justify-center gap-2 w-full py-2.5 rounded-[10px] text-[13px] font-medium cursor-pointer border border-slate-200 bg-slate-50 text-slate-600 hover:bg-slate-100 hover:border-slate-300 hover:text-slate-900 transition-all"
          onClick={() => void openViewer()}
        >
          <span>Browse All Captures</span>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M5 12h14M12 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </>
  );
}

/* ------------------------------------------------------------------ app --- */

export function CompactApp() {
  const [mode, setMode] = useState<Mode>("clips");
  const [status, setStatus] = useState<CaptureStatus>({
    running: false,
    last_capture_time: null,
    next_capture_time: null,
    captures_today: 0,
  });
  const [session, setSession] = useState<SessionStatus>(IDLE_SESSION);
  // null until settings have been read. The first-run panel is decided by the
  // persisted flag, so nothing else may render before it is known.
  const [onboarded, setOnboardedLocal] = useState<boolean | null>(null);
  // `null` = not probed yet, or the probe failed. Only a resolved setup with no
  // ffmpeg renders the banner, so a failed probe never invents a missing
  // dependency.
  const [ffmpeg, setFfmpeg] = useState<FfmpegSetup | null>(null);

  useEffect(() => {
    getSettings()
      .then((s) => setOnboardedLocal(s.onboarded))
      // Settings are unreachable — a first-run panel that cannot be dismissed
      // would be worse than none.
      .catch(() => setOnboardedLocal(true));
  }, []);

  // Both lookups spawn a process, so this runs once at startup and afterwards
  // only when the user does something that could have changed the answer —
  // never on a poll.
  const checkFfmpeg = useCallback(async () => {
    try {
      setFfmpeg(await getFfmpegSetup());
    } catch {
      setFfmpeg(null);
    }
  }, []);

  useEffect(() => {
    void checkFfmpeg();
  }, [checkFfmpeg]);

  // Both loops are polled from here regardless of which tab is showing, so the
  // mode switch can always say which one is live. They are independent: interval
  // clips and a continuous session can run at the same time.
  useEffect(() => {
    const poll = async () => {
      try {
        setStatus(await getCaptureStatus());
      } catch {}
    };
    void poll();
    const t = setInterval(poll, 3000);
    return () => clearInterval(t);
  }, []);

  useEffect(() => {
    const poll = async () => {
      try {
        setSession(await getSessionStatus());
      } catch {}
    };
    void poll();
    const t = setInterval(poll, 2000);
    return () => clearInterval(t);
  }, []);

  const handleClose = useCallback(() => { appWindow.close(); }, []);
  const handleMinimize = useCallback(() => { appWindow.minimize(); }, []);
  const handleDrag = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button")) return;
    appWindow.startDragging();
  }, []);

  const tabCls = (active: boolean) =>
    `flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-lg text-[13px] font-semibold cursor-pointer border-none transition-all ${
      active ? "bg-white text-slate-900 shadow-sm" : "bg-transparent text-slate-500 hover:text-slate-900"
    }`;

  return (
    <div className="flex flex-col h-screen bg-white rounded-xl overflow-hidden border border-slate-200">
      {/* Custom titlebar */}
      <div
        className="flex items-center justify-between px-3 py-2.5 bg-slate-50 border-b border-slate-100 select-none cursor-default"
        onMouseDown={handleDrag}
      >
        <div className="flex items-center gap-2">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" className="shrink-0">
            <circle cx="12" cy="12" r="10" fill="#0EA5E9" opacity="0.15" />
            <path d="M8 14c0-2.2 1.8-4 4-4s4 1.8 4 4" stroke="#0EA5E9" strokeWidth="2" strokeLinecap="round" />
            <circle cx="12" cy="8" r="2" fill="#0EA5E9" />
          </svg>
          <span className="text-[13px] font-bold text-slate-900 tracking-tight">ChirpTime</span>
        </div>
        <div className="flex gap-1">
          {/* Always reachable, both modes: the folder holding the recordings. */}
          <button
            className="w-6 h-6 flex items-center justify-center bg-transparent border-none text-slate-400 rounded-md hover:bg-slate-100 hover:text-slate-600 transition-all cursor-pointer"
            onClick={() => void openFolder().catch(() => {})}
            title="Open recordings folder"
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
            </svg>
          </button>
          <button
            className="w-6 h-6 flex items-center justify-center bg-transparent border-none text-slate-400 rounded-md hover:bg-slate-100 hover:text-slate-600 transition-all cursor-pointer"
            onClick={handleMinimize}
            title="Minimize"
          >
            <svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" strokeWidth="1.5" /></svg>
          </button>
          <button
            className="w-6 h-6 flex items-center justify-center bg-transparent border-none text-slate-400 rounded-md hover:bg-red-100 hover:text-red-500 transition-all cursor-pointer"
            onClick={handleClose}
            title="Close"
          >
            <svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" strokeWidth="1.5" /><line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" strokeWidth="1.5" /></svg>
          </button>
        </div>
      </div>

      {onboarded === null ? (
        <div className="flex-1 flex items-center justify-center text-slate-400 text-sm">
          Loading...
        </div>
      ) : !onboarded ? (
        <Onboarding onDone={() => setOnboardedLocal(true)} />
      ) : (
        <>
          {ffmpeg !== null && ffmpeg.ffmpeg_path === null && (
            <FfmpegBanner setup={ffmpeg} onResolved={() => void checkFfmpeg()} />
          )}

          {/* Mode switch. The red dot follows whichever mode is actually running,
              so switching tabs never hides that something is recording. */}
          <div className="px-4 pt-3">
            <div className="flex gap-0.5 p-0.5 rounded-[10px] border border-slate-200 bg-slate-100">
              <button className={tabCls(mode === "clips")} onClick={() => setMode("clips")}>
                <span>Clips</span>
                {status.running && <span className="w-1.5 h-1.5 rounded-full bg-red-500 dot-active" />}
              </button>
              <button className={tabCls(mode === "session")} onClick={() => setMode("session")}>
                <span>Session</span>
                {session.recording && <span className="w-1.5 h-1.5 rounded-full bg-red-500 dot-active" />}
              </button>
            </div>
          </div>

          {mode === "clips" ? (
            <ClipsPane status={status} onChanged={setStatus} />
          ) : (
            <SessionPane status={session} onChanged={setSession} />
          )}
        </>
      )}
    </div>
  );
}
