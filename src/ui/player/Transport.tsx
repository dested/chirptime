/**
 * The row under the stage. Normally the transport; with a selection, the
 * selection bar; while confirming a delete, the confirm row. One 40 px slot.
 */
import { useEffect, useRef, useState } from "react";
import type { SessionManifest, SessionTrack } from "../types";
import type { SyncEngine } from "../session/engine";
import { isVideoTrack } from "../session/engine";
import { fmtClock12, fmtHms, fmtSize, fmtSpan, fmtTimeAmPm } from "../session/format";
import { IconDown, IconFlag, IconMuted, IconPause, IconPlay, IconScissors, IconTrash, IconVolume, IconX } from "../app/icons";
import type { Axis } from "./timeline";
import { Popover, MenuItem } from "./Popover";
import { trackName } from "./Header";
import { collapse } from "../session/cuts";

export const SPEEDS: { rate: number; label: string }[] = [
  { rate: 0.5, label: "½×" },
  { rate: 1, label: "1×" },
  { rate: 2, label: "2×" },
  { rate: 4, label: "4×" },
  { rate: 16, label: "16×" },
];

const GHOST =
  "flex h-8 items-center gap-[7px] whitespace-nowrap rounded-ctl px-3 text-[12.5px] font-semibold text-ink-2 transition-colors duration-150 hover:bg-card/70 hover:text-ink";

/* ---- transport ---------------------------------------------------------- */

interface TransportProps {
  engine: SyncEngine;
  axis: Axis;
  manifest: SessionManifest;
  live: boolean;
  audioIds: string[];
  onAudio: (ids: string[]) => void;
  onMark: () => void;
  /** start a selection at the playhead, ready to stretch */
  onSelect: () => void;
}

export function Transport({ engine, axis, manifest, live, audioIds, onAudio, onMark, onSelect }: TransportProps) {
  const clockRef = useRef<HTMLSpanElement>(null);
  const elapsedRef = useRef<HTMLSpanElement>(null);
  const [playing, setPlaying] = useState(engine.playing);
  const [rate, setRate] = useState(engine.rate);
  const [audioOpen, setAudioOpen] = useState(false);
  const axisRef = useRef(axis);
  axisRef.current = axis;

  useEffect(() => {
    // 60 Hz text, written straight to the DOM: never React state at frame rate
    const write = (t: number) => {
      const a = axisRef.current;
      if (clockRef.current) clockRef.current.textContent = fmtClock12(t);
      if (elapsedRef.current) {
        const el = collapse(a.holes, t) - collapse(a.holes, a.from);
        elapsedRef.current.textContent = `${fmtHms(el)} / ${fmtHms(a.span)}`;
      }
    };
    write(engine.masterEpochMs);
    const offFrame = engine.subscribeFrame((s) => write(s.masterEpochMs));
    const offState = engine.subscribeState((s) => {
      setPlaying(s.playing);
      setRate(s.rate);
    });
    setPlaying(engine.playing);
    setRate(engine.rate);
    return () => {
      offFrame();
      offState();
    };
  }, [engine]);

  const audioTracks = manifest.tracks.filter((t) => !isVideoTrack(t));
  const muted = audioIds.length === 0;

  return (
    <div className="flex h-10 items-center gap-2.5">
      <button
        className="grid h-[38px] w-[38px] shrink-0 place-items-center rounded-full bg-ink text-white transition-transform duration-150 active:scale-95"
        onClick={() => engine.togglePlaying()}
        aria-label={playing ? "Pause (Space)" : "Play (Space)"}
        title={playing ? "Pause (Space)" : "Play (Space)"}
      >
        {playing ? <IconPause size={13} /> : <IconPlay size={13} />}
      </button>
      <div className="ml-1 flex items-baseline gap-2">
        <span ref={clockRef} className="num min-w-[118px] text-[19px] font-semibold tracking-[-0.03em]" />
        <span ref={elapsedRef} className="num whitespace-nowrap text-[11.5px] text-ink-3" />
      </div>
      <div className="ml-2 flex rounded-[9px] bg-sunk p-0.5" role="group" aria-label="Speed">
        {SPEEDS.map((s) => (
          <button
            key={s.rate}
            className={`h-[26px] whitespace-nowrap rounded-[7px] px-2.5 text-[12px] font-semibold transition-colors duration-150 ${
              rate === s.rate ? "bg-card text-ink shadow-[0_1px_2px_rgba(0,0,0,.08)]" : "text-ink-3 hover:text-ink-2"
            }`}
            onClick={() => engine.setRate(s.rate)}
          >
            {s.label}
          </button>
        ))}
      </div>
      <div className="flex-1" />
      <button
        className={GHOST}
        onClick={onSelect}
        title="Select a stretch to delete or export · or drag across the timeline, or press I and O"
      >
        <IconScissors size={15} />
        Select
      </button>
      <button
        className={`${GHOST} disabled:cursor-default disabled:opacity-45 disabled:hover:bg-transparent disabled:hover:text-ink-2`}
        disabled={!live}
        onClick={onMark}
        title={live ? "Mark this moment (M)" : "Marks can be added while this session is recording"}
      >
        <IconFlag size={15} />
        Mark
        <span className="kbd">M</span>
      </button>
      <div className="relative flex items-center">
        <button
          className={`${GHOST} px-2.5`}
          onClick={() => onAudio(muted ? audioTracks.map((t) => t.id) : [])}
          title={muted ? "Muted · click for sound" : "Mute"}
          aria-label={muted ? "Unmute" : "Mute"}
          disabled={audioTracks.length === 0}
        >
          {muted ? <IconMuted size={15} /> : <IconVolume size={15} />}
        </button>
        {audioTracks.length > 1 && (
          <button
            className="grid h-8 w-5 place-items-center rounded-[6px] text-ink-3 hover:bg-card/70 hover:text-ink"
            onClick={() => setAudioOpen((v) => !v)}
            aria-label="Audio sources"
            title="Audio sources"
          >
            <IconDown size={11} />
          </button>
        )}
        <Popover open={audioOpen} onClose={() => setAudioOpen(false)} align="right" side="above" className="w-[200px]">
          <div className="px-2.5 pb-1 pt-1.5 text-[11px] font-medium text-ink-3">Play sound from</div>
          {audioTracks.map((t) => {
            const on = audioIds.includes(t.id);
            return (
              <MenuItem
                key={t.id}
                checked={on}
                onClick={() => onAudio(on ? audioIds.filter((x) => x !== t.id) : [...audioIds, t.id])}
              >
                {trackName(t)}
              </MenuItem>
            );
          })}
        </Popover>
      </div>
    </div>
  );
}

