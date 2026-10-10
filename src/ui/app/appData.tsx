/**
 * What every page of the app window shares: the session list, each session's
 * manifest, the recorder's status, and the disk. One provider polls; pages read
 * and ask for refreshes.
 *
 * Manifests load in the background, one at a time, and are kept until the
 * session's summary changes (a delete, an undo, a finalize, more footage), so
 * the Library and Marks read them without asking again.
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { EnvironmentInfo, SessionManifest, SessionStatus, SessionSummary, SessionTrashReceipt } from "../types";
import { listen } from "@tauri-apps/api/event";
import { countExports, fetchSession, getEnvironment, getSessionStatus, listSessions, undoDeleteSession } from "../hooks/useApi";
import { EXPORTS_CHANGED } from "../export/api";
import { getRecorderControl, resumeSession, startRecording, stopRecording, type RecorderControl } from "../recorder/api";

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return "Something went wrong";
}

interface AppData {
  sessions: SessionSummary[] | null;
  sessionsError: string | null;
  refreshSessions: () => Promise<void>;
  /** every loaded manifest by session id; fills in as they arrive */
  manifests: ReadonlyMap<string, SessionManifest>;
  /** true once every listed session's manifest has been tried */
  manifestsReady: boolean;
  /** a note was saved: patch the cached manifest without reading it again */
  patchMark: (sessionId: string, epochMs: number, label: string) => void;
  /** markers across every session, or null while counting */
  markCount: number | null;
  /** finished exports, or null when unknown */
  exportCount: number | null;
  status: SessionStatus | null;
  env: EnvironmentInfo | null;
  /** when the open session was paused, or null when it is not paused */
  pausedSince: number | null;
  recorderBusy: boolean;
  recorderError: string | null;
  /** set when Stop discarded a session under a minute; it can still be undone */
  discarded: SessionTrashReceipt | null;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  resume: () => Promise<void>;
  undoDiscard: () => Promise<void>;
}

const Ctx = createContext<AppData | null>(null);

export function useAppData(): AppData {
  const v = useContext(Ctx);
  if (!v) throw new Error("useAppData outside AppDataProvider");
  return v;
}

const STATUS_POLL_MS = 2_000;

