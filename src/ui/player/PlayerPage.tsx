/**
 * `#/session/<id>?t=<epoch_ms>`: one session, played back in sync, with the day
 * log, the day ribbon and the delete flow.
 *
 * The engine is rebuilt whenever the manifest is re-read (after a delete, an
 * undo, a finalize); the playhead, play state, speed and sound carry over.
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { SessionManifest } from "../types";
import { addMarker, deleteRange, fetchSession, undoCut } from "../hooks/useApi";
import { SyncEngine, isVideoTrack, sessionBounds } from "../session/engine";
import { collapse, sessionHoles } from "../session/cuts";
import { fmtSize, fmtSpan } from "../session/format";
import { errorText, useAppData } from "../app/appData";
import { navigate } from "../app/router";
import { Header } from "./Header";
import { Stage } from "./Stage";
import type { FollowMode, View } from "./Stage";
import { useCursorFollow } from "./useFollow";
import { LogCard } from "./LogCard";
import { ConfirmRow, SelectionBar, Transport } from "./Transport";
import { Ribbon, placeZoom } from "./Ribbon";
import type { Sel, SelPhase } from "./Ribbon";
import { Toast } from "./Toast";
import type { ToastState } from "./Toast";
import { makeAxis, quietSpans, rangeBytes, screenOutages, screenTracks, sessionTotals, snapTargets } from "./timeline";

const WIDE_PX = 1600;
const COLLAPSE_MS = 240;
/** the backend finalizes ~12 s after a delete; re-read just after */
const FINALIZE_REREAD_MS = 13_500;
const DEFAULT_ZOOM_MS = 2 * 3_600_000;

interface Props {
  id: string;
  initialT: number | null;
}

export function PlayerPage({ id, initialT }: Props) {
  const [manifest, setManifest] = useState<SessionManifest | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    fetchSession(id)
      .then((m) => alive && setManifest(m))
      .catch((e: unknown) => alive && setLoadError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [id]);

  if (loadError) {
    return (
      <div className="px-8 pt-6">
        <h1 className="text-[20px] font-bold tracking-[-0.02em]">This session could not be opened</h1>
        <p className="mt-2 max-w-[560px] text-ink-2">{loadError}</p>
        <a className="mt-4 inline-block font-semibold text-ink underline" href="#/library">
          Back to the Library
        </a>
      </div>
    );
  }
  if (!manifest) return <div className="px-8 pt-6 text-ink-3">Opening the session</div>;
  return <Player manifest={manifest} setManifest={setManifest} initialT={initialT} />;
}

function useContentWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) setW(r.width);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

