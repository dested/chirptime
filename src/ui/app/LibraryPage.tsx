/**
 * `#/library`: every session on a time map of days × hours, with an inspector
 * for the selected one. Click a block to inspect it, double-click (or Open) to
 * play it. Whole sessions delete from the inspector with the Player's undo
 * toast; sessions under a minute are offered for clean-up in one go.
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { SessionManifest } from "../types";
import { deleteSession, undoDeleteSession } from "../hooks/useApi";
import { thumbUrl } from "../hooks/mediaUrl";
import { fmtDayTitle, fmtSize, fmtSpan, fmtTick, fmtTimeAmPm } from "../session/format";
import { isExpectedGap, screenNumber, thumbInstant } from "../player/timeline";
import { Toast } from "../player/Toast";
import type { ToastState } from "../player/Toast";
import { errorText, useAppData } from "./appData";
import { href, navigate } from "./router";
import { IconExport, IconInfo, IconTrash, IconWarn } from "./icons";
import {
  dayStart,
  footageMs,
  heroScreen,
  hourOfDay,
  hourRange,
  layoutDays,
  libraryTotals,
  nearestFootage,
  sessionFacts,
  thumbInstants,
} from "./libraryModel";
import type { DayRow, Piece, SessionFacts } from "./libraryModel";

const LABEL_W = 96;
const ROW_H = 30;
const MONTH_H = 28;
const INSPECTOR_W = 308;
const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WD = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

type Mode = "days" | "list";

/** Ticks every `ms` while `active`, so a live block grows. */
function useNow(active: boolean, ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(t);
  }, [active, ms]);
  return now;
}

