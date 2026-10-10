/**
 * The widget while a session records: the clock, the desk map as a health
 * map, Mark with an inline note, Pause and Stop. An outage on any screen turns
 * into the alert at the top, in plain words, with "Restart screens".
 */
import { useEffect, useRef, useState } from "react";
import { thumbUrl } from "../hooks/mediaUrl";
import { fmtHms, fmtSize, fmtTimeAmPm } from "../session/format";
import type { SessionMarker, SessionStatus, TrackSummary } from "../types";
import { keyLabel, type RecorderControl } from "./api";
import { DeskMap, type TileLook } from "./DeskMap";
import { healthOf, outageTitle, screenOutage, troubleLines, type Outage } from "./live";
import type { RecordingSetup } from "./setup";
import { IconFlag, IconPause, IconPlay, IconWarn } from "./ui";

/** The newest frame of a track's last finished segment; it moves on once a minute. */
export function latestThumb(sessionId: string | null, t: TrackSummary | undefined, width: number): string | null {
  if (!sessionId || !t || t.last_segment_end_ms === null) return null;
  const at = Math.floor((t.last_segment_end_ms - 1_500) / 1000) * 1000;
  return thumbUrl(sessionId, t.id, at, width);
}

export interface MarksState {
  count: number;
  last: SessionMarker | null;
}

interface Props {
  setup: RecordingSetup;
  control: RecorderControl;
  status: SessionStatus | null;
  now: number;
  marks: MarksState;
  busy: "pause" | "stop" | "mark" | "restart" | null;
  error: string | null;
  onMark: () => void;
  onPauseToggle: () => void;
  onStop: () => void;
  onRestart: () => void;
  onDetails: () => void;
  onNote: (epochMs: number, label: string) => Promise<void>;
}

function AlertBox({ o, now, busy, onRestart, onDetails }: { o: Outage; now: number; busy: boolean; onRestart: () => void; onDetails: () => void }) {
  return (
    <div className="mt-1 shrink-0 rounded-[10px] border border-amber-line bg-amber-wash px-[11px] py-2.5" role="alert">
      <div className="flex items-center gap-[7px] text-[12.5px] font-bold text-[#8A4A05]">
        <IconWarn />
        {outageTitle(o, now)}
      </div>
      <div className="mt-1 text-[11.5px] leading-[1.4] text-[#7A4A12]">
        {o.why}. {o.still}
      </div>
      <div className="mt-2 flex gap-1.5">
        <button
          className="h-[26px] rounded-[7px] bg-[#8A4A05] px-2.5 text-[11.5px] font-bold text-white disabled:opacity-60"
          onClick={onRestart}
          disabled={busy}
        >
          {busy ? "Restarting" : "Restart screens"}
        </button>
        <button className="h-[26px] rounded-[7px] px-2.5 text-[11.5px] font-semibold text-[#8A4A05] hover:bg-amber-line/40" onClick={onDetails}>
          Details
        </button>
      </div>
    </div>
  );
}

