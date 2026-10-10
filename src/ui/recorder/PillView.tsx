/** The recorder shrunk to a 264×52 pill: the clock, a tick per track, Mark and Stop. */
import type { MouseEvent } from "react";
import { fmtHms } from "../session/format";
import type { SessionStatus } from "../types";
import { keyLabel, type RecorderControl } from "./api";
import { healthOf, screenIndex } from "./live";
import { IconFlag } from "./ui";

interface Props {
  control: RecorderControl;
  status: SessionStatus | null;
  now: number;
  markKey: string;
  onExpand: () => void;
  onMark: () => void;
  onStop: () => void;
  onDrag: (e: MouseEvent) => void;
}

export function PillView({ control, status, now, markKey, onExpand, onMark, onStop, onDrag }: Props) {
  const paused = control.paused_since_epoch_ms !== null;
  const started = status?.started_epoch_ms ?? null;
  const tracks = status?.tracks ?? [];
  const screens = tracks.filter((t) => screenIndex(t) !== null).sort((a, b) => (screenIndex(a) ?? 0) - (screenIndex(b) ?? 0));
  const cam = tracks.find((t) => t.kind === "webcam");
  const tick = (ok: boolean) => (ok ? "rgba(243,240,232,.45)" : "#E39A3B");

  return (
    <div
      className="flex h-[52px] w-[264px] select-none items-center gap-2.5 rounded-[26px] bg-[#161615] pl-4 pr-2 text-[#F3F0E8] shadow-[inset_0_0_0_1px_rgba(255,255,255,.08)]"
      onMouseDown={onDrag}
    >
      <button className="flex items-center gap-2.5" onClick={onExpand} title="Show the full recorder">
        <span
          className={`h-2.5 w-2.5 rounded-full ${paused ? "bg-[rgba(243,240,232,.45)]" : "bg-rec dot-active"}`}
          style={paused ? undefined : { boxShadow: "0 0 0 4px rgba(229,55,43,.22)" }}
        />
        <span className={`num text-[16px] font-medium tracking-[-0.03em] ${paused ? "text-[rgba(243,240,232,.5)]" : ""}`}>
          {started === null ? "0:00:00" : fmtHms(now - started)}
        </span>
      </button>
      <span className="flex gap-[3px]" title={paused ? "Paused" : "Screens and camera"}>
        {screens.map((t) => (
          <i key={t.id} className="block h-3 w-1 rounded-[2px]" style={{ background: tick(paused || healthOf(t) === "ok") }} />
        ))}
        {cam && <i className="ml-[3px] block h-3 w-[7px] rounded-[2px]" style={{ background: tick(paused || healthOf(cam) === "ok") }} />}
      </span>
      <div className="flex-1" />
      <button
        className="grid h-9 w-9 place-items-center rounded-full bg-canary text-canary-ink disabled:opacity-40"
        title={paused ? "Paused: resume to mark" : `Mark${markKey ? ` (${keyLabel(markKey)})` : ""}`}
        aria-label="Mark"
        disabled={paused}
        onClick={onMark}
      >
        <IconFlag />
      </button>
      <button className="grid h-9 w-9 place-items-center rounded-full bg-rec" title="Stop" aria-label="Stop" onClick={onStop}>
        <span className="h-[11px] w-[11px] rounded-[2px] bg-white" />
      </button>
    </div>
  );
}