/** What changes about a session when its manifest does. */
function summarySig(s: SessionSummary): string {
  return `${s.ended_epoch_ms}:${s.segment_count}:${s.bytes}:${s.duration_ms}`;
}

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [sessionsError, setSessionsError] = useState<string | null>(null);
  const [status, setStatus] = useState<SessionStatus | null>(null);
  const [env, setEnv] = useState<EnvironmentInfo | null>(null);
  const [recorderBusy, setRecorderBusy] = useState(false);
  const [recorderError, setRecorderError] = useState<string | null>(null);
  const [discarded, setDiscarded] = useState<SessionTrashReceipt | null>(null);
  const [manifests, setManifests] = useState<ReadonlyMap<string, SessionManifest>>(() => new Map());
  const [manifestsReady, setManifestsReady] = useState(false);
  /** the summary each cached manifest was read against */
  const readAgainst = useRef(new Map<string, string>());

  const refreshSessions = useCallback(async () => {
    try {
      const list = await listSessions();
      setSessions([...list].sort((a, b) => b.started_epoch_ms - a.started_epoch_ms));
      setSessionsError(null);
    } catch (e) {
      setSessionsError(errorText(e));
    }
  }, []);

  const refreshEnv = useCallback(async () => {
    try {
      setEnv(await getEnvironment());
    } catch {
      setEnv(null);
    }
  }, []);

  useEffect(() => {
    void refreshSessions();
    void refreshEnv();
  }, [refreshSessions, refreshEnv]);

  // Recorder status: polled, so a start or stop from the widget shows up here,
  // and read again at once whenever the recorder announces a change.
  const wasRecording = useRef<boolean | null>(null);
  const [pausedSince, setPausedSince] = useState<number | null>(null);
  const applyControl = (c: RecorderControl | null) => setPausedSince(c?.recording ? c.paused_since_epoch_ms : null);
  useEffect(() => {
    let alive = true;
    const poll = async () => {
      try {
        const [s, c] = await Promise.all([getSessionStatus(), getRecorderControl().catch(() => null)]);
        if (!alive) return;
        setStatus(s);
        applyControl(c);
        if (wasRecording.current !== null && wasRecording.current !== s.recording) {
          void refreshSessions();
          void refreshEnv();
        }
        wasRecording.current = s.recording;
      } catch {
        if (alive) setStatus(null);
      }
    };
    void poll();
    const t = window.setInterval(poll, STATUS_POLL_MS);
    const off = listen<RecorderControl>("recorder-changed", (e) => {
      if (!alive) return;
      applyControl(e.payload);
      void poll();
    });
    return () => {
      alive = false;
      window.clearInterval(t);
      void off.then((f) => f()).catch(() => {});
    };
  }, [refreshSessions, refreshEnv]);

  // Load manifests in the background, newest first, one at a time. A cached one
  // is kept while its summary is unchanged; a live one is read again each time.
  useEffect(() => {
    if (!sessions) return;
    let alive = true;
    const listed = new Set(sessions.map((s) => s.id));
    setManifests((prev) => {
      const gone = [...prev.keys()].filter((id) => !listed.has(id));
      if (!gone.length) return prev;
      const next = new Map(prev);
      for (const id of gone) {
        next.delete(id);
        readAgainst.current.delete(id);
      }
      return next;
    });
    void (async () => {
      for (const s of sessions) {
        if (!s.has_manifest) continue;
        const sig = summarySig(s);
        if (s.ended_epoch_ms !== null && readAgainst.current.get(s.id) === sig) continue;
        try {
          const m = await fetchSession(s.id);
          if (!alive) return;
          readAgainst.current.set(s.id, sig);
          setManifests((prev) => new Map(prev).set(s.id, m));
        } catch {
          // an unreadable manifest: the Library falls back to the summary
        }
        if (!alive) return;
      }
      if (alive) setManifestsReady(true);
    })();
    return () => {
      alive = false;
    };
  }, [sessions]);

  // The Exports count: read at start, whenever an export finishes, and when the
  // window comes back into focus, and when the Exports page adds or deletes one.
  const [exportCount, setExportCount] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    const read = () =>
      countExports()
        .then((n) => alive && setExportCount(n))
        .catch(() => alive && setExportCount(null));
    void read();
    const onHash = () => void read();
    window.addEventListener("hashchange", onHash);
    window.addEventListener("focus", onHash);
    window.addEventListener(EXPORTS_CHANGED, onHash);
    const off = listen<{ state?: unknown }>("export-progress", (e) => {
      if (e.payload.state === "done") void read();
    });
    return () => {
      alive = false;
      window.removeEventListener("hashchange", onHash);
      window.removeEventListener("focus", onHash);
      window.removeEventListener(EXPORTS_CHANGED, onHash);
      void off.then((f) => f()).catch(() => {});
    };
  }, []);

  const markCount = useMemo(() => {
    if (!manifestsReady) return null;
    let n = 0;
    for (const m of manifests.values()) n += m.markers.length;
    return n;
  }, [manifests, manifestsReady]);

  const patchMark = useCallback((sessionId: string, epochMs: number, label: string) => {
    setManifests((prev) => {
      const m = prev.get(sessionId);
      if (!m) return prev;
      const markers = m.markers.map((mk) => (mk.epoch_ms === epochMs ? { ...mk, label } : mk));
      return new Map(prev).set(sessionId, { ...m, markers });
    });
  }, []);

  const start = useCallback(async () => {
    setRecorderBusy(true);
    setRecorderError(null);
    try {
      setDiscarded(null);
      setStatus(await startRecording());
      setPausedSince(null);
      wasRecording.current = true;
      void refreshSessions();
    } catch (e) {
      setRecorderError(errorText(e));
    } finally {
      setRecorderBusy(false);
    }
  }, [refreshSessions]);

  const stop = useCallback(async () => {
    setRecorderBusy(true);
    setRecorderError(null);
    try {
      const out = await stopRecording();
      setStatus(out.status);
      setPausedSince(null);
      setDiscarded(out.discarded);
      wasRecording.current = false;
      void refreshSessions();
      void refreshEnv();
    } catch (e) {
      setRecorderError(errorText(e));
    } finally {
      setRecorderBusy(false);
    }
  }, [refreshSessions, refreshEnv]);

  const resume = useCallback(async () => {
    setRecorderBusy(true);
    setRecorderError(null);
    try {
      applyControl(await resumeSession());
    } catch (e) {
      setRecorderError(errorText(e));
    } finally {
      setRecorderBusy(false);
    }
  }, []);

  const undoDiscard = useCallback(async () => {
    if (!discarded) return;
    setRecorderError(null);
    try {
      await undoDeleteSession(discarded.id);
      setDiscarded(null);
      void refreshSessions();
      void refreshEnv();
    } catch (e) {
      setDiscarded(null);
      setRecorderError(errorText(e));
    }
  }, [discarded, refreshSessions, refreshEnv]);

  // The undo is only good until the trash is emptied.
  useEffect(() => {
    if (!discarded) return;
    const t = window.setTimeout(() => setDiscarded(null), Math.max(0, discarded.undo_until_epoch_ms - Date.now()));
    return () => window.clearTimeout(t);
  }, [discarded]);

  const value = useMemo<AppData>(
    () => ({
      sessions,
      sessionsError,
      refreshSessions,
      manifests,
      manifestsReady,
      patchMark,
      markCount,
      exportCount,
      status,
      env,
      pausedSince,
      recorderBusy,
      recorderError,
      discarded,
      start,
      stop,
      resume,
      undoDiscard,
    }),
    [
      sessions,
      sessionsError,
      refreshSessions,
      manifests,
      manifestsReady,
      patchMark,
      markCount,
      exportCount,
      status,
      env,
      pausedSince,
      recorderBusy,
      recorderError,
      discarded,
      start,
      stop,
      resume,
      undoDiscard,
    ],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
