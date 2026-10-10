/** The day log beside the stage: click a line to jump; away lines can select themselves for deletion. */
import { useEffect, useMemo, useRef, useState } from "react";
import type { SessionManifest } from "../types";
import type { SyncEngine } from "../session/engine";
import { fmtSpan, fmtTimeAmPm } from "../session/format";
import { IconTrash } from "../app/icons";
import { NoteField } from "../app/NoteField";
import { currentLineIndex, deriveLog } from "./log";
import type { LogKind, LogLine } from "./log";

interface Props {
  engine: SyncEngine;
  manifest: SessionManifest;
  live: boolean;
  width: number;
  onSeek: (t: number) => void;
  onSelectRange: (from: number, to: number) => void;
  onNoteSaved: (epochMs: number, label: string) => void;
}

type Filter = "all" | "marks";

/** Ticks once a second while a pending cut is counting down, otherwise never. */
function useNowWhile(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [active]);
  return now;
}

/** The playhead at 5 Hz. */
function usePlayhead(engine: SyncEngine): number {
  const [t, setT] = useState(engine.masterEpochMs);
  useEffect(() => {
    setT(engine.masterEpochMs);
    return engine.subscribeState((s) => setT(s.masterEpochMs));
  }, [engine]);
  return t;
}

export function LogCard({ engine, manifest, live, width, onSeek, onSelectRange, onNoteSaved }: Props) {
  const [filter, setFilter] = useState<Filter>("all");
  const hasPending = manifest.cuts.some((c) => c.state === "pending" && c.created_epoch_ms + 10_000 > Date.now());
  const now = useNowWhile(hasPending);
  const lines = useMemo(() => deriveLog(manifest, { live, now }), [manifest, live, now]);
  const playhead = usePlayhead(engine);
  const current = currentLineIndex(lines, playhead);
  const shown = filter === "marks" ? lines.filter((l) => l.kind === "mark") : lines;

  // keep the current line in view as the day plays
  const listRef = useRef<HTMLDivElement>(null);
  const currentKey = lines[current]?.key;
  useEffect(() => {
    if (!currentKey) return;
    const el = listRef.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(currentKey)}"]`);
    el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [currentKey]);

  return (
    <div
      className="flex shrink-0 flex-col overflow-hidden rounded-card border border-line bg-card"
      style={{ width }}
    >
      <div className="flex items-center px-3.5 pb-2.5 pt-3">
        <span className="text-[14px] font-bold">Log</span>
        <span className="ml-2 text-[12px] text-ink-3">click a line to jump</span>
        <div className="ml-auto flex rounded-[9px] bg-sunk p-0.5">
          {(["all", "marks"] as const).map((f) => (
            <button
              key={f}
              className={`h-[26px] rounded-[7px] px-2.5 text-[12px] font-semibold transition-colors duration-150 ${
                filter === f ? "bg-card text-ink shadow-[0_1px_2px_rgba(0,0,0,.08)]" : "text-ink-3 hover:text-ink-2"
              }`}
              onClick={() => setFilter(f)}
            >
              {f === "all" ? "All" : "Marks"}
            </button>
          ))}
        </div>
      </div>
      <div
        ref={listRef}
        className="min-h-0 flex-1 overflow-y-auto [mask-image:linear-gradient(#000_92%,transparent)]"
      >
        {shown.length === 0 && <div className="border-t border-line-2 px-3.5 py-3 text-[12px] text-ink-3">No marks in this session</div>}
        {shown.map((l) => (
          <Line
            key={l.key}
            line={l}
            current={lines[current]?.key === l.key}
            sessionId={manifest.id}
            live={live}
            onSeek={onSeek}
            onSelectRange={onSelectRange}
            onNoteSaved={onNoteSaved}
          />
        ))}
      </div>
    </div>
  );
}

const DOT: Partial<Record<LogKind, string>> = {
  mark: "inline-block h-[7px] w-[7px] rounded-[2px] bg-canary-deep mr-[7px] align-[1px]",
  stopped: "inline-block h-[7px] w-[7px] rounded-full bg-[#E39A3B] mr-[7px] align-[1px]",
  cut: "inline-block h-[2px] w-[7px] bg-rec mr-[7px] align-[3px]",
};

function Line({
  line,
  current,
  sessionId,
  live,
  onSeek,
  onSelectRange,
  onNoteSaved,
}: {
  line: LogLine;
  current: boolean;
  sessionId: string;
  live: boolean;
  onSeek: (t: number) => void;
  onSelectRange: (from: number, to: number) => void;
  onNoteSaved: (epochMs: number, label: string) => void;
}) {
  const note = line.note;
  const range = line.range;
  const titleTone =
    line.kind === "stopped" ? "text-[#8A4A05]" : line.kind === "cut" || line.kind === "interrupted" ? "text-ink-3" : "text-ink";
  return (
    <div
      data-key={line.key}
      role="button"
      tabIndex={0}
      className={`grid cursor-pointer grid-cols-[62px_1fr] gap-2.5 border-t border-line-2 px-3.5 py-[9px] transition-colors duration-150 ${
        current ? "bg-[rgba(255,216,77,.16)] shadow-[inset_3px_0_0_var(--color-canary-deep)]" : "hover:bg-paper"
      }`}
      onClick={() => onSeek(line.at)}
      onKeyDown={(e) => {
        if (e.key === "Enter" && e.target === e.currentTarget) onSeek(line.at);
      }}
    >
      <div className="num whitespace-nowrap pt-0.5 text-[11px] text-ink-3">{fmtTimeAmPm(line.at)}</div>
      <div>
        <div className={`text-[13px] leading-[1.38] ${titleTone}`}>
          {DOT[line.kind] && <span className={DOT[line.kind]} />}
          {line.title}
        </div>
        {note ? (
          <NoteField
            sessionId={sessionId}
            epochMs={note.epochMs}
            label={note.label}
            lockedReason={live ? "Stop recording to edit notes" : null}
            onSaved={(label) => onNoteSaved(note.epochMs, label)}
            className="mt-0.5 text-[12px] leading-[1.38]"
          />
        ) : (
          line.sub && <div className={`mt-0.5 text-[12px] leading-[1.38] ${line.subMuted ? "text-ink-4" : "text-ink-3"}`}>{line.sub}</div>
        )}
        {range && (
          <button
            className="mt-[7px] inline-flex h-6 items-center gap-1.5 rounded-[7px] border border-line bg-card px-[9px] text-[11.5px] font-semibold text-ink transition-colors duration-150 hover:border-rec-line hover:text-rec"
            onClick={(e) => {
              e.stopPropagation();
              onSelectRange(range.from, range.to);
            }}
          >
            <IconTrash size={13} />
            Delete {fmtSpan(range.to - range.from)}
          </button>
        )}
      </div>
    </div>
  );
}
