/**
 * FFmpeg: what was found, or the way to get it. One flow for Settings and
 * first run. The download states its size before the button is pressed, and
 * shows progress all the way, because a silent 90 MB transfer reads as a hang.
 */
import { useCallback, useState } from "react";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { clearFfmpegDir, installFfmpeg, pickFfmpegDir } from "../hooks/useApi";
import { useTauriEvent } from "../recorder/live";
import { fmtSize } from "../session/format";
import type { FfmpegProgress, FfmpegSetup } from "../types";

const MANUAL_URL = "https://www.gyan.dev/ffmpeg/builds/";

const PHASE: Record<FfmpegProgress["phase"], string> = {
  downloading: "Downloading FFmpeg",
  verifying: "Checking the download",
  extracting: "Unpacking",
  checking: "Making sure it runs",
  done: "Done",
};

export const btn =
  "h-8 rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold text-ink-2 transition-colors hover:border-ink-4 hover:text-ink disabled:opacity-50";
export const btnInk = "h-8 rounded-ctl bg-ink px-3 text-[12.5px] font-bold text-white transition-opacity disabled:opacity-50";

export function FfmpegPanel({ setup, onChanged }: { setup: FfmpegSetup | null; onChanged: () => void }) {
  const [progress, setProgress] = useState<FfmpegProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Subscribed for the life of the panel, so one reopened mid-install rejoins it.
  useTauriEvent<FfmpegProgress>("ffmpeg-install-progress", setProgress);

  const run = useCallback(
    async (f: () => Promise<unknown>) => {
      setBusy(true);
      setError(null);
      try {
        await f();
        onChanged();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
        setProgress(null);
      }
    },
    [onChanged],
  );

  if (!setup) return <div className="text-[12.5px] text-ink-3">Looking for FFmpeg</div>;

  if (busy && progress) {
    const pct = progress.total_bytes > 0 ? Math.min(100, Math.round((progress.downloaded_bytes / progress.total_bytes) * 100)) : 0;
    const downloading = progress.phase === "downloading";
    return (
      <div>
        <div className="flex items-baseline justify-between text-[12.5px]">
          <span className="font-semibold">{PHASE[progress.phase]}</span>
          {downloading && (
            <span className="num text-[11px] text-ink-3">
              {fmtSize(progress.downloaded_bytes)} of {fmtSize(progress.total_bytes)}
            </span>
          )}
        </div>
        <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-sunk">
          <div
            className={`h-full rounded-full bg-ink transition-[width] duration-200 ${downloading ? "" : "animate-pulse"}`}
            style={{ width: downloading ? `${pct}%` : "100%" }}
          />
        </div>
        <div className="mt-2 text-[11.5px] text-ink-3">Nothing is installed until the download matches its known checksum</div>
      </div>
    );
  }

  if (setup.ffmpeg_path) {
    const how = setup.manual_dir ? "From a folder you chose" : setup.managed ? "Installed by ChirpTime" : "Found on this PC";
    return (
      <div>
        <div className="flex items-center gap-2 text-[12.5px] font-semibold">
          <span className="h-2 w-2 rounded-full bg-ink" />
          {how}
        </div>
        <div className="num mt-1 break-all text-[11px] text-ink-3">{setup.ffmpeg_path}</div>
        {!setup.ffprobe_path && <div className="mt-1 text-[11.5px] text-[#9A5A0C]">ffprobe.exe is missing next to it; sessions cannot be repaired without it</div>}
        <div className="mt-3 flex gap-2">
          <button className={btn} disabled={busy} onClick={() => void run(pickFfmpegDir)}>
            Use my own
          </button>
          {setup.manual_dir && (
            <button className={btn} disabled={busy} onClick={() => void run(clearFfmpegDir)}>
              Forget my folder
            </button>
          )}
          <button className={btn} disabled={busy} onClick={onChanged}>
            Check again
          </button>
        </div>
        {error && <div className="mt-2 text-[11.5px] text-rec">{error}</div>}
      </div>
    );
  }

  return (
    <div>
      <div className="text-[12.5px] leading-[1.5] text-ink-2">
        ChirpTime records through FFmpeg and cannot capture anything without it. It can fetch it for you: about{" "}
        {fmtSize(setup.download_bytes)}, checked against a known checksum, installed just for you with no admin rights.
      </div>
      <div className="num mt-1.5 break-all text-[11px] text-ink-3">{setup.install_dir}</div>
      <div className="mt-3 flex flex-wrap gap-2">
        <button className={btnInk} disabled={busy} onClick={() => void run(installFfmpeg)}>
          {error ? "Try again" : `Download FFmpeg (${fmtSize(setup.download_bytes)})`}
        </button>
        <button className={btn} disabled={busy} onClick={() => void run(pickFfmpegDir)}>
          I already have it
        </button>
        <button className={btn} disabled={busy} onClick={onChanged}>
          Check again
        </button>
      </div>
      <button className="mt-2 text-[11.5px] text-ink-3 underline hover:text-ink-2" onClick={() => void openUrl(MANUAL_URL).catch(() => {})}>
        Or install it yourself
      </button>
      {error && <div className="mt-2 text-[11.5px] text-rec">{error}</div>}
    </div>
  );
}
