/**
 * The screen under the mouse, for Focus to follow. Cursor samples are fetched
 * per 10-minute window around the playhead and evaluated at the engine's 5 Hz
 * state rate. `hasData` stays false until some window returns a sample; until
 * then the player keeps its static screen. A failed fetch reads as no data.
 */
import { useEffect, useRef, useState } from "react";
import type { CursorSample, SessionTrack } from "../types";
import type { SyncEngine } from "../session/engine";
import { getCursorTrack } from "../hooks/useApi";
import { CURSOR_WINDOW_MS, followScreen, mergeWindows, windowsFor } from "./follow";

export function useCursorFollow(
  engine: SyncEngine,
  sessionId: string,
  screens: readonly SessionTrack[],
  initial: string | null,
): { hasData: boolean; followedId: string | null } {
  const [hasData, setHasData] = useState(false);
  const [followedId, setFollowedId] = useState<string | null>(initial);

  // Survives engine rebuilds (a delete re-reads the manifest); cut ranges come
  // back from the backend already excluded, and the clock never rests in one.
  const cache = useRef({ session: sessionId, windows: new Map<number, CursorSample[]>(), requested: new Set<number>(), merged: [] as CursorSample[] });
  if (cache.current.session !== sessionId) {
    cache.current = { session: sessionId, windows: new Map(), requested: new Set(), merged: [] };
  }
  const current = useRef(initial);
  const screensRef = useRef(screens);
  screensRef.current = screens;

  useEffect(() => {
    let alive = true;
    const c = cache.current;

    const evaluate = (t: number) => {
      const next = followScreen(c.merged, screensRef.current, t, current.current);
      if (next !== current.current) {
        current.current = next;
        setFollowedId(next);
      }
    };

    const ensure = (t: number) => {
      for (const w of windowsFor(t)) {
        if (c.requested.has(w)) continue;
        c.requested.add(w);
        const from = w * CURSOR_WINDOW_MS;
        getCursorTrack(sessionId, from, from + CURSOR_WINDOW_MS)
          .then((samples) => {
            // kept even when this engine is gone: the next one reuses the cache
            c.windows.set(w, samples);
            c.merged = mergeWindows(c.windows);
            if (cache.current !== c) return;
            if (samples.length) setHasData(true);
            if (alive) evaluate(engine.masterEpochMs);
          })
          .catch(() => {
            // no cursor track (or no command): the player keeps its static screen
          });
      }
    };

    const t0 = engine.masterEpochMs;
    ensure(t0);
    evaluate(t0);
    const off = engine.subscribeState((s) => {
      ensure(s.masterEpochMs);
      evaluate(s.masterEpochMs);
    });
    return () => {
      alive = false;
      off();
    };
  }, [engine, sessionId]);

  return { hasData, followedId };
}
