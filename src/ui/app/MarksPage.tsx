/**
 * `#/marks`: every mark across every session, newest first, grouped by day.
 * Each row shows the camera at that moment, the time, the note (edited in
 * place) and Open, which plays the session from the mark.
 */
import { useMemo } from "react";
import type { SessionManifest, SessionMarker, SessionTrack } from "../types";
import { thumbUrl } from "../hooks/mediaUrl";
import { fmtDayTitle, fmtTimeAmPm } from "../session/format";
import { thumbInstant } from "../player/timeline";
import { sessionHoles } from "../session/cuts";
import { useAppData } from "./appData";
import { href } from "./router";
import { NoteField } from "./NoteField";
import { dayStart, nearestFootage } from "./libraryModel";

interface MarkRow {
  manifest: SessionManifest;
  webcam: SessionTrack | null;
  mark: SessionMarker;
}

export function MarksPage() {
  const { sessions, manifests, manifestsReady, status, patchMark } = useAppData();
  const liveId = status?.recording ? status.session_id : null;

  const days = useMemo(() => {
    const rows: MarkRow[] = [];
    for (const m of manifests.values()) {
      const holes = sessionHoles(m);
      const webcam = m.tracks.find((t) => t.kind === "webcam") ?? null;
      for (const mark of m.markers) {
        if (holes.some((h) => mark.epoch_ms >= h.from && mark.epoch_ms < h.to)) continue;
        rows.push({ manifest: m, webcam, mark });
      }
    }
    rows.sort((a, b) => b.mark.epoch_ms - a.mark.epoch_ms);
    const out: { day: number; rows: MarkRow[] }[] = [];
    for (const r of rows) {
      const day = dayStart(r.mark.epoch_ms);
      const last = out[out.length - 1];
      if (last && last.day === day) last.rows.push(r);
      else out.push({ day, rows: [r] });
    }
    return out;
  }, [manifests]);

  const count = days.reduce((a, d) => a + d.rows.length, 0);
  const sessionCount = new Set(days.flatMap((d) => d.rows.map((r) => r.manifest.id))).size;
  const loading = sessions === null || (!manifestsReady && count === 0);

  return (
    <div className="h-full overflow-y-auto px-8 pb-10 pt-6">
      <h1 className="text-[24px] font-bold tracking-[-0.02em]">Marks</h1>
      {!loading && count > 0 && (
        <div className="num mt-1 text-[11px] text-ink-3">
          {count} {count === 1 ? "mark" : "marks"} across {sessionCount} {sessionCount === 1 ? "session" : "sessions"}
        </div>
      )}
      {loading && <p className="mt-6 text-ink-3">Gathering marks</p>}
      {!loading && count === 0 && (
        <p className="mt-6 max-w-[460px] text-ink-2">
          No marks yet. Press Mark in the recorder (Ctrl Alt M) to flag a moment, and it shows up here with room for a note.
        </p>
      )}

      <div className="max-w-[760px]">
        {days.map((d) => (
          <section key={d.day} className="mt-6">
            <h2 className="mb-2 text-[14px] font-bold">{fmtDayTitle(d.day)}</h2>
            <ul className="overflow-hidden rounded-card border border-line bg-card">
              {d.rows.map((r, i) => (
                <Row
                  key={`${r.manifest.id}-${r.mark.epoch_ms}`}
                  row={r}
                  first={i === 0}
                  live={r.manifest.id === liveId}
                  onSaved={(label) => patchMark(r.manifest.id, r.mark.epoch_ms, label)}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}

function Row({ row, first, live, onSaved }: { row: MarkRow; first: boolean; live: boolean; onSaved: (label: string) => void }) {
  const { manifest: m, webcam, mark } = row;
  const shot = webcam ? nearestFootage(m, webcam, mark.epoch_ms) : null;
  const open = href({ name: "session", id: m.id, t: mark.epoch_ms });
  const sessionEnd = m.ended_epoch_ms;
  return (
    <li className={`flex items-center gap-4 px-3 py-2.5 ${first ? "" : "border-t border-line-2"}`}>
      <a href={open} className="relative h-[54px] w-24 shrink-0 overflow-hidden rounded-[6px] bg-mat" tabIndex={-1} aria-hidden="true">
        {webcam && shot !== null && (
          <img
            src={thumbUrl(m.id, webcam.id, thumbInstant(shot), 192)}
            alt=""
            className="h-full w-full object-cover"
            onError={(e) => {
              e.currentTarget.style.visibility = "hidden";
            }}
          />
        )}
        <span className="absolute bottom-0 left-0 top-0 w-[3px] bg-canary" />
      </a>
      <div className="w-[76px] shrink-0">
        <div className="num text-[12.5px] font-semibold">{fmtTimeAmPm(mark.epoch_ms)}</div>
      </div>
      <div className="min-w-0 flex-1">
        <NoteField
          sessionId={m.id}
          epochMs={mark.epoch_ms}
          label={mark.label}
          lockedReason={live ? "Stop recording to edit notes" : null}
          onSaved={onSaved}
          className="text-[13px] leading-[1.4]"
          tone="text-ink"
        />
        <div className="num mt-0.5 text-[11px] text-ink-3">
          Session {fmtTimeAmPm(m.started_epoch_ms)} – {live ? "now" : sessionEnd !== null ? fmtTimeAmPm(sessionEnd) : "interrupted"}
        </div>
      </div>
      <a
        href={open}
        className="flex h-8 shrink-0 items-center rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper"
      >
        Open
      </a>
    </li>
  );
}