/* ---- selection bar ------------------------------------------------------ */

interface SelectionBarProps {
  manifest: SessionManifest;
  from: number;
  to: number;
  /** collapsed length of the range */
  lengthMs: number;
  bytes: number;
  /** null = all tracks */
  tracks: string[] | null;
  onTracks: (ids: string[] | null) => void;
  onExport: () => void;
  onDelete: () => void;
  onDeleteBefore: () => void;
  onDeleteAfter: () => void;
  onClear: () => void;
}

function tracksLabel(all: SessionTrack[], ids: string[] | null): string {
  if (!ids || ids.length === all.length) return "All tracks";
  if (ids.length === 1) {
    const t = all.find((x) => x.id === ids[0]);
    return t ? trackName(t) : "1 track";
  }
  return `${ids.length} tracks`;
}

export function SelectionBar(p: SelectionBarProps) {
  const [tracksOpen, setTracksOpen] = useState(false);
  const [delOpen, setDelOpen] = useState(false);
  const all = p.manifest.tracks;
  const chosen = p.tracks ?? all.map((t) => t.id);

  const toggle = (id: string) => {
    const next = chosen.includes(id) ? chosen.filter((x) => x !== id) : [...chosen, id];
    if (next.length === 0) return; // at least one track
    p.onTracks(next.length === all.length ? null : all.map((t) => t.id).filter((x) => next.includes(x)));
  };

  return (
    <div className="flex h-10 items-center gap-2 rounded-[10px] border border-line bg-card pl-2 pr-1.5 shadow-[0_1px_2px_rgba(0,0,0,.04)]">
      <span className="num flex h-6 items-center rounded-chip bg-canary px-[9px] text-[12.5px] font-bold text-canary-ink">
        {fmtSpan(p.lengthMs)}
      </span>
      <span className="whitespace-nowrap text-[12.5px] text-ink-2">
        {fmtTimeAmPm(p.from)} – {fmtTimeAmPm(p.to)} <span className="text-ink-4">·</span>{" "}
        <span className="num text-[11.5px]">{fmtSize(p.bytes)}</span>
      </span>
      <div className="relative">
        <button className={`${GHOST} h-7 px-2`} onClick={() => setTracksOpen((v) => !v)} aria-expanded={tracksOpen}>
          {tracksLabel(all, p.tracks)}
          <IconDown size={12} />
        </button>
        <Popover open={tracksOpen} onClose={() => setTracksOpen(false)} side="above" className="w-[220px]">
          <div className="px-2.5 pb-1 pt-1.5 text-[11px] font-medium text-ink-3">Delete or export from</div>
          {all.map((t) => (
            <MenuItem key={t.id} checked={chosen.includes(t.id)} onClick={() => toggle(t.id)}>
              {trackName(t)}
            </MenuItem>
          ))}
        </Popover>
      </div>
      <div className="flex-1" />
      <button className={`${GHOST} h-7`} onClick={p.onExport}>
        Export
        <span className="kbd">E</span>
      </button>
      <div className="relative flex">
        <button
          className="flex h-7 items-center gap-[7px] whitespace-nowrap rounded-l-ctl border border-rec-line bg-card pl-3 pr-2.5 text-[12.5px] font-semibold text-rec transition-colors duration-150 hover:bg-[#FDF1EF]"
          onClick={p.onDelete}
        >
          <IconTrash size={14} />
          Delete
          <span className="kbd !border-rec-line !text-[#C9524A]">X</span>
        </button>
        <button
          className="grid h-7 w-6 place-items-center rounded-r-ctl border border-l-0 border-rec-line bg-card text-rec transition-colors duration-150 hover:bg-[#FDF1EF]"
          onClick={() => setDelOpen((v) => !v)}
          aria-label="More ways to delete"
          aria-expanded={delOpen}
        >
          <IconDown size={11} />
        </button>
        <Popover open={delOpen} onClose={() => setDelOpen(false)} align="right" side="above" className="w-[270px]">
          <MenuItem
            danger
            onClick={() => {
              setDelOpen(false);
              p.onDeleteBefore();
            }}
          >
            Delete everything before the playhead
          </MenuItem>
          <MenuItem
            danger
            onClick={() => {
              setDelOpen(false);
              p.onDeleteAfter();
            }}
          >
            Delete everything after the playhead
          </MenuItem>
        </Popover>
      </div>
      <button
        className="grid h-7 w-7 place-items-center rounded-ctl text-ink-2 transition-colors duration-150 hover:bg-paper hover:text-ink"
        onClick={p.onClear}
        title="Clear (Esc)"
        aria-label="Clear the selection"
      >
        <IconX size={13} />
      </button>
    </div>
  );
}

