/**
 * The recorder widget (Tauri label `main`, 320×540, always on top). Idle, it
 * shows what the next recording takes and Start; recording, the clock, the
 * desk map as a health map, Mark, Pause and Stop; it shrinks to a 264×52 pill.
 * Setup and the full settings live in the app window (`open_app`), so the
 * widget never shows onboarding.
 *
 * Start, stop, pause and mark go through the same Rust actions as the tray and
 * the global hotkeys (controls.rs), and every change comes back as
 * `recorder-changed`, so all three surfaces agree.
 */
import { useCallback, useEffect, useState, type MouseEvent, type ReactNode } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { fetchSession, listSessions, undoDeleteSession } from "./hooks/useApi";
import { fmtSize } from "./session/format";
import type { SessionMarker, SessionSummary, SessionTrashReceipt } from "./types";
import {
  DISCARDED_EVENT,
  MARK_EVENT,
  markNow,
  openApp,
  pauseSession,
  restartScreens,
  resumeSession,
  setLiveMarkerLabel,
  startRecording,
  stopRecording,
} from "./recorder/api";
import { IdleView } from "./recorder/IdleView";
import { useLive, useTauriEvent } from "./recorder/live";
import { OptionsView } from "./recorder/OptionsView";
import { PillView } from "./recorder/PillView";
import { RecordingView, type MarksState } from "./recorder/RecordingView";
import { useRecordingSetup } from "./recorder/setup";
import { CtMark, IconBack, IconChevron, IconClose, IconLibrary, IconMinimize, IconOptions, IconPill } from "./recorder/ui";

const appWindow = getCurrentWindow();
const FULL = { w: 320, h: 540 };
const PILL = { w: 264, h: 52 };

/** First run happens in the app window; open it once per launch until it is done. */
let setupOpened = false;

type Busy = "start" | "pause" | "stop" | "mark" | "restart" | null;

interface Toast {
  text: string;
  undoId: string | null;
  until: number;
}

function useNow(ms: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(t);
  }, [ms]);
  return now;
}

