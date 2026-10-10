/** The widget while nothing records: what the next recording will take, and Start. */
import { useState } from "react";
import { thumbUrl } from "../hooks/mediaUrl";
import { fmtSize, fmtTimeAmPm } from "../session/format";
import type { SessionSummary } from "../types";
import { keyLabel } from "./api";
import { DeskMap } from "./DeskMap";
import { freeLine, idleLine, screenOn, toggleScreen, type RecordingSetup } from "./setup";
import { CheckBox, IconChevron, IconWarn } from "./ui";

interface Props {
  setup: RecordingSetup;
  lastSession: SessionSummary | null;
  busy: boolean;
  error: string | null;
  needsSetup: boolean;
  onStart: () => void;
  onOptions: () => void;
  onOpen: (route: string) => void;
}

/** `today 11:00 AM`, `yesterday 4:12 PM`, `Oct 3 9:05 AM` */
export function whenLabel(epochMs: number, now = Date.now()): string {
  const d = new Date(epochMs);
  const day = (t: number) => new Date(t).toDateString();
  const time = fmtTimeAmPm(epochMs);
  if (day(epochMs) === day(now)) return `today ${time}`;
  if (day(epochMs) === day(now - 86_400_000)) return `yesterday ${time}`;
  return `${d.toLocaleDateString("en-US", { month: "short", day: "numeric" })} ${time}`;
}

/** `10 min`, `2h 05m` */
export function lengthLabel(ms: number): string {
  const min = Math.round(ms / 60_000);
  if (min < 1) return `${Math.max(1, Math.round(ms / 1000))} s`;
  if (min < 60) return `${min} min`;
  return `${Math.floor(min / 60)}h ${String(min % 60).padStart(2, "0")}m`;
}

function LastSessionCard({ s, onOpen }: { s: SessionSummary; onOpen: (route: string) => void }) {
  const [thumbOk, setThumbOk] = useState(true);
  const at = s.started_epoch_ms + Math.min(30_000, Math.floor(s.duration_ms / 2));
  return (
    <button
      className="mb-3 mt-auto flex shrink-0 items-center gap-2.5 rounded-card border border-line bg-card p-2 text-left transition-colors hover:border-ink-4"
      onClick={() => onOpen(`#/session/${encodeURIComponent(s.id)}`)}
    >
      {thumbOk ? (
        <img
          src={thumbUrl(s.id, "screen-0", Math.floor(at / 1000) * 1000, 128)}
          alt=""
          onError={() => setThumbOk(false)}
          className="h-7 w-16 shrink-0 rounded-[4px] bg-sunk object-cover"
        />
      ) : (
        <span className="h-7 w-16 shrink-0 rounded-[4px] bg-sunk" />
      )}
      <span className="min-w-0">
        <span className="block truncate text-[12.5px] font-semibold">Last session, {whenLabel(s.started_epoch_ms)}</span>
        <span className="num mt-0.5 block text-[10.5px] text-ink-3">
          {lengthLabel(s.duration_ms)} · {fmtSize(s.bytes)}
        </span>
      </span>
      <IconChevron className="ml-auto shrink-0 text-ink-4" />
    </button>
  );
}

function Chip({ on, label, onClick }: { on: boolean; label: string; onClick: () => void }) {
  return (
    <button
      aria-pressed={on}
      onClick={onClick}
      className={`flex h-7 min-w-0 items-center gap-1.5 whitespace-nowrap rounded-[7px] bg-paper px-2 text-[11.5px] font-medium transition-colors hover:bg-sunk ${on ? "text-ink-2" : "text-ink-3"}`}
    >
      <CheckBox on={on} />
      {label}
    </button>
  );
}

