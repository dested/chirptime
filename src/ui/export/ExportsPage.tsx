/**
 * `#/exports`: every finished export, newest first, with jobs still running
 * on top. A row plays the file in the default player, shows it in Explorer,
 * links back to the session at the range's start, or deletes it behind the
 * same 10 s undo toast the player uses (the file goes when the toast does).
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { mediaUrl, thumbUrl } from "../hooks/mediaUrl";
import { fmtDayTitle, fmtSize, fmtTimeAmPm } from "../session/format";
import { thumbInstant } from "../player/timeline";
import { Toast } from "../player/Toast";
import type { ToastState } from "../player/Toast";
import { errorText } from "../app/appData";
import { href } from "../app/router";
import { IconPlay, IconTrash } from "../app/icons";
import { EXPORTS_CHANGED, cancelExport, deleteExport, listExports, onExportProgress, revealExport } from "./api";
import { dropJob, useJobs } from "./jobs";
import type { Job } from "./jobs";
import type { ExportRecord } from "./types";

const UNDO_MS = 10_000;

const LAYOUT_NAMES: Record<string, string> = {
  devlog: "Devlog",
  short: "Short",
  "side-by-side": "Side by side",
  desk: "Desk",
  raw: "Raw tracks",
};

/** `24:05`, `1:02:10` */
function fmtLength(ms: number): string {
  const t = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = String(t % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

export function ExportsPage() {
  const [rows, setRows] = useState<ExportRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const jobs = useJobs();

  const refresh = useCallback(async () => {
    try {
      setRows(await listExports());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    // a job finishing adds a row
    return onExportProgress((p) => {
      if (p.state === "done") void refresh();
    });
  }, [refresh]);

  /* ---- delete with undo: hidden now, deleted when the toast runs out ---- */
  const [pending, setPending] = useState<ExportRecord | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);
  const pendingRef = useRef(pending);
  pendingRef.current = pending;

  const commit = useCallback(
    async (row: ExportRecord) => {
      try {
        await deleteExport(row.id);
      } catch (e) {
        setToast({ kind: "message", id: Date.now(), title: "Could not delete the export", detail: errorText(e), tone: "error" });
      }
      // the sidebar's count listens for this
      window.dispatchEvent(new Event(EXPORTS_CHANGED));
      void refresh();
    },
    [refresh],
  );

  const remove = useCallback(
    (row: ExportRecord) => {
      // a second delete commits the first one now
      const prev = pendingRef.current;
      if (prev) void commit(prev);
      const now = Date.now();
      setPending(row);
      setToast({ kind: "deleted", cutId: row.id, title: `Deleted ${row.title}`, detail: `frees ${fmtSize(row.bytes)}`, startedAt: now, until: now + UNDO_MS, undoing: false });
    },
    [commit],
  );

  const undo = useCallback(() => {
    setPending(null);
    setToast(null);
  }, []);

  const dismiss = useCallback(() => {
    const p = pendingRef.current;
    setToast(null);
    if (p) {
      setPending(null);
      void commit(p);
    }
  }, [commit]);

  // Leaving the page doesn't keep a delete waiting: it goes through at once.
  useEffect(
    () => () => {
      const p = pendingRef.current;
      if (p) void deleteExport(p.id).catch(() => {});
    },
    [],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "z" && (e.ctrlKey || e.metaKey) && pendingRef.current) {
        e.preventDefault();
        undo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo]);

  // A finished job's row comes from the index from now on.
  useEffect(() => {
    if (!rows) return;
    const listed = new Set(rows.map((r) => r.id));
    for (const j of jobs) if (listed.has(j.id)) dropJob(j.id);
  }, [rows, jobs]);

  const listed = new Set(rows?.map((r) => r.id) ?? []);
  const running = jobs.filter((j) => !listed.has(j.id) && (j.progress.state === "running" || j.progress.state === "failed"));
  const visible = (rows ?? []).filter((r) => r.id !== pending?.id);
  const totalBytes = visible.reduce((a, r) => a + r.bytes, 0);

  return (
    <div className="relative h-full">
      <div className="h-full overflow-y-auto px-8 pb-16 pt-6">
        <h1 className="text-[24px] font-bold tracking-[-0.02em]">Exports</h1>
        {rows !== null && visible.length > 0 && (
          <div className="num mt-1 text-[11px] text-ink-3">
            {visible.length} {visible.length === 1 ? "export" : "exports"} · {fmtSize(totalBytes)}
          </div>
        )}
        {error && <p className="mt-6 max-w-[560px] text-rec">{error}</p>}
        {rows === null && !error && <p className="mt-6 text-ink-3">Reading the exports folder</p>}
        {rows !== null && visible.length === 0 && running.length === 0 && (
          <p className="mt-6 max-w-[460px] text-ink-2">
            Nothing exported yet. Open a session, pick a stretch and press Export; the finished file shows up here.
          </p>
        )}

        {(running.length > 0 || visible.length > 0) && (
          <ul className="mt-6 max-w-[880px] overflow-hidden rounded-card border border-line bg-card">
            {running.map((j, i) => (
              <JobRow key={j.id} job={j} first={i === 0} />
            ))}
            {visible.map((r, i) => (
              <Row key={r.id} row={r} first={running.length === 0 && i === 0} onDelete={() => remove(r)} />
            ))}
          </ul>
        )}
      </div>
      <div className="pointer-events-none absolute bottom-3 left-0 right-0 z-30 flex justify-center">
        <Toast toast={toast} onUndo={undo} onDismiss={dismiss} />
      </div>
    </div>
  );
}

function Thumb({ row }: { row: ExportRecord }) {
  // the export's own first frame, else the session's screen at the range start
  const sources = [row.thumb ? mediaUrl(row.thumb) : null, thumbUrl(row.sessionId, "screen-0", thumbInstant(row.from + 10_000), 192)].filter(
    (s): s is string => s !== null,
  );
  const [i, setI] = useState(0);
  const src = sources[i];
  return (
    <span className="relative block h-[54px] w-24 shrink-0 overflow-hidden rounded-[6px] bg-mat">
      {src && <img key={src} src={src} alt="" draggable={false} className="h-full w-full object-cover" onError={() => setI((n) => n + 1)} />}
    </span>
  );
}

function Row({ row, first, onDelete }: { row: ExportRecord; first: boolean; onDelete: () => void }) {
  const [failure, setFailure] = useState<string | null>(null);
  const act = (p: Promise<void>) => {
    setFailure(null);
    p.catch((e: unknown) => setFailure(errorText(e)));
  };
  const name = LAYOUT_NAMES[row.layout] ?? row.layout;
  return (
    <li className={`flex items-center gap-4 px-3 py-2.5 ${first ? "" : "border-t border-line-2"}`}>
      <button className="group relative shrink-0" onClick={() => act(revealExport(row.id, true))} title="Play" aria-label={`Play ${row.title}`}>
        <Thumb row={row} />
        <span className="absolute inset-0 grid place-items-center rounded-[6px] text-white opacity-0 transition-opacity duration-150 group-hover:bg-black/35 group-hover:opacity-100">
          <IconPlay size={18} />
        </span>
      </button>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-semibold">{row.title}</div>
        <div className="num mt-0.5 truncate text-[11px] text-ink-3">
          {name !== row.title ? `${name} · ` : ""}
          <a className="text-ink-2 underline decoration-line underline-offset-2 hover:decoration-ink-3" href={href({ name: "session", id: row.sessionId, t: row.from })}>
            {fmtDayTitle(row.from)}, {fmtTimeAmPm(row.from)} – {fmtTimeAmPm(row.to)}
          </a>
        </div>
        {failure && <div className="mt-0.5 text-[11px] text-rec">{failure}</div>}
      </div>
      <div className="num w-[60px] shrink-0 text-right text-[12px] text-ink-2">{fmtLength(row.durationMs)}</div>
      <div className="num w-[64px] shrink-0 text-right text-[12px] text-ink-2">{fmtSize(row.bytes)}</div>
      <div className="flex shrink-0 items-center gap-1.5">
        <button
          className="flex h-8 items-center gap-1.5 rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper"
          onClick={() => act(revealExport(row.id, true))}
        >
          <IconPlay size={13} />
          Play
        </button>
        <button
          className="flex h-8 items-center rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper"
          onClick={() => act(revealExport(row.id))}
        >
          Show in folder
        </button>
        <button
          className="grid h-8 w-8 place-items-center rounded-ctl text-ink-3 transition-colors duration-150 hover:bg-[#FDECEA] hover:text-rec"
          onClick={onDelete}
          title="Delete"
          aria-label={`Delete ${row.title}`}
        >
          <IconTrash size={15} />
        </button>
      </div>
    </li>
  );
}

function JobRow({ job, first }: { job: Job; first: boolean }) {
  const p = job.progress;
  const pct = Math.round(p.fraction * 100);
  const failed = p.state === "failed";
  return (
    <li className={`flex items-center gap-4 px-3 py-2.5 ${first ? "" : "border-t border-line-2"}`}>
      <span className="relative block h-[54px] w-24 shrink-0 overflow-hidden rounded-[6px] bg-mat">
        <img
          src={thumbUrl(job.sessionId, "screen-0", thumbInstant(job.from + 10_000), 192)}
          alt=""
          className="h-full w-full object-cover opacity-50"
          onError={(e) => {
            e.currentTarget.style.visibility = "hidden";
          }}
        />
      </span>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-semibold">{job.title}</div>
        {failed ? (
          <div className="mt-0.5 text-[11px] text-rec">{p.error ?? "The export stopped"}</div>
        ) : (
          <div className="mt-1.5 flex items-center gap-2.5">
            <div className="relative h-1.5 w-[220px] overflow-hidden rounded-[3px] bg-sunk">
              <div className="absolute bottom-0 left-0 top-0 rounded-[3px] bg-ink transition-[width] duration-200" style={{ width: `${pct}%` }} />
            </div>
            <span className="num text-[11px] text-ink-3">Exporting · {pct}%</span>
          </div>
        )}
      </div>
      {failed ? (
        <button
          className="flex h-8 items-center rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper"
          onClick={() => dropJob(job.id)}
        >
          Dismiss
        </button>
      ) : (
        <button
          className="flex h-8 items-center rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold transition-colors duration-150 hover:bg-paper"
          onClick={() => void cancelExport(job.id).catch(() => {})}
        >
          Cancel
        </button>
      )}
    </li>
  );
}