/* ---- confirm row -------------------------------------------------------- */

export function ConfirmRow({
  lengthMs,
  bytes,
  busy,
  onCancel,
  onConfirm,
}: {
  lengthMs: number;
  bytes: number;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="flex h-10 items-center gap-2 rounded-[10px] border border-rec-line bg-card pl-3 pr-1.5 shadow-[0_1px_2px_rgba(0,0,0,.04)]">
      <span className="flex text-rec">
        <IconTrash size={14} />
      </span>
      <span className="truncate whitespace-nowrap text-[12.5px] text-ink">
        <b className="font-bold">Delete {fmtSpan(lengthMs)}?</b>{" "}
        <span className="text-ink-2">Frees {fmtSize(bytes)} now. Undo for 10 seconds.</span>
      </span>
      <div className="flex-1" />
      <button className={`${GHOST} h-7`} onClick={onCancel} disabled={busy}>
        Cancel
        <span className="kbd">Esc</span>
      </button>
      <button
        className="flex h-7 items-center gap-[7px] whitespace-nowrap rounded-ctl border border-rec bg-rec px-3 text-[12.5px] font-semibold text-white transition-opacity duration-150 hover:opacity-90 disabled:opacity-60"
        onClick={onConfirm}
        disabled={busy}
      >
        {busy ? "Deleting" : "Delete"}
        <span className="kbd !border-white/35 !bg-transparent !text-white/75">↵</span>
      </button>
    </div>
  );
}
