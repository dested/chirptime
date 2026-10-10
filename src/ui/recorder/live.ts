/**
 * The live session as the widget shows it: the recorder's control state (kept
 * current by `recorder-changed` and a 2 s poll), the status with each track's
 * health, and the plain-words reading of that health.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getSessionStatus } from "../hooks/useApi";
import { numberList } from "../player/timeline";
import { fmtSpan } from "../session/format";
import type { SessionStatus, TrackSummary } from "../types";
import { CHANGED_EVENT, getRecorderControl, type RecorderControl } from "./api";

const POLL_MS = 2_000;
/** MAX_RESTARTS in recorder.rs: a crashing track gives up after this many tries. */
export const MAX_TRIES = 20;

export interface Live {
  control: RecorderControl | null;
  status: SessionStatus | null;
  refresh: () => Promise<void>;
}

/** Subscribe to a Tauri event for the life of the component. */
export function useTauriEvent<T>(event: string, on: (payload: T) => void): void {
  const ref = useRef(on);
  ref.current = on;
  useEffect(() => {
    let dispose: (() => void) | null = null;
    let alive = true;
    listen<T>(event, (e) => ref.current(e.payload))
      .then((un) => {
        if (alive) dispose = un;
        else un();
      })
      .catch(() => {});
    return () => {
      alive = false;
      dispose?.();
    };
  }, [event]);
}

export function useLive(): Live {
  const [control, setControl] = useState<RecorderControl | null>(null);
  const [status, setStatus] = useState<SessionStatus | null>(null);

  const refresh = useCallback(async () => {
    const [c, s] = await Promise.allSettled([getRecorderControl(), getSessionStatus()]);
    if (c.status === "fulfilled") setControl(c.value);
    if (s.status === "fulfilled") setStatus(s.value);
  }, []);

  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(t);
  }, [refresh]);

  useTauriEvent<RecorderControl>(CHANGED_EVENT, (c) => {
    setControl(c);
    void refresh();
  });

  return { control, status, refresh };
}

/* ---- health in words ------------------------------------------------------- */

export type Health = "ok" | "warn" | "dead";

/** Dead: waiting out an outage or given up. Warn: restarting after a crash. */
export function healthOf(t: TrackSummary): Health {
  if (t.status === "failed" || t.waiting_since_epoch_ms !== null) return "dead";
  if (t.status === "restarting") return "warn";
  return "ok";
}

export function screenIndex(t: TrackSummary): number | null {
  const m = /^screen-(\d+)$/.exec(t.id);
  return m ? Number(m[1]) : null;
}

export function trackName(t: TrackSummary): string {
  const n = screenIndex(t);
  if (n !== null) return `Screen ${n + 1}`;
  switch (t.kind) {
    case "webcam":
      return "Camera";
    case "mic":
      return "Microphone";
    case "system-audio":
      return "System audio";
    default:
      return t.label;
  }
}

/** When this track's outage began: the wait, or the last footage of one that gave up. */
function downSince(t: TrackSummary): number {
  return t.waiting_since_epoch_ms ?? t.last_segment_end_ms ?? t.start_epoch_ms;
}

export interface Outage {
  /** 1-based screen numbers */
  screens: number[];
  since: number;
  why: string;
  /** what is still recording, as a sentence */
  still: string;
}

/** Screens that are down, as one alert. Null while every screen records. */
export function screenOutage(status: SessionStatus | null): Outage | null {
  if (!status) return null;
  const down = status.tracks.filter((t) => screenIndex(t) !== null && healthOf(t) === "dead");
  if (down.length === 0) return null;
  const up = status.tracks.filter((t) => healthOf(t) !== "dead");
  const upScreens = up.flatMap((t) => {
    const n = screenIndex(t);
    return n === null ? [] : [n + 1];
  });
  const parts: string[] = [];
  if (upScreens.length) parts.push(`${upScreens.length === 1 ? "Screen" : "Screens"} ${numberList(upScreens)}`);
  if (up.some((t) => t.kind === "webcam")) parts.push("Camera");
  if (up.some((t) => t.kind === "mic" || t.kind === "system-audio")) parts.push("audio");
  const still =
    parts.length === 0
      ? "Nothing else is recording."
      : `${parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}` : parts[0]} ${
          parts.length > 1 || upScreens.length > 1 ? "are" : "is"
        } still recording.`;
  return {
    screens: down.flatMap((t) => {
      const n = screenIndex(t);
      return n === null ? [] : [n + 1];
    }),
    since: Math.min(...down.map(downSince)),
    why: (down.find((t) => t.last_error_human)?.last_error_human ?? "Screen capture stopped").replace(/\.$/, ""),
    still: still.charAt(0).toUpperCase() + still.slice(1),
  };
}

/** `Screens 1–3 stopped 14 min ago` */
export function outageTitle(o: Outage, now: number): string {
  const who = `${o.screens.length === 1 ? "Screen" : "Screens"} ${numberList(o.screens)}`;
  const ago = now - o.since;
  return ago < 60_000 ? `${who} stopped just now` : `${who} stopped ${fmtSpan(ago).replace(/^(\d+)m$/, "$1 min")} ago`;
}

/** One line per track that is restarting, or a camera or mic that is down. */
export function troubleLines(status: SessionStatus | null): { id: string; text: string }[] {
  if (!status) return [];
  return status.tracks.flatMap((t) => {
    const h = healthOf(t);
    if (h === "ok") return [];
    if (h === "warn") return [{ id: t.id, text: `${trackName(t)} restarting, try ${Math.max(1, t.restarts)} of ${MAX_TRIES}` }];
    if (screenIndex(t) !== null) return [];
    const why = t.last_error_human ? `: ${t.last_error_human.replace(/\.$/, "")}` : "";
    return [{ id: t.id, text: `${trackName(t)} stopped${why}` }];
  });
}