function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((e) => {
      const r = e[0]?.contentRect;
      if (r) setW(r.width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

export function LibraryPage() {
  const { sessions, sessionsError, manifests, status, refreshSessions } = useAppData();
  const liveId = status?.recording ? status.session_id : null;
  const now = useNow(liveId !== null, 15_000);

  // While recording, read the live session again now and then so its block
  // picks up away stretches and thumbnails.
  useEffect(() => {
    if (!liveId) return;
    const t = window.setInterval(() => void refreshSessions(), 60_000);
    return () => window.clearInterval(t);
  }, [liveId, refreshSessions]);

  const facts = useMemo(
    () => (sessions ?? []).map((s) => sessionFacts(s, manifests.get(s.id) ?? null, liveId, now)),
    [sessions, manifests, liveId, now],
  );
  const totals = libraryTotals(facts);
  const shorts = facts.filter((f) => f.short);

  const [mode, setMode] = useState<Mode>("days");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const autoPicked = useRef(false);
  useEffect(() => {
    if (autoPicked.current || facts.length === 0) return;
    autoPicked.current = true;
    setSelectedId((facts.find((f) => !f.short) ?? facts[0])?.id ?? null);
  }, [facts]);
  const selected = facts.find((f) => f.id === selectedId) ?? null;

  /* ---- deleting whole sessions ---- */
  const [toast, setToast] = useState<ToastState | null>(null);
  const trashed = useRef<string[]>([]);
  const [busy, setBusy] = useState(false);

  const trash = useCallback(
    async (list: SessionFacts[], title: string) => {
      if (busy || list.length === 0) return;
      setBusy(true);
      const done: string[] = [];
      let freed = 0;
      let until = Infinity;
      let failure: string | null = null;
      for (const f of list) {
        try {
          const r = await deleteSession(f.id);
          done.push(f.id);
          freed += r.bytes;
          until = Math.min(until, r.undo_until_epoch_ms);
        } catch (e) {
          failure = errorText(e);
        }
      }
      setBusy(false);
      if (done.length) {
        // the next session along keeps the inspector filled
        const order = facts.map((f) => f.id);
        const at = selectedId ? order.indexOf(selectedId) : -1;
        if (selectedId && done.includes(selectedId)) {
          const next = order.slice(at + 1).concat(order.slice(0, at).reverse()).find((id) => !done.includes(id)) ?? null;
          setSelectedId(next);
        }
        trashed.current = done;
        setToast({
          kind: "deleted",
          cutId: `sessions-${done.join(",")}-${Date.now()}`,
          title: failure ? `Deleted ${done.length} of ${list.length}` : title,
          detail: `freed ${fmtSize(freed)}`,
          startedAt: Date.now(),
          until,
          undoing: false,
        });
        void refreshSessions();
      } else if (failure) {
        setToast({ kind: "message", id: Date.now(), title: "Nothing was deleted", detail: failure, tone: "error" });
      }
    },
    [busy, facts, selectedId, refreshSessions],
  );

  const undo = useCallback(async () => {
    if (!toast || toast.kind !== "deleted" || toast.undoing) return;
    setToast({ ...toast, undoing: true });
    let failure: string | null = null;
    const ids = trashed.current;
    for (const id of ids) {
      try {
        await undoDeleteSession(id);
      } catch (e) {
        failure = errorText(e);
      }
    }
    trashed.current = [];
    await refreshSessions();
    setToast(
      failure
        ? { kind: "message", id: Date.now(), title: "Could not undo", detail: failure, tone: "error" }
        : { kind: "message", id: Date.now(), title: ids.length === 1 ? "Restored the session" : `Restored ${ids.length} sessions`, detail: null, tone: "info" },
    );
    if (ids.length === 1 && !failure) setSelectedId(ids[0] ?? null);
  }, [toast, refreshSessions]);

  const [confirming, setConfirming] = useState(false);
  useEffect(() => setConfirming(false), [selectedId]);

  const deleteSelected = useCallback(() => {
    if (!selected) return;
    setConfirming(false);
    void trash([selected], `Deleted ${fmtDayTitle(selected.start)}`);
  }, [selected, trash]);

  /* ---- keys ---- */
  const keys = useRef({ selected, confirming, toast, undo, deleteSelected });
  keys.current = { selected, confirming, toast, undo, deleteSelected };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      if (t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      const k = keys.current;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        if (k.toast?.kind === "deleted") {
          e.preventDefault();
          void k.undo();
        }
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key === "Escape") {
        if (k.confirming) setConfirming(false);
        else setSelectedId(null);
      } else if (e.key === "Enter" && k.selected) {
        e.preventDefault();
        if (k.confirming) k.deleteSelected();
        else navigate({ name: "session", id: k.selected.id, t: null });
      } else if (e.key === "Delete" && k.selected && !k.selected.live) {
        setConfirming(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const dismissToast = useCallback(() => setToast(null), []);

  return (
    <div className="flex h-full flex-col overflow-hidden px-8 pb-6 pt-6">
      <div className="flex shrink-0 items-center gap-3.5">
        <div>
          <h1 className="text-[24px] font-bold tracking-[-0.02em]">Library</h1>
          {sessions && (
            <div className="num mt-1 text-[11px] text-ink-3">
              {totals.sessions} {totals.sessions === 1 ? "session" : "sessions"} · {totals.hours} {totals.hours === 1 ? "hour" : "hours"} ·{" "}
              {fmtSize(totals.bytes)} on disk
            </div>
          )}
        </div>
        <div className="flex-1" />
        {sessions && sessions.length > 0 && (
          <div className="flex rounded-[9px] bg-sunk p-0.5">
            {(["days", "list"] as const).map((m) => (
              <button
                key={m}
                className={`h-7 rounded-[7px] px-3 text-[12.5px] font-semibold transition-colors duration-150 ${
                  mode === m ? "bg-card text-ink shadow-[0_1px_2px_rgba(0,0,0,.08)]" : "text-ink-3 hover:text-ink-2"
                }`}
                onClick={() => setMode(m)}
              >
                {m === "days" ? "Days" : "List"}
              </button>
            ))}
          </div>
        )}
      </div>

      {sessionsError && <p className="mt-6 text-[13px] text-rec">Could not list sessions: {sessionsError}</p>}
      {sessions === null && !sessionsError && <p className="mt-6 text-ink-3">Loading sessions</p>}
      {sessions?.length === 0 && (
        <p className="mt-6 max-w-[420px] text-ink-2">Nothing recorded yet. Press Start in the recorder and your day shows up here.</p>
      )}

      {facts.length > 0 && (
        <div className="mt-[22px] flex min-h-0 flex-1 gap-8">
          <div className="flex min-w-0 flex-1 flex-col">
            {shorts.length > 0 && (
              <CleanupBar
                count={shorts.length}
                bytes={shorts.reduce((a, f) => a + f.bytes, 0)}
                busy={busy}
                onClean={() => void trash(shorts, `Deleted ${shorts.length === 1 ? "1 short session" : `${shorts.length} short sessions`}`)}
              />
            )}
            {mode === "days" ? (
              <TimeMap
                facts={facts}
                selectedId={selectedId}
                onSelect={setSelectedId}
                onOpen={(id) => navigate({ name: "session", id, t: null })}
              />
            ) : (
              <SessionList facts={facts} selectedId={selectedId} onSelect={setSelectedId} />
            )}
          </div>
          <Inspector
            facts={selected}
            confirming={confirming}
            busy={busy}
            onAskDelete={() => setConfirming(true)}
            onCancelDelete={() => setConfirming(false)}
            onDelete={deleteSelected}
          />
        </div>
      )}

      <div className="pointer-events-none absolute bottom-3 left-0 right-0 z-30 flex justify-center">
        <Toast toast={toast} onUndo={() => void undo()} onDismiss={dismissToast} />
      </div>
    </div>
  );
}

/* ---- clean-up offer --------------------------------------------------------- */

function CleanupBar({ count, bytes, busy, onClean }: { count: number; bytes: number; busy: boolean; onClean: () => void }) {
  return (
    <div className="mb-3 flex h-[38px] shrink-0 items-center gap-2.5 rounded-[9px] border border-line bg-card pl-3 pr-1.5 text-[12.5px] text-ink-2">
      <span className="text-ink-3">
        <IconInfo size={14} />
      </span>
      <span className="min-w-0 truncate">
        <b className="font-semibold text-ink">
          {count === 1 ? "1 session is under a minute" : `${count} sessions are under a minute`}
        </b>{" "}
        <span className="num text-[11px] text-ink-3">· {fmtSize(bytes)}</span>, most likely a Start pressed twice
      </span>
      <button
        className="ml-auto h-7 shrink-0 rounded-[7px] px-2.5 text-[12px] font-semibold text-ink transition-colors duration-150 hover:bg-sunk disabled:opacity-50"
        disabled={busy}
        onClick={onClean}
      >
        Clean up
      </button>
    </div>
  );
}

/* ---- the time map ----------------------------------------------------------- */

interface MapProps {
  facts: SessionFacts[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onOpen: (id: string) => void;
}

function TimeMap({ facts, selectedId, onSelect, onOpen }: MapProps) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const rows = useMemo(() => layoutDays(facts), [facts]);
  const { h0, h1 } = useMemo(() => hourRange(rows), [rows]);
  const hrsW = Math.max(0, width - LABEL_W);
  const pxPerHour = hrsW / (h1 - h0);
  const step = pxPerHour >= 56 ? 1 : 2;
  const xAt = (hour: number) => LABEL_W + (hour - h0) * pxPerHour;
  const ticks: number[] = [];
  for (let h = Math.ceil(h0 / step) * step; h < h1; h += step) if (h > h0) ticks.push(h);

  // rows with a month header wherever the month changes
  const items: ({ kind: "month"; key: string; label: string } | { kind: "day"; key: string; row: DayRow })[] = [];
  let lastMonth = "";
  for (const r of rows) {
    const d = new Date(r.day);
    const m = `${d.getFullYear()}-${d.getMonth()}`;
    if (m !== lastMonth) {
      lastMonth = m;
      const thisYear = d.getFullYear() === new Date().getFullYear();
      items.push({ kind: "month", key: `m-${m}`, label: thisYear ? MONTHS[d.getMonth()] ?? "" : `${MONTHS[d.getMonth()] ?? ""} ${d.getFullYear()}` });
    }
    items.push({ kind: "day", key: `d-${r.day}`, row: r });
  }

  return (
    <div ref={ref} className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
      {width > 0 && (
        <>
          {/* hour labels, pinned while the days scroll */}
          <div className="sticky top-0 z-10 h-6 bg-paper">
            {ticks.map((h) => (
              <div key={h} className="num absolute top-1 w-10 -translate-x-1/2 text-center text-[10.5px] text-ink-3" style={{ left: xAt(h) }}>
                {hourLabel(h)}
              </div>
            ))}
          </div>
          <div className="relative pb-4">
            {ticks.map((h) => (
              <div key={h} className="pointer-events-none absolute bottom-0 top-0 w-px bg-line-2" style={{ left: xAt(h) }} />
            ))}
            {items.map((it) =>
              it.kind === "month" ? (
                <div key={it.key} className="relative flex items-end pb-1" style={{ height: MONTH_H }}>
                  <span className="bg-paper pr-2 text-[12.5px] font-bold">{it.label}</span>
                </div>
              ) : (
                <div key={it.key} className="relative" style={{ height: ROW_H }}>
                  <div className="absolute left-0 top-[5px] whitespace-nowrap text-[12.5px] text-ink-2">
                    <span className="inline-block w-[30px] text-ink-3">{WD[new Date(it.row.day).getDay()]}</span>
                    <span className="num">{new Date(it.row.day).getDate()}</span>
                  </div>
                  {it.row.pieces.map((p) => (
                    <Block
                      key={`${p.facts.id}-${p.from}`}
                      piece={p}
                      x0={xAt(Math.max(h0, hourOfDay(p.from, it.row.day)))}
                      x1={xAt(Math.min(h1, hourOfDay(p.to, it.row.day)))}
                      selected={p.facts.id === selectedId}
                      onSelect={onSelect}
                      onOpen={onOpen}
                    />
                  ))}
                </div>
              ),
            )}
          </div>
        </>
      )}
    </div>
  );
}

function hourLabel(h: number): string {
  if (h === 12) return "noon";
  if (h === 0 || h === 24) return "12a";
  return h < 12 ? `${h}a` : `${h - 12}p`;
}

function Block({
  piece,
  x0,
  x1,
  selected,
  onSelect,
  onOpen,
}: {
  piece: Piece;
  x0: number;
  x1: number;
  selected: boolean;
  onSelect: (id: string) => void;
  onOpen: (id: string) => void;
}) {
  const f = piece.facts;
  const w = Math.max(4, x1 - x0);
  const span = piece.to - piece.from;
  const at = (t: number) => (span > 0 ? ((t - piece.from) / span) * w : 0);
  const m = f.manifest;
  const cam = f.webcam;
  const shots = !f.short && m && cam ? thumbInstants(piece.from, piece.to, w) : [];
  const n = shots.length;
  const showDot = f.outage !== null && !piece.continues;

  return (
    <>
      <button
        className={`absolute top-[5px] flex h-5 overflow-hidden rounded-[4px] transition-shadow duration-150 ${
          f.short ? "bg-ink-4" : "bg-ink-2"
        } ${selected ? "shadow-[0_0_0_2px_var(--color-paper),0_0_0_4px_var(--color-ink)]" : "hover:shadow-[0_0_0_2px_var(--color-paper),0_0_0_3px_var(--color-ink-3)]"}`}
        style={{ left: x0, width: w }}
        title={`${fmtDayTitle(f.start)} · ${fmtTimeAmPm(f.start)} – ${f.live ? "now" : fmtTimeAmPm(f.end)} · ${fmtSpan(f.lengthMs)}`}
        aria-label={`Session ${fmtDayTitle(f.start)} ${fmtTimeAmPm(f.start)}`}
        aria-pressed={selected}
        onClick={() => onSelect(f.id)}
        onDoubleClick={() => onOpen(f.id)}
      >
        {m &&
          cam &&
          shots.map((t, i) => {
            const shot = nearestFootage(m, cam, t);
            return shot === null ? null : (
              <img
                key={i}
                src={thumbUrl(m.id, cam.id, thumbInstant(shot), 64)}
                alt=""
                draggable={false}
                className="pointer-events-none absolute top-0 h-full object-cover [filter:saturate(.7)_contrast(1.05)]"
                style={{ left: (i * w) / n, width: w / n - (i < n - 1 ? 1 : 0) }}
                onError={(e) => {
                  e.currentTarget.style.visibility = "hidden";
                }}
              />
            );
          })}
        {/* away and paused: a paper veil; cuts: the paper itself */}
        {f.quiet.map((q) => {
          const a = Math.max(0, at(q.from));
          const b = Math.min(w, at(q.to));
          return b - a >= 1 ? <span key={`q${q.from}`} className="pointer-events-none absolute top-0 h-full bg-[rgba(245,243,238,.62)]" style={{ left: a, width: b - a }} /> : null;
        })}
        {f.holes.map((h) => {
          const a = Math.max(0, at(h.from));
          const b = Math.min(w, at(h.to));
          return b - a >= 1 ? <span key={`h${h.from}`} className="pointer-events-none absolute top-0 h-full bg-paper" style={{ left: a, width: b - a }} /> : null;
        })}
        {f.live && !piece.continues && (
          <span className="pointer-events-none absolute bottom-0 right-0 top-0 w-[3px] animate-[rec-breathe_1.6s_ease-in-out_infinite] bg-rec" />
        )}
      </button>
      {showDot && (
        <span
          className="pointer-events-none absolute top-0.5 h-2 w-2 rounded-full bg-[#E39A3B] shadow-[0_0_0_2px_var(--color-paper)]"
          style={{ left: x0 + w - 5 }}
        />
      )}
    </>
  );
}

/* ---- list view ---------------------------------------------------------------- */

function SessionList({ facts, selectedId, onSelect }: { facts: SessionFacts[]; selectedId: string | null; onSelect: (id: string) => void }) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <ul className="overflow-hidden rounded-card border border-line bg-card">
        {facts.map((f, i) => (
          <li key={f.id} className={i ? "border-t border-line-2" : ""}>
            <button
              className={`flex w-full items-center gap-4 px-4 py-2.5 text-left transition-colors duration-150 ${
                f.id === selectedId ? "bg-[rgba(255,216,77,.16)] shadow-[inset_3px_0_0_var(--color-canary-deep)]" : "hover:bg-paper"
              }`}
              onClick={() => onSelect(f.id)}
              onDoubleClick={() => navigate({ name: "session", id: f.id, t: null })}
            >
              <span className="w-[190px] shrink-0 truncate text-[13px] font-semibold">{fmtDayTitle(f.start)}</span>
              <span className="num whitespace-nowrap text-[11.5px] text-ink-3">
                {fmtTimeAmPm(f.start)} – {f.live ? "now" : fmtTimeAmPm(f.end)}
              </span>
              {f.live && (
                <span className="flex items-center gap-1.5 text-[11.5px] font-semibold text-rec">
                  <span className="dot-active h-1.5 w-1.5 rounded-full bg-rec" />
                  Recording
                </span>
              )}
              {f.outage && <span className="h-2 w-2 shrink-0 rounded-full bg-[#E39A3B]" title="A screen stopped recording" />}
              {f.marks.length > 0 && (
                <span className="flex items-center gap-1 text-[11.5px] text-ink-3">
                  <span className="h-[7px] w-[7px] rounded-[2px] bg-canary-deep" />
                  <span className="num">{f.marks.length}</span>
                </span>
              )}
              <span className="num ml-auto whitespace-nowrap text-[11.5px] text-ink-2">{fmtSpan(f.lengthMs)}</span>
              <span className="num w-[72px] text-right text-[11.5px] text-ink-3">{fmtSize(f.bytes)}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/* ---- inspector ---------------------------------------------------------------- */

interface InspectorProps {
  facts: SessionFacts | null;
  confirming: boolean;
  busy: boolean;
  onAskDelete: () => void;
  onCancelDelete: () => void;
  onDelete: () => void;
}

function Inspector({ facts: f, confirming, busy, onAskDelete, onCancelDelete, onDelete }: InspectorProps) {
  if (!f) {
    return (
      <aside className="grid shrink-0 place-items-center rounded-card border border-line bg-card px-8 text-center text-[12.5px] text-ink-3" style={{ width: INSPECTOR_W }}>
        Click a session on the map to see it here. Double-click to play it.
      </aside>
    );
  }
  const m = f.manifest;
  return (
    <aside className="flex shrink-0 flex-col overflow-hidden rounded-card border border-line bg-card" style={{ width: INSPECTOR_W }}>
      <Hero facts={f} />
      <div className="flex min-h-0 flex-1 flex-col px-[18px] pb-4 pt-4">
        <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
          <h2 className="text-[17px] font-bold tracking-[-0.01em]">{fmtDayTitle(f.start)}</h2>
          <div className="num mt-1 text-[11.5px] text-ink-3">
            {fmtTimeAmPm(f.start)} – {f.live ? "now" : fmtTimeAmPm(f.end)}
            {dayStart(f.end - 1) !== dayStart(f.start) && !f.live ? " next day" : ""}
          </div>
          <div className="mt-4 grid grid-cols-3 gap-2.5">
            <Stat value={fmtSpan(f.lengthMs)} label="long" />
            <Stat value={fmtSize(f.bytes)} label="on disk" />
            <Stat value={m ? String(m.tracks.length) : "–"} label="tracks" />
          </div>
          {f.live && (
            <div className="mt-4 flex items-center gap-2 rounded-[9px] border border-rec-line px-3 py-2 text-[12px] font-semibold text-rec">
              <span className="dot-active h-2 w-2 rounded-full bg-rec" />
              Recording now
            </div>
          )}
          {f.outage && m && <Health facts={f} manifest={m} />}
          {m && <CameraStrip facts={f} manifest={m} />}
          {m && <TrackRows manifest={m} />}
          {f.marks.length > 0 && <MarkRows facts={f} />}
        </div>

        {confirming ? (
          <div className="mt-3 rounded-[9px] border border-rec-line bg-[#FFF6F5] p-3">
            <div className="text-[12.5px] leading-[1.45] text-ink">
              <b className="font-semibold">Delete this session?</b> Frees <span className="num">{fmtSize(f.bytes)}</span> now. Undo for 10 seconds.
            </div>
            <div className="mt-2.5 flex justify-end gap-2">
              <button className="h-8 rounded-ctl border border-line bg-card px-3 text-[12.5px] font-semibold" onClick={onCancelDelete}>
                Cancel <span className="kbd ml-1">Esc</span>
              </button>
              <button
                className="h-8 rounded-ctl bg-rec px-3 text-[12.5px] font-semibold text-white transition-opacity duration-150 hover:opacity-90 disabled:opacity-50"
                disabled={busy}
                onClick={onDelete}
              >
                Delete <span className="kbd ml-1 !border-white/30 !bg-transparent !text-white/80">↵</span>
              </button>
            </div>
          </div>
        ) : (
          <div className="mt-3 flex gap-2">
            <a
              href={href({ name: "session", id: f.id, t: null })}
              className="flex h-[34px] flex-1 items-center justify-center rounded-[9px] bg-ink text-[13px] font-semibold text-white transition-opacity duration-150 hover:opacity-90"
            >
              Open
            </a>
            <a
              href={href({ name: "export", id: f.id, from: null, to: null })}
              className="flex h-[34px] items-center justify-center gap-[7px] rounded-[9px] border border-line bg-card px-3.5 text-[13px] font-semibold transition-colors duration-150 hover:bg-paper"
            >
              <IconExport size={14} />
              Export
            </a>
            <button
              className="grid h-[34px] w-[34px] place-items-center rounded-[9px] border border-line bg-card text-ink-3 transition-colors duration-150 hover:border-rec-line hover:text-rec disabled:opacity-40 disabled:hover:border-line disabled:hover:text-ink-3"
              title={f.live ? "Stop recording to delete this session" : "Delete session"}
              aria-label="Delete session"
              disabled={f.live || busy}
              onClick={onAskDelete}
            >
              <IconTrash size={15} />
            </button>
          </div>
        )}
      </div>
    </aside>
  );
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div>
      <div className="num text-[15px] font-semibold">{value}</div>
      <div className="mt-0.5 text-[11.5px] text-ink-3">{label}</div>
    </div>
  );
}

function Hero({ facts: f }: { facts: SessionFacts }) {
  const m = f.manifest;
  const screen = m ? heroScreen(m) : null;
  const mid = f.start + (f.end - f.start) / 2;
  const sAt = m && screen ? nearestFootage(m, screen, mid) : null;
  const cAt = m && f.webcam ? nearestFootage(m, f.webcam, sAt ?? mid) : null;
  return (
    <div className="relative h-[130px] shrink-0 bg-mat">
      {m && screen && sAt !== null && (
        <img
          key={`${m.id}-s`}
          src={thumbUrl(m.id, screen.id, thumbInstant(sAt), 640)}
          alt=""
          className="h-full w-full object-cover opacity-95"
          onError={(e) => {
            e.currentTarget.style.visibility = "hidden";
          }}
        />
      )}
      {m && f.webcam && cAt !== null && (
        <img
          key={`${m.id}-c`}
          src={thumbUrl(m.id, f.webcam.id, thumbInstant(cAt), 192)}
          alt=""
          className="absolute bottom-2.5 right-2.5 h-[52px] w-[92px] rounded-[5px] object-cover shadow-[0_0_0_2px_rgba(0,0,0,.5)]"
          onError={(e) => {
            e.currentTarget.style.visibility = "hidden";
          }}
        />
      )}
    </div>
  );
}

function Health({ facts: f, manifest: m }: { facts: SessionFacts; manifest: SessionManifest }) {
  const o = f.outage;
  if (!o) return null;
  const end = f.end;
  // how much of the session after the stop the screens missed
  const lost = end - o.at;
  const lead = o.screens.length === 1 ? `Screen ${o.screens[0]} stopped at ${fmtTimeAmPm(o.at)}.` : `Screens stopped at ${fmtTimeAmPm(o.at)}.`;
  const back = m.tracks.filter((t) => t.kind === "screen").some((t) => t.segments.some((s) => s.start_epoch_ms > o.at + 60_000));
  return (
    <div className="mt-4 flex gap-2 rounded-[9px] border border-amber-line bg-amber-wash px-3 py-2.5 text-[12px] leading-[1.45] text-[#7A4A12]">
      <span className="mt-px shrink-0 text-amber">
        <IconWarn size={13} />
      </span>
      <span>
        <b className="font-semibold text-[#8A4A05]">{lead}</b>{" "}
        {back ? "They came back later; the Player shows where." : `The last ${fmtSpan(lost)} ${f.webcam ? "are camera and sound only" : "have no screens"}.`}
      </span>
    </div>
  );
}

function CameraStrip({ facts: f, manifest: m }: { facts: SessionFacts; manifest: SessionManifest }) {
  const cam = f.webcam;
  if (!cam || f.end - f.start < 60_000) return null;
  const n = 6;
  const shots = Array.from({ length: n }, (_, i) => nearestFootage(m, cam, f.start + ((i + 0.5) / n) * (f.end - f.start)));
  return (
    <div className="mt-4">
      <div className="flex gap-0.5 overflow-hidden rounded-[6px]">
        {shots.map((t, i) => (
          <div key={i} className="h-[34px] min-w-0 flex-1 bg-sunk">
            {t !== null && (
              <img
                src={thumbUrl(m.id, cam.id, thumbInstant(t), 96)}
                alt=""
                className="h-full w-full object-cover"
                onError={(e) => {
                  e.currentTarget.style.visibility = "hidden";
                }}
              />
            )}
          </div>
        ))}
      </div>
      <div className="num mt-[5px] flex justify-between text-[10px] text-ink-3">
        <span>{fmtTick(f.start)}</span>
        <span>{f.live ? "now" : fmtTick(f.end)}</span>
      </div>
    </div>
  );
}

function TrackRows({ manifest: m }: { manifest: SessionManifest }) {
  return (
    <div className="mt-4 border-t border-line-2">
      {m.tracks.map((t) => {
        const failures = t.gaps.filter((g) => !isExpectedGap(g.reason) && g.to_epoch_ms - g.from_epoch_ms > 60_000).length;
        const bad = t.status === "failed" || failures > 0;
        const video = t.kind === "screen" || t.kind === "webcam";
        const name = t.kind === "screen" ? `Screen ${screenNumber(t)}` : t.kind === "webcam" ? "Camera" : t.kind === "mic" ? "Mic" : "System audio";
        return (
          <div key={t.id} className="flex h-[30px] items-center gap-2 border-b border-line-2 text-[12px]">
            <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${bad ? "bg-[#E39A3B]" : "bg-ink-4"}`} />
            <span className="shrink-0 font-semibold">{name}</span>
            <span className="num min-w-0 truncate text-[11px] text-ink-3">{video ? `${t.width}×${t.height} · ${t.codec}` : t.codec}</span>
            <span className="num ml-auto shrink-0 text-[11px] text-ink-2">{fmtSpan(footageMs(m, t))}</span>
          </div>
        );
      })}
    </div>
  );
}

function MarkRows({ facts: f }: { facts: SessionFacts }) {
  return (
    <div className="mt-4">
      <div className="text-[12px] font-semibold text-ink-2">Marks</div>
      <div className="mt-1">
        {f.marks.map((mk) => (
          <a
            key={mk.epoch_ms}
            href={href({ name: "session", id: f.id, t: mk.epoch_ms })}
            className="-mx-1.5 flex h-7 items-center gap-2 rounded-[6px] px-1.5 text-[12px] transition-colors duration-150 hover:bg-paper"
          >
            <span className="h-[7px] w-[7px] shrink-0 rounded-[2px] bg-canary-deep" />
            <span className="num shrink-0 text-[11px] text-ink-3">{fmtTimeAmPm(mk.epoch_ms)}</span>
            <span className={`min-w-0 truncate ${mk.label.trim() ? "text-ink" : "text-ink-4"}`}>{mk.label.trim() || "No note"}</span>
          </a>
        ))}
      </div>
    </div>
  );
}