function NoteField({ mark, onNote }: { mark: SessionMarker; onNote: (epochMs: number, label: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(mark.label);
  const [err, setErr] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setDraft(mark.label);
    setEditing(false);
    setErr(null);
  }, [mark.epoch_ms, mark.label]);

  // Tab from anywhere in the widget opens the note for the newest mark.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || editing) return;
      if (document.activeElement instanceof HTMLInputElement) return;
      e.preventDefault();
      setEditing(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing]);

  useEffect(() => {
    if (editing) input.current?.focus();
  }, [editing]);

  const at = fmtTimeAmPm(mark.epoch_ms);
  const save = async () => {
    try {
      await onNote(mark.epoch_ms, draft.trim());
      setEditing(false);
      setErr(null);
    } catch (e) {
      setErr(String(e));
    }
  };

  if (editing) {
    return (
      <div className="mt-3 shrink-0">
        <input
          ref={input}
          value={draft}
          maxLength={200}
          placeholder={`Note for the ${at} mark`}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void save();
            if (e.key === "Escape") {
              setDraft(mark.label);
              setEditing(false);
            }
          }}
          onBlur={() => void save()}
          className="h-[34px] w-full rounded-[9px] border border-ink-4 bg-card px-2.5 text-[12px] text-ink outline-none placeholder:text-ink-3"
        />
        {err && <div className="mt-1 text-[11px] text-rec">{err}</div>}
      </div>
    );
  }
  return (
    <button
      className="mt-3 flex h-[34px] shrink-0 items-center gap-2 rounded-[9px] border border-dashed border-line px-2.5 text-left text-[12px] text-ink-3 transition-colors hover:border-ink-4"
      onClick={() => setEditing(true)}
    >
      <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-canary-deep" />
      <span className="min-w-0 truncate">
        {mark.label ? (
          <>
            <span className="text-ink">{mark.label}</span> · {at}
          </>
        ) : (
          `Note for the ${at} mark`
        )}
      </span>
      <span className="kbd ml-auto">Tab</span>
    </button>
  );
}

function SoundState({ label, t, paused }: { label: string; t: TrackSummary | undefined; paused: boolean }) {
  if (!t) return null;
  const h = healthOf(t);
  if (paused) {
    return (
      <span className="flex flex-1 items-center gap-1.5">
        <span className="h-1.5 w-1.5 rounded-full bg-ink-4" />
        {label}
        <span className="text-ink-3">paused</span>
      </span>
    );
  }
  const dot = h === "ok" ? "bg-ink" : h === "warn" ? "bg-[#E39A3B]" : "bg-ink-4";
  return (
    <span className="flex flex-1 items-center gap-1.5">
      <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
      {label}
      <span className="text-ink-3">{h === "ok" ? "recording" : h === "warn" ? "restarting" : "stopped"}</span>
    </span>
  );
}