/** How many marks this session has, and the newest (the one the note field edits). */
function useMarks(sessionId: string | null): [MarksState, (m: SessionMarker) => void] {
  const [marks, setMarks] = useState<MarksState>({ count: 0, last: null });
  useEffect(() => {
    setMarks({ count: 0, last: null });
    if (!sessionId) return;
    let alive = true;
    fetchSession(sessionId)
      .then((m) => {
        if (!alive) return;
        const sorted = [...m.markers].sort((a, b) => a.epoch_ms - b.epoch_ms);
        setMarks((cur) => (cur.count > 0 ? cur : { count: sorted.length, last: sorted[sorted.length - 1] ?? null }));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [sessionId]);
  useTauriEvent<SessionMarker>(MARK_EVENT, (m) => setMarks((cur) => ({ count: cur.count + 1, last: m })));
  const noted = useCallback((m: SessionMarker) => {
    setMarks((cur) => (cur.last?.epoch_ms === m.epoch_ms ? { ...cur, last: m } : cur));
  }, []);
  return [marks, noted];
}

function TitleButton({ title, onClick, children }: { title: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      title={title}
      aria-label={title}
      onClick={onClick}
      className="grid h-7 w-7 place-items-center rounded-[7px] text-ink-3 transition-colors hover:bg-sunk hover:text-ink"
    >
      {children}
    </button>
  );
}

export function CompactApp() {
  const setup = useRecordingSetup();
  const live = useLive();
  const control = live.control;
  const recording = control?.recording ?? false;
  const paused = control?.paused_since_epoch_ms != null;
  const [view, setView] = useState<"main" | "options">("main");
  const [pill, setPill] = useState(false);
  const [busy, setBusy] = useState<Busy>(null);
  const [error, setError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [toast, setToast] = useState<Toast | null>(null);
  const now = useNow(recording ? 1000 : 30_000);
  const [marks, noted] = useMarks(control?.session_id ?? null);

  // The window itself is transparent so the widget and the pill can be round.
  useEffect(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
  }, []);

  useEffect(() => {
    const size = pill ? PILL : FULL;
    appWindow.setSize(new LogicalSize(size.w, size.h)).catch(() => {});
  }, [pill]);
  useEffect(() => {
    if (!recording) setPill(false);
  }, [recording]);

  // The library footer and the last session: read again whenever a session ends.
  useEffect(() => {
    if (recording) return;
    let alive = true;
    listSessions()
      .then((s) => alive && setSessions(s))
      .catch(() => alive && setSessions(null));
    return () => {
      alive = false;
    };
  }, [recording]);

  const onboarded = setup.settings?.onboarded ?? true;
  useEffect(() => {
    if (onboarded || setupOpened) return;
    setupOpened = true;
    openApp("#/library").catch(() => {});
  }, [onboarded]);

  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), Math.max(0, toast.until - Date.now()));
    return () => window.clearTimeout(t);
  }, [toast]);

  const act = useCallback(
    async (kind: Exclude<Busy, null>, run: () => Promise<unknown>) => {
      setBusy(kind);
      setError(null);
      try {
        await run();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(null);
        void live.refresh();
      }
    },
    [live],
  );

  const start = () =>
    act("start", async () => {
      await startRecording();
      setView("main");
    });
  const stop = () => act("stop", () => stopRecording());
  // The Undo for a discarded short session, whichever surface stopped it.
  useTauriEvent<SessionTrashReceipt>(DISCARDED_EVENT, (r) =>
    setToast({ text: "Discarded a session under a minute", undoId: r.id, until: r.undo_until_epoch_ms }),
  );
  const pauseToggle = () => act("pause", () => (paused ? resumeSession() : pauseSession()));
  const mark = () => act("mark", () => markNow());
  const restart = () => act("restart", () => restartScreens());
  const open = (route: string) => void openApp(route).catch((e) => setError(String(e)));
  const note = async (epochMs: number, label: string) => {
    noted(await setLiveMarkerLabel(epochMs, label));
  };

  const drag = (e: MouseEvent) => {
    if (e.button !== 0) return;
    if (e.target instanceof Element && e.target.closest("button, input, select, a")) return;
    void appWindow.startDragging().catch(() => {});
  };

  if (pill && recording && control) {
    return (
      <PillView
        control={control}
        status={live.status}
        now={now}
        markKey={setup.prefs?.hotkeys.mark ?? ""}
        onExpand={() => setPill(false)}
        onMark={() => void mark()}
        onStop={() => void stop()}
        onDrag={drag}
      />
    );
  }

  const ended = (sessions ?? []).filter((s) => s.ended_epoch_ms !== null);
  const totalBytes = (sessions ?? []).reduce((a, s) => a + s.bytes, 0);
  const options = view === "options";

  let titlebar;
  if (options) {
    titlebar = (
      <>
        <button
          title="Back"
          aria-label="Back"
          onClick={() => setView("main")}
          className="-ml-1 grid h-7 w-7 place-items-center rounded-[7px] text-ink hover:bg-sunk"
        >
          <IconBack />
        </button>
        <span className="-ml-0.5 text-[13px] font-bold tracking-[-0.01em]">Recording options</span>
      </>
    );
  } else {
    titlebar = (
      <>
        <span className="flex">
          <CtMark
            size={18}
            dot={recording ? (paused ? "var(--color-ink-3)" : "var(--color-rec)") : "var(--color-ink)"}
            arcs={recording && !paused ? "rgba(229,55,43,.55)" : "var(--color-ink-3)"}
          />
        </span>
        <span className={`text-[13px] font-bold tracking-[-0.01em] ${recording && !paused ? "text-rec" : ""}`}>
          {recording ? (paused ? "Paused" : "Recording") : "ChirpTime"}
        </span>
      </>
    );
  }

  return (
    <div className="flex h-screen select-none flex-col overflow-hidden rounded-[12px] border border-line bg-[#FBFAF7]">
      <div className="flex h-10 shrink-0 items-center gap-2 pl-3 pr-2" onMouseDown={drag}>
        {titlebar}
        <div className="flex-1" />
        {!options &&
          (recording ? (
            <TitleButton title="Shrink to pill" onClick={() => setPill(true)}>
              <IconPill />
            </TitleButton>
          ) : (
            <>
              <TitleButton title="Library" onClick={() => open("#/library")}>
                <IconLibrary />
              </TitleButton>
              <TitleButton title="Recording options" onClick={() => setView("options")}>
                <IconOptions />
              </TitleButton>
            </>
          ))}
        <TitleButton title="Minimize" onClick={() => void appWindow.minimize().catch(() => {})}>
          <IconMinimize />
        </TitleButton>
        <TitleButton title={recording ? "Hide (keeps recording)" : "Close"} onClick={() => void appWindow.close().catch(() => {})}>
          <IconClose />
        </TitleButton>
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-4 pt-1">
        {options ? (
          <OptionsView setup={setup} locked={recording} />
        ) : recording && control ? (
          <RecordingView
            setup={setup}
            control={control}
            status={live.status}
            now={now}
            marks={marks}
            busy={busy === "start" ? null : busy}
            error={error}
            onMark={() => void mark()}
            onPauseToggle={() => void pauseToggle()}
            onStop={() => void stop()}
            onRestart={() => void restart()}
            onDetails={() => control.session_id && open(`#/session/${encodeURIComponent(control.session_id)}`)}
            onNote={note}
          />
        ) : (
          <IdleView
            setup={setup}
            lastSession={ended[0] ?? null}
            busy={busy === "start"}
            error={error}
            needsSetup={!onboarded}
            onStart={() => void start()}
            onOptions={() => setView("options")}
            onOpen={open}
          />
        )}
      </div>

      {toast && (
        <div className="toast-in mx-3 mb-2 flex h-9 shrink-0 items-center gap-2 rounded-[9px] bg-ink px-3 text-[12px] text-white">
          <span className="min-w-0 flex-1 truncate">{toast.text}</span>
          {toast.undoId && (
            <button
              className="font-semibold text-canary"
              onClick={() => {
                const id = toast.undoId;
                setToast(null);
                if (id) undoDeleteSession(id).then(() => listSessions().then(setSessions)).catch((e) => setError(String(e)));
              }}
            >
              Undo
            </button>
          )}
        </div>
      )}

      <div className="flex h-11 shrink-0 items-center border-t border-line-2 px-4">
        {options ? (
          <>
            <span className="text-[11.5px] text-ink-3">Applies to the next recording</span>
            <FootLink label="All settings" onClick={() => open("#/settings")} />
          </>
        ) : recording && control ? (
          <>
            <span className="num text-[11px] text-ink-3">
              {live.status ? `${live.status.tracks.length} ${live.status.tracks.length === 1 ? "track" : "tracks"}` : ""}
            </span>
            {control.session_id && (
              <FootLink label="Open this session" onClick={() => open(`#/session/${encodeURIComponent(control.session_id ?? "")}`)} />
            )}
          </>
        ) : (
          <>
            <span className="num text-[11px] text-ink-3">
              {sessions ? `${sessions.length} ${sessions.length === 1 ? "session" : "sessions"} · ${fmtSize(totalBytes)}` : ""}
            </span>
            <FootLink label="Library" onClick={() => open("#/library")} />
          </>
        )}
      </div>
    </div>
  );
}

function FootLink({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button className="ml-auto flex items-center gap-1 text-[12.5px] font-semibold hover:text-ink-2" onClick={onClick}>
      {label}
      <IconChevron />
    </button>
  );
}
