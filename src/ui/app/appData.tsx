/**
 * What every page of the app window shares: the session list, the recorder's
 * status, and the disk. One provider polls; pages read and ask for refreshes.
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { EnvironmentInfo, SessionStatus, SessionSummary } from "../types";
import { fetchSession, getEnvironment, getSessionStatus, listSessions, startSession, stopSession } from "../hooks/useApi";

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return "Something went wrong";
}

interface AppData {
  sessions: SessionSummary[] | null;
  sessionsError: string | null;
  refreshSessions: () => Promise<void>;
  /** markers across every session, or null while counting */
  markCount: number | null;
  status: SessionStatus | null;
  env: EnvironmentInfo | null;
  recorderBusy: boolean;
  recorderError: string | null;
  start: () => Promise<void>;
  stop: () => Promise<void>;
}

const Ctx = createContext<AppData | null>(null);

export function useAppData(): AppData {
  const v = useContext(Ctx);
  if (!v) throw new Error("useAppData outside AppDataProvider");
  return v;
}

const STATUS_POLL_MS = 2_000;

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [sessionsError, setSessionsError] = useState<string | null>(null);
  const [status, setStatus] = useState<SessionStatus | null>(null);
  const [env, setEnv] = useState<EnvironmentInfo | null>(null);
  const [recorderBusy, setRecorderBusy] = useState(false);
  const [recorderError, setRecorderError] = useState<string | null>(null);
  const [markCount, setMarkCount] = useState<number | null>(null);
  const markCache = useRef(new Map<string, number>());

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

  // Recorder status: polled, so a start or stop from the widget shows up here.
  const wasRecording = useRef<boolean | null>(null);
  useEffect(() => {
    let alive = true;
    const poll = async () => {
      try {
        const s = await getSessionStatus();
        if (!alive) return;
        setStatus(s);
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
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, [refreshSessions, refreshEnv]);

  // Count marks in the background, one manifest at a time, cached per session.
  // A finished session's markers never change; a live one is re-read each time.
  useEffect(() => {
    if (!sessions) return;
    let alive = true;
    void (async () => {
      let total = 0;
      for (const s of sessions) {
        if (!s.has_manifest) continue;
        const cached = s.ended_epoch_ms !== null ? markCache.current.get(s.id) : undefined;
        if (cached !== undefined) {
          total += cached;
          continue;
        }
        try {
          const m = await fetchSession(s.id);
          markCache.current.set(s.id, m.markers.length);
          total += m.markers.length;
        } catch {
          // an unreadable manifest has no marks we can show
        }
        if (!alive) return;
      }
      if (alive) setMarkCount(total);
    })();
    return () => {
      alive = false;
    };
  }, [sessions]);

  const start = useCallback(async () => {
    setRecorderBusy(true);
    setRecorderError(null);
    try {
      setStatus(await startSession());
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
      setStatus(await stopSession());
      wasRecording.current = false;
      void refreshSessions();
      void refreshEnv();
    } catch (e) {
      setRecorderError(errorText(e));
    } finally {
      setRecorderBusy(false);
    }
  }, [refreshSessions, refreshEnv]);

  const value = useMemo<AppData>(
    () => ({
      sessions,
      sessionsError,
      refreshSessions,
      markCount,
      status,
      env,
      recorderBusy,
      recorderError,
      start,
      stop,
    }),
    [sessions, sessionsError, refreshSessions, markCount, status, env, recorderBusy, recorderError, start, stop],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