function useWindowWide(): boolean {
  const [wide, setWide] = useState(() => window.innerWidth >= WIDE_PX);
  useEffect(() => {
    const on = () => setWide(window.innerWidth >= WIDE_PX);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return wide;
}

function Player({
  manifest,
  setManifest,
  initialT,
}: {
  manifest: SessionManifest;
  setManifest: (m: SessionManifest) => void;
  initialT: number | null;
}) {
  const id = manifest.id;
  const { status, refreshSessions, patchMark } = useAppData();
  const live = status?.recording === true && status.session_id === id;
  const wide = useWindowWide();
  const [contentRef, contentW] = useContentWidth();

  /* ---- the engine, rebuilt per manifest with the transport carried over ---- */
  const carry = useRef({ t: initialT, playing: false, rate: 1 });
  // Sound on by default: mic and system audio together, as the transport's "All".
  const [audioIds, setAudioIds] = useState<string[]>(() => manifest.tracks.filter((t) => !isVideoTrack(t)).map((t) => t.id));
  const engine = useMemo(() => new SyncEngine(manifest), [manifest]);
  useEffect(() => {
    const c = carry.current;
    engine.setRate(c.rate);
    if (c.t !== null && Number.isFinite(c.t)) engine.seekTo(c.t);
    engine.startLoop();
    if (import.meta.env.DEV) window.__chirpEngine = engine;
    if (c.playing) engine.setPlaying(true);
    const off = engine.subscribeFrame((s) => {
      carry.current = { t: s.masterEpochMs, playing: s.playing, rate: s.rate };
    });
    return () => {
      off();
      engine.stopLoop();
    };
  }, [engine]);
  useEffect(() => {
    engine.setAudioTracks(audioIds);
  }, [engine, audioIds]);

  /* ---- facts ---- */
  const holes = useMemo(() => sessionHoles(manifest), [manifest]);
  const bounds = useMemo(() => sessionBounds(manifest), [manifest]);
  const axis = useMemo(() => makeAxis(holes, bounds.start, bounds.end), [holes, bounds]);
  const totals = useMemo(() => sessionTotals(manifest), [manifest]);
  const idle = useMemo(() => quietSpans(manifest), [manifest]);
  const outages = useMemo(() => screenOutages(manifest), [manifest]);
  const targets = useMemo(() => snapTargets(manifest), [manifest]);
  const screens = useMemo(() => screenTracks(manifest), [manifest]);
  const webcam = manifest.tracks.find((t) => t.kind === "webcam") ?? null;

  /* ---- view ---- */
  const [view, setView] = useState<View>("focus");
  // Focus defaults to screen-0, else the largest screen by pixel count.
  const [focusedId, setFocusedId] = useState<string | null>(() => {
    const s0 = screens.find((t) => t.id === "screen-0");
    if (s0) return s0.id;
    const big = [...screens].sort((a, b) => (b.monitor ? b.monitor.w * b.monitor.h : b.width * b.height) - (a.monitor ? a.monitor.w * a.monitor.h : a.width * a.height))[0];
    return big?.id ?? null;
  });
  // Focus follows the mouse when the session has a cursor track; 1–9 (or a
  // click on a screen) pins one, F or 0 goes back to following.
  const [follow, setFollow] = useState(true);
  const cursor = useCursorFollow(engine, id, screens, focusedId);
  const followMode: FollowMode = !cursor.hasData ? "none" : follow ? "on" : "pinned";
  const shownId = followMode === "on" && cursor.followedId ? cursor.followedId : focusedId;
  const focused = screens.find((t) => t.id === shownId) ?? screens[0] ?? webcam;
  const pin = useCallback((fid: string) => {
    setFocusedId(fid);
    setFollow(false);
    setView("focus");
  }, []);

  /* ---- mark notes: edited in place, without rebuilding the engine ---- */
  const [notes, setNotes] = useState<ReadonlyMap<number, string>>(() => new Map());
  useEffect(() => setNotes(new Map()), [manifest]);
  const logManifest = useMemo(
    () => (notes.size ? { ...manifest, markers: manifest.markers.map((mk) => ({ ...mk, label: notes.get(mk.epoch_ms) ?? mk.label })) } : manifest),
    [manifest, notes],
  );
  const onNoteSaved = useCallback(
    (epochMs: number, label: string) => {
      setNotes((prev) => new Map(prev).set(epochMs, label));
      patchMark(id, epochMs, label);
    },
    [id, patchMark],
  );

  /* ---- zoom bracket: default 2 h around the playhead, at most 40% of a short day ---- */
  const [zoom, setZoom] = useState(() => {
    const len = Math.min(DEFAULT_ZOOM_MS, axis.span * 0.4);
    const start = initialT ?? bounds.start;
    const z0 = makeAxis(holes, bounds.start, bounds.end);
    return placeZoom(z0, start, { from: bounds.start, to: Math.min(bounds.end, bounds.start + len) }, 0.5);
  });
  // A cut can swallow part of the bracket; keep it inside the session.
  useEffect(() => {
    setZoom((z) => ({ from: Math.max(bounds.start, Math.min(z.from, bounds.end - 60_000)), to: Math.min(bounds.end, Math.max(z.to, bounds.start + 60_000)) }));
  }, [bounds]);

  /* ---- selection ---- */
  const [sel, setSel] = useState<Sel | null>(null);
  const [phase, setPhase] = useState<SelPhase>("select");
  const [selTracks, setSelTracks] = useState<string[] | null>(null);
  const [edge, setEdge] = useState<number | null>(null);
  const [snapHint, setSnapHint] = useState<{ edge: "from" | "to"; kind: NonNullable<Sel["snapFrom"]> } | null>(null);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);

  const zoomToFit = useCallback(
    (s: { from: number; to: number }) => {
      const pad = (collapse(holes, s.to) - collapse(holes, s.from)) * 0.25 + 60_000;
      setZoom({ from: Math.max(bounds.start, s.from - pad), to: Math.min(bounds.end, s.to + pad) });
    },
    [holes, bounds],
  );

  const choose = useCallback(
    (s: Sel, done: boolean, source: "day" | "zoom" | "other") => {
      setSel(s);
      setPhase("select");
      setEdge(null);
      const hint =
        s.snapFrom && s.snapFrom !== "segment"
          ? { edge: "from" as const, kind: s.snapFrom }
          : s.snapTo && s.snapTo !== "segment"
            ? { edge: "to" as const, kind: s.snapTo }
            : s.snapFrom
              ? { edge: "from" as const, kind: s.snapFrom }
              : s.snapTo
                ? { edge: "to" as const, kind: s.snapTo }
                : null;
      setSnapHint(hint);
      if (done && source !== "zoom") zoomToFit(s);
    },
    [zoomToFit],
  );

  const clearSel = useCallback(() => {
    setSel(null);
    setPhase("select");
    setSnapHint(null);
    setEdge(null);
  }, []);

  const seek = useCallback((t: number) => engine.seekTo(t), [engine]);

  const selLength = sel ? collapse(holes, sel.to) - collapse(holes, sel.from) : 0;
  const selBytes = useMemo(() => (sel ? rangeBytes(manifest, sel.from, sel.to, selTracks ?? undefined) : 0), [manifest, sel, selTracks]);

  /* ---- delete, undo, finalize ---- */
  const reread = useCallback(async () => {
    try {
      setManifest(await fetchSession(id));
    } catch (e) {
      setToast({ kind: "message", id: Date.now(), title: "Could not re-read this session", detail: errorText(e), tone: "error" });
    }
  }, [id, setManifest]);

  const finalizeTimer = useRef(0);
  useEffect(() => () => window.clearTimeout(finalizeTimer.current), []);

  const doDelete = useCallback(async () => {
    if (!sel || busy) return;
    setBusy(true);
    try {
      const receipt = await deleteRange(id, Math.round(sel.from), Math.round(sel.to), selTracks ?? undefined);
      // the range folds away, then the seam takes its place
      setPhase("collapsing");
      await new Promise((r) => window.setTimeout(r, COLLAPSE_MS));
      const m = await fetchSession(id);
      setManifest(m);
      clearSel();
      setSelTracks(null);
      setToast({
        kind: "deleted",
        cutId: receipt.cut.id,
        title: `Deleted ${fmtSpan(selLength)}`,
        detail: `freed ${fmtSize(receipt.freed_bytes_estimate)}`,
        startedAt: Date.now(),
        until: receipt.undo_until_epoch_ms,
        undoing: false,
      });
      window.clearTimeout(finalizeTimer.current);
      finalizeTimer.current = window.setTimeout(() => void reread(), FINALIZE_REREAD_MS);
      void refreshSessions();
    } catch (e) {
      setPhase("confirm");
      setToast({ kind: "message", id: Date.now(), title: "Nothing was deleted", detail: errorText(e), tone: "error" });
    } finally {
      setBusy(false);
    }
  }, [sel, busy, id, selTracks, selLength, setManifest, clearSel, reread, refreshSessions]);

  const doUndo = useCallback(async () => {
    if (!toast || toast.kind !== "deleted" || toast.undoing) return;
    setToast({ ...toast, undoing: true });
    try {
      const m = await undoCut(id, toast.cutId);
      window.clearTimeout(finalizeTimer.current);
      setManifest(m);
      setToast({ kind: "message", id: Date.now(), title: `Restored ${toast.title.replace(/^Deleted /, "")}`, detail: null, tone: "info" });
      void refreshSessions();
    } catch (e) {
      setToast({ kind: "message", id: Date.now(), title: "Could not undo", detail: errorText(e), tone: "error" });
    }
  }, [toast, id, setManifest, refreshSessions]);

  const doMark = useCallback(async () => {
    if (!live) return;
    try {
      await addMarker("");
      await reread();
    } catch (e) {
      setToast({ kind: "message", id: Date.now(), title: "Could not add a mark", detail: errorText(e), tone: "error" });
    }
  }, [live, reread]);

  const goExport = useCallback(() => {
    navigate({ name: "export", id, from: sel ? sel.from : null, to: sel ? sel.to : null });
  }, [id, sel]);

  const selectBefore = useCallback(() => {
    const t = engine.masterEpochMs;
    if (t - bounds.start < 1000) return;
    setSel({ from: bounds.start, to: t, snapFrom: "start", snapTo: null });
    setSnapHint(null);
    setPhase("confirm");
  }, [engine, bounds]);
  const selectAfter = useCallback(() => {
    const t = engine.masterEpochMs;
    if (bounds.end - t < 1000) return;
    setSel({ from: t, to: bounds.end, snapFrom: null, snapTo: "end" });
    setSnapHint(null);
    setPhase("confirm");
  }, [engine, bounds]);

  /** The Select button: one minute from the playhead (or up to the end), its edges ready to drag. */
  const selectHere = useCallback(() => {
    const t = engine.masterEpochMs;
    const to = Math.min(bounds.end, t + 60_000);
    const from = to - t < 5_000 ? Math.max(bounds.start, bounds.end - 60_000) : t;
    if (to - from < 1000) return;
    choose({ from, to, snapFrom: null, snapTo: null }, true, "other");
  }, [engine, bounds, choose]);

  /* ---- keys ---- */
  const keyState = useRef({ sel, phase, edge, toast, screens, view, live });
  keyState.current = { sel, phase, edge, toast, screens, view, live };
  const actions = useRef({ doDelete, doUndo, doMark, goExport, clearSel, choose, pin });
  actions.current = { doDelete, doUndo, doMark, goExport, clearSel, choose, pin };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target;
      if (target instanceof HTMLElement && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) return;
      const k = keyState.current;
      const a = actions.current;
      const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;

      if ((e.ctrlKey || e.metaKey) && key === "z") {
        if (k.toast?.kind === "deleted") {
          e.preventDefault();
          void a.doUndo();
        }
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;

      if (k.phase === "confirm" && k.sel) {
        if (key === "Enter") {
          e.preventDefault();
          void a.doDelete();
        } else if (key === "Escape") {
          e.preventDefault();
          setPhase("select");
        }
        return;
      }
      if (k.phase === "collapsing") return;

      switch (key) {
        case " ":
          e.preventDefault();
          engine.togglePlaying();
          break;
        case "ArrowLeft":
          e.preventDefault();
          engine.nudgeSeconds(e.shiftKey ? -60 : -5);
          break;
        case "ArrowRight":
          e.preventDefault();
          engine.nudgeSeconds(e.shiftKey ? 60 : 5);
          break;
        case "d":
          setView((v) => (v === "desk" ? "focus" : "desk"));
          break;
        case "Escape":
          if (k.sel || k.edge !== null) a.clearSel();
          break;
        case "i":
        case "o": {
          const t = engine.masterEpochMs;
          const other = k.sel ? (key === "i" ? k.sel.to : k.sel.from) : k.edge;
          if (other === null) {
            setEdge(t);
            break;
          }
          const from = key === "i" ? t : other;
          const to = key === "i" ? other : t;
          if (to - from < 1000) {
            setSel(null);
            setEdge(t);
            break;
          }
          a.choose({ from, to, snapFrom: null, snapTo: null }, true, "other");
          break;
        }
        case "x":
          if (k.sel) setPhase("confirm");
          break;
        case "e":
          a.goExport();
          break;
        case "m":
          if (k.live) void a.doMark();
          break;
        case "f":
        case "0":
          setFollow(true);
          setView("focus");
          break;
        default:
          if (/^[1-9]$/.test(key)) {
            const s = k.screens[Number(key) - 1];
            if (s) a.pin(s.id);
          }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [engine]);

  const dismissToast = useCallback(() => setToast(null), []);
  const logW = wide ? 400 : 300;

  return (
    <div className="flex h-full flex-col overflow-hidden px-6 pb-4">
      <Header manifest={manifest} live={live} totals={totals} wide={wide} onSeek={seek} onExport={goExport} />

      <div className="flex min-h-0 flex-1 gap-5 pt-2">
        <div className="flex min-w-0 flex-1 flex-col">
          <Stage
            engine={engine}
            manifest={manifest}
            view={view}
            focusedId={focused?.id ?? null}
            wide={wide}
            follow={followMode}
            onFocus={pin}
            onView={setView}
          />
          <div className="mt-3">
            {sel && phase !== "select" ? (
              <ConfirmRow lengthMs={selLength} bytes={selBytes} busy={busy || phase === "collapsing"} onCancel={() => setPhase("select")} onConfirm={() => void doDelete()} />
            ) : sel ? (
              <SelectionBar
                manifest={manifest}
                from={sel.from}
                to={sel.to}
                lengthMs={selLength}
                bytes={selBytes}
                tracks={selTracks}
                onTracks={setSelTracks}
                onExport={goExport}
                onDelete={() => setPhase("confirm")}
                onDeleteBefore={selectBefore}
                onDeleteAfter={selectAfter}
                onClear={clearSel}
              />
            ) : (
              <Transport engine={engine} axis={axis} manifest={manifest} live={live} audioIds={audioIds} onAudio={setAudioIds} onMark={() => void doMark()} onSelect={selectHere} />
            )}
          </div>
        </div>
        <LogCard
          engine={engine}
          manifest={logManifest}
          live={live}
          width={logW}
          onSeek={seek}
          onSelectRange={(from, to) => {
            // show the start of what would go, as the day ribbon does
            seek(from);
            choose({ from, to, snapFrom: "gap", snapTo: "gap" }, true, "other");
          }}
          onNoteSaved={onNoteSaved}
        />
      </div>

      <div ref={contentRef} className={`shrink-0 ${wide ? "mt-[22px]" : "mt-4"}`}>
        {contentW > 0 && (
          <Ribbon
            engine={engine}
            manifest={manifest}
            axis={axis}
            holes={holes}
            idle={idle}
            outages={outages}
            zoom={zoom}
            onZoom={setZoom}
            sel={sel}
            phase={phase}
            edge={edge}
            snapHint={snapHint}
            targets={targets}
            focused={focused}
            webcam={webcam}
            width={contentW}
            wide={wide}
            onSeek={seek}
            onSelect={choose}
          />
        )}
      </div>

      <div className="pointer-events-none absolute bottom-2 left-0 right-0 z-30 flex justify-center">
        <Toast toast={toast} onUndo={() => void doUndo()} onDismiss={dismissToast} />
      </div>
    </div>
  );
}