export function RecordingView({ setup, control, status, now, marks, busy, error, onMark, onPauseToggle, onStop, onRestart, onDetails, onNote }: Props) {
  const paused = control.paused_since_epoch_ms !== null;
  const outage = paused ? null : screenOutage(status);
  const started = status?.started_epoch_ms ?? null;
  const elapsed = started === null ? 0 : Math.max(0, now - started);
  const tracks = status?.tracks ?? [];
  const byId = (id: string) => tracks.find((t) => t.id === id);
  const sid = control.session_id;
  const keys = setup.prefs?.hotkeys;
  // While recording, the map shows what this session records, not the options.
  const recorded = setup.screens.filter((s) => byId(`screen-${s.index}`));
  const screens = recorded.length > 0 ? recorded : setup.screens;

  const look = (i: number): TileLook => {
    const t = byId(`screen-${i}`);
    return {
      kind: "live",
      thumb: latestThumb(sid, t, 240),
      health: paused ? "paused" : t ? healthOf(t) : "dead",
    };
  };
  const cam = byId("webcam");
  const camLook: TileLook | null = cam
    ? { kind: "live", thumb: latestThumb(sid, cam, 160), health: paused ? "paused" : healthOf(cam) }
    : null;
  const trouble = paused ? [] : troubleLines(status);

  return (
    <>
      {outage && <AlertBox o={outage} now={now} busy={busy === "restart"} onRestart={onRestart} onDetails={onDetails} />}
      <div className={`flex shrink-0 items-baseline gap-2 ${outage ? "pt-2.5" : "pt-1.5"}`}>
        <div
          className={`num font-medium leading-none tracking-[-0.05em] ${paused ? "text-ink-3" : ""}`}
          style={{ fontSize: outage ? 30 : 42 }}
          aria-label="Recording time"
        >
          {fmtHms(elapsed)}
        </div>
      </div>
      <div className="mt-1.5 shrink-0 text-[11.5px] text-ink-3">
        {paused && control.paused_since_epoch_ms !== null ? (
          <>
            <span className="font-semibold text-ink-2">Paused</span> since {fmtTimeAmPm(control.paused_since_epoch_ms)}
          </>
        ) : (
          <>Since {started === null ? "now" : fmtTimeAmPm(started)}</>
        )}{" "}
        <span className="text-ink-4">·</span> <span className="num text-[11px]">{fmtSize(status?.total_bytes ?? 0)}</span> written
      </div>

      <div className="mt-3 shrink-0 rounded-card border border-line bg-card px-3 pb-3 pt-2.5">
        <div className="flex justify-center">
          <DeskMap screens={screens} width={262} maxHeight={outage ? 84 : 110} look={look} camera={camLook} />
        </div>
        {(byId("mic") || tracks.some((t) => t.kind === "system-audio")) && (
          <div className="mt-2.5 flex items-center gap-2 text-[11.5px] text-ink-2">
            <SoundState label="Mic" t={tracks.find((t) => t.kind === "mic")} paused={paused} />
            <SoundState label="System" t={tracks.find((t) => t.kind === "system-audio")} paused={paused} />
          </div>
        )}
        {trouble.map((l) => (
          <div key={l.id} className="mt-2 flex items-center gap-1.5 text-[11.5px] text-[#9A5A0C]">
            <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-[#E39A3B]" />
            <span className="truncate">{l.text}</span>
          </div>
        ))}
      </div>

      <div className="mt-3 flex shrink-0 gap-2">
        <button
          className="flex h-11 flex-1 items-center gap-2 rounded-[11px] bg-canary px-3 text-[13.5px] font-bold text-canary-ink transition-opacity disabled:opacity-50"
          onClick={onMark}
          disabled={paused || busy === "mark"}
          title={paused ? "Paused: resume to mark" : "Flag this moment"}
        >
          <IconFlag />
          Mark
          {keys?.mark && (
            <span className="kbd ml-auto border-[rgba(43,34,0,.25)] bg-transparent text-[rgba(43,34,0,.6)]">{keyLabel(keys.mark)}</span>
          )}
        </button>
        <button
          title={paused ? `Resume${keys?.pause ? ` (${keyLabel(keys.pause)})` : ""}` : `Pause${keys?.pause ? ` (${keyLabel(keys.pause)})` : ""}`}
          aria-label={paused ? "Resume" : "Pause"}
          className={`grid h-11 w-11 place-items-center rounded-[11px] border transition-colors disabled:opacity-50 ${
            paused ? "border-ink bg-ink text-white" : "border-line bg-card text-ink hover:border-ink-4"
          }`}
          onClick={onPauseToggle}
          disabled={busy === "pause" || busy === "stop"}
        >
          {paused ? <IconPlay /> : <IconPause />}
        </button>
        <button
          title={`Stop${keys?.record ? ` (${keyLabel(keys.record)})` : ""}`}
          aria-label="Stop"
          className="grid h-11 w-11 place-items-center rounded-[11px] bg-rec shadow-[0_6px_14px_-6px_rgba(229,55,43,.6)] disabled:opacity-60"
          onClick={onStop}
          disabled={busy === "stop"}
        >
          <span className="h-[13px] w-[13px] rounded-[3px] bg-white" />
        </button>
      </div>

      {error && <div className="mt-2 shrink-0 text-[11.5px] text-rec">{error}</div>}
      {!outage && marks.last && <NoteField mark={marks.last} onNote={onNote} />}
      <div className="mt-2 shrink-0 text-[11.5px] text-ink-3">
        {marks.count === 0
          ? "Mark flags this moment so you can find it later"
          : outage && marks.last
            ? `${marks.count} ${marks.count === 1 ? "mark" : "marks"} · last at ${fmtTimeAmPm(marks.last.epoch_ms)}`
            : `${marks.count} ${marks.count === 1 ? "mark" : "marks"} this session`}
      </div>
    </>
  );
}