export function IdleView({ setup, lastSession, busy, error, needsSetup, onStart, onOptions, onOpen }: Props) {
  const { options, screens, setOptions, env, ffmpeg, prefs } = setup;
  const noFfmpeg = ffmpeg !== null && ffmpeg.ffmpeg_path === null;
  const free = freeLine(env, fmtSize);
  const nothing = !options.record_screens && !options.record_webcam && !options.record_mic && !options.record_system_audio;
  const startKey = prefs?.hotkeys.record ?? "";

  return (
    <>
      <div className="shrink-0 pt-2">
        <div className="text-[22px] font-bold tracking-[-0.02em]">Not recording</div>
        <div className="num mt-1 h-[15px] text-[11px] text-ink-3">{free ?? ""}</div>
      </div>

      {(noFfmpeg || needsSetup) && (
        <div className="mt-3 shrink-0 rounded-[10px] border border-amber-line bg-amber-wash px-[11px] py-2.5">
          <div className="flex items-center gap-[7px] text-[12.5px] font-bold text-[#8A4A05]">
            <IconWarn />
            {noFfmpeg ? "FFmpeg is not installed" : "Finish setting up"}
          </div>
          <div className="mt-1 text-[11.5px] leading-[1.4] text-[#7A4A12]">
            {noFfmpeg
              ? "ChirpTime records through FFmpeg. Nothing can be captured until it is set up."
              : "Pick your screens, camera and folder in the ChirpTime window."}
          </div>
          <button
            className="mt-2 h-[26px] rounded-[7px] bg-[#8A4A05] px-2.5 text-[11.5px] font-bold text-white"
            onClick={() => onOpen(noFfmpeg ? "#/settings" : "#/library")}
          >
            {noFfmpeg ? "Set up FFmpeg" : "Open setup"}
          </button>
        </div>
      )}

      <div className="mt-3.5 shrink-0 rounded-card border border-line bg-card p-3">
        <div className="flex items-baseline">
          <span className="text-[12.5px] font-semibold">Records</span>
          <button className="ml-auto text-[12px] font-semibold text-ink-3 hover:text-ink" onClick={onOptions}>
            Change
          </button>
        </div>
        <div className="mt-2.5 flex justify-center">
          <DeskMap
            screens={screens}
            width={262}
            maxHeight={noFfmpeg || needsSetup ? 70 : 110}
            look={(i) => ({ kind: "pick", on: screenOn(options, screens, i) })}
            camera={{ kind: "pick", on: options.record_webcam }}
            onPick={(i) => setOptions(toggleScreen(options, screens, i))}
            onCamera={() => setOptions({ ...options, record_webcam: !options.record_webcam })}
          />
        </div>
        <div className="mt-2.5 grid grid-cols-3 gap-[5px]">
          <Chip on={options.record_webcam} label="Camera" onClick={() => setOptions({ ...options, record_webcam: !options.record_webcam })} />
          <Chip on={options.record_mic} label="Mic" onClick={() => setOptions({ ...options, record_mic: !options.record_mic })} />
          <Chip
            on={options.record_system_audio}
            label="System"
            onClick={() => setOptions({ ...options, record_system_audio: !options.record_system_audio })}
          />
        </div>
      </div>

      <button
        className="mt-3.5 flex h-12 shrink-0 items-center gap-2.5 rounded-[11px] bg-ink px-3.5 text-[14px] font-bold tracking-[-0.01em] text-white shadow-[inset_0_1px_0_rgba(255,255,255,.12),0_6px_16px_-6px_rgba(26,25,23,.5)] transition-opacity disabled:opacity-50"
        onClick={onStart}
        disabled={busy || noFfmpeg || nothing}
      >
        <span className="h-3 w-3 rounded-full bg-rec" />
        {busy ? "Starting" : "Start recording"}
        {startKey && (
          <span className="kbd ml-auto border-white/20 bg-transparent text-white/50">{keyLabel(startKey)}</span>
        )}
      </button>
      <div className={`mt-2 shrink-0 text-center text-[11.5px] ${error ? "text-rec" : "text-ink-3"}`}>
        {error ?? (nothing ? "Pick at least one thing to record" : idleLine(options))}
      </div>

      {/* A setup notice takes the room the last session would use. */}
      {lastSession && !noFfmpeg && !needsSetup && <LastSessionCard s={lastSession} onOpen={onOpen} />}
    </>
  );
}
