/**
 * Mock IPC handlers owned by `library` (round 2). Return `UNHANDLED` for any
 * command this file doesn't answer; the core mock in ../mockIpc.ts tries the
 * extras before its own switch.
 *
 * - `set_marker_label` edits the in-memory fixture manifest, refusing the live
 *   session like the backend does.
 * - The live session: the core mock's `start_session` records as `mock-live`
 *   but lists nothing. This file watches start/stop go by (it sees every
 *   command first and lets them through) and answers `list_sessions` and
 *   `get_session` with a growing, footage-less `mock-live` manifest, so the
 *   Library's live block can be seen.
 */
import { UNHANDLED, type MockCtx, type MockResult } from "./ctx";
import type { SessionManifest, SessionMarker } from "../../types";
import { summarizeSession } from "../../hooks/useApi";

const LIVE_ID = "mock-live";
const LIVE_NOTES_ERR = "Stop recording to edit notes";
const NO_MARK_ERR = "That mark is no longer in this session";

let liveSince: number | null = null;

function liveManifest(): SessionManifest | null {
  if (liveSince === null) return null;
  const track = (id: string, kind: "screen" | "webcam") => ({
    id,
    kind,
    label: id,
    monitor: kind === "screen" ? { output_idx: 0, adapter: 0, x: 0, y: 0, w: 2560, h: 1440 } : null,
    codec: "h264",
    width: 1280,
    height: 720,
    fps: 30,
    start_epoch_ms: liveSince ?? 0,
    dir: id,
    segments: [],
    gaps: [],
    status: "recording" as const,
    restarts: 0,
  });
  return {
    version: 1,
    id: LIVE_ID,
    started_epoch_ms: liveSince,
    ended_epoch_ms: null,
    segment_seconds: 60,
    desktop: { x: 0, y: 0, w: 2560, h: 1440 },
    tracks: [track("screen-0", "screen"), track("webcam", "webcam")],
    markers: [],
    rebuilt: false,
    cuts: [],
  };
}

function str(a: Record<string, unknown>, k: string): string {
  const v = a[k];
  if (typeof v !== "string") throw new Error(`mock: ${k} must be a string`);
  return v;
}

function int(a: Record<string, unknown>, k: string): number {
  const v = a[k];
  if (typeof v !== "number" || !Number.isFinite(v)) throw new Error(`mock: ${k} must be a number`);
  return Math.round(v);
}

export function handleLibrary(cmd: string, a: Record<string, unknown>, ctx: MockCtx): MockResult {
  switch (cmd) {
    case "start_session":
      if (liveSince === null) liveSince = Date.now();
      return UNHANDLED;
    case "stop_session":
      liveSince = null;
      return UNHANDLED;
    case "list_sessions": {
      const listed = ctx.sessionIds().map((id) => summarizeSession(ctx.session(id)));
      const live = liveManifest();
      if (live) listed.unshift({ ...summarizeSession(live), duration_ms: Date.now() - live.started_epoch_ms });
      return listed;
    }
    case "get_session": {
      const live = liveManifest();
      if (live && a.id === LIVE_ID) return live;
      return UNHANDLED;
    }
    case "set_marker_label": {
      const id = str(a, "sessionId");
      if (id === LIVE_ID) throw new Error(LIVE_NOTES_ERR);
      const at = int(a, "epochMs");
      const label = str(a, "label").trim();
      const mk = ctx.session(id).markers.find((m) => m.epoch_ms === at);
      if (!mk) throw new Error(NO_MARK_ERR);
      mk.label = label;
      const out: SessionMarker = { ...mk };
      return out;
    }
    default:
      return UNHANDLED;
  }
}
