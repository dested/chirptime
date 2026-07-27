import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import {
  fetchDates,
  fetchDateEntries,
  fetchMonitorConfig,
  deleteEntry,
  listSessions,
  fetchSession,
  rebuildSession,
  getSessionStatus,
} from "./hooks/useApi";
import { mediaUrl } from "./hooks/mediaUrl";
import type { CaptureEntry, MonitorConfig, SessionManifest, SessionSummary } from "./types";
import { SessionBrowser } from "./session/SessionBrowser";
import { SessionPlayer } from "./session/SessionPlayer";

type View = "calendar" | "gallery" | "viewer" | "sessions" | "session";

const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const SPEEDS = [1, 1.5, 2, 4];

function getCalendarDays(year: number, month: number) {
  const first = new Date(year, month, 1);
  const last = new Date(year, month + 1, 0);
  const startOffset = first.getDay();
  const daysInMonth = last.getDate();
  const cells: (number | null)[] = [];
  for (let i = 0; i < startOffset; i++) cells.push(null);
  for (let d = 1; d <= daysInMonth; d++) cells.push(d);
  while (cells.length < 42) cells.push(null);
  return cells;
}

function formatDateStr(year: number, month: number, day: number) {
  return `${year}-${String(month + 1).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

function formatTimeAmPm(displayTime: string): string {
  const [h, m] = displayTime.split(":");
  const hour = parseInt(h);
  const ampm = hour >= 12 ? "PM" : "AM";
  const h12 = hour === 0 ? 12 : hour > 12 ? hour - 12 : hour;
  return `${h12}:${m} ${ampm}`;
}

function BrandMark() {
  return (
    <div className="flex items-center gap-2.5 text-lg font-bold text-slate-900 tracking-tight">
      <svg width="24" height="24" viewBox="0 0 24 24" fill="none">
        <circle cx="12" cy="12" r="10" fill="#0EA5E9" opacity="0.15" />
        <path d="M8 14c0-2.2 1.8-4 4-4s4 1.8 4 4" stroke="#0EA5E9" strokeWidth="2" strokeLinecap="round" />
        <circle cx="12" cy="8" r="2" fill="#0EA5E9" />
      </svg>
      <span>ChirpTime</span>
    </div>
  );
}

function ViewerTabs({
  tab,
  onClips,
  onSessions,
}: {
  tab: "clips" | "sessions";
  onClips: () => void;
  onSessions: () => void;
}) {
  const cls = (active: boolean) =>
    `px-3.5 py-1.5 rounded-lg text-[13px] font-semibold cursor-pointer transition-all ${
      active ? "bg-white text-slate-900 shadow-sm" : "text-slate-500 hover:text-slate-900"
    }`;
  return (
    <div className="flex gap-0.5 p-0.5 rounded-[10px] border border-slate-200 bg-slate-100">
      <button className={cls(tab === "clips")} onClick={onClips}>
        Clips
      </button>
      <button className={cls(tab === "sessions")} onClick={onSessions}>
        Sessions
      </button>
    </div>
  );
}

export function ViewerApp() {
  const now = new Date();
  const [view, setView] = useState<View>("calendar");
  const [calYear, setCalYear] = useState(now.getFullYear());
  const [calMonth, setCalMonth] = useState(now.getMonth());
  const [availableDates, setAvailableDates] = useState<Set<string>>(new Set());
  const [dateCounts, setDateCounts] = useState<Map<string, number>>(new Map());
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [entries, setEntries] = useState<CaptureEntry[]>([]);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [loading, setLoading] = useState(false);
  const [monitorConfig, setMonitorConfig] = useState<MonitorConfig | null>(null);
  const [zoomedMonitor, setZoomedMonitor] = useState<number | "webcam" | null>(null);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [containerSize, setContainerSize] = useState({ w: 0, h: 0 });
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(false);
  const [sessionsError, setSessionsError] = useState<string | null>(null);
  const [manifest, setManifest] = useState<SessionManifest | null>(null);
  const [sessionSeekMs, setSessionSeekMs] = useState<number | null>(null);
  const [liveSessionId, setLiveSessionId] = useState<string | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const mediaContainerRef = useRef<HTMLDivElement>(null);
  const savedTimeRef = useRef<number>(0);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const linkDate = params.get("date");
    const linkTs = params.get("ts");

    fetchMonitorConfig().then(setMonitorConfig).catch(() => {});

    fetchDates().then(async (dates) => {
      setAvailableDates(new Set(dates));
      const counts = new Map<string, number>();
      dates.forEach(async (d) => {
        try {
          const e = await fetchDateEntries(d);
          counts.set(d, e.entries.length);
          setDateCounts(new Map(counts));
        } catch {}
      });

      if (linkDate) {
        try {
          const data = await fetchDateEntries(linkDate);
          setSelectedDate(linkDate);
          setEntries(data.entries);
          if (linkTs) {
            const idx = data.entries.findIndex((e) => e.timestamp === linkTs);
            if (idx >= 0) {
              setSelectedIdx(idx);
              setView("viewer");
            } else {
              setView("gallery");
            }
          } else {
            setView("gallery");
          }
        } catch {}
      }
    });
  }, []);

  const loadSessions = useCallback(async () => {
    setSessionsLoading(true);
    setSessionsError(null);
    // Which session is *actually* recording; a force-killed one has no end time
    // and would otherwise claim to be live forever.
    getSessionStatus()
      .then((s) => setLiveSessionId(s.recording ? s.session_id : null))
      .catch(() => setLiveSessionId(null));
    try {
      const list = await listSessions();
      list.sort((a, b) => b.started_epoch_ms - a.started_epoch_ms);
      setSessions(list);
    } catch {
      setSessions([]);
      setSessionsError("Could not read the session index");
    }
    setSessionsLoading(false);
  }, []);

  const openSession = useCallback(async (id: string) => {
    setSessionsLoading(true);
    setSessionsError(null);
    try {
      const m = await fetchSession(id);
      setManifest(m);
      setView("session");
    } catch {
      setSessionsError(`Could not load session ${id}`);
      setView("sessions");
    }
    setSessionsLoading(false);
  }, []);

  const openSessions = useCallback(() => {
    setManifest(null);
    setSessionSeekMs(null);
    setView("sessions");
    void loadSessions();
  }, [loadSessions]);

  /** Persist a manifest that only exists because we scanned the directory. */
  const saveRebuiltIndex = useCallback(async () => {
    if (!manifest) return;
    try {
      setManifest(await rebuildSession(manifest.id));
    } catch {}
  }, [manifest]);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const linkSession = params.get("session");
    if (linkSession) {
      // With `session`, `ts` is an epoch-ms instant rather than a clip's
      // `HH-MM-SS` — a session has no per-clip timestamps to name.
      const ts = Number(params.get("ts"));
      setSessionSeekMs(Number.isFinite(ts) && ts > 0 ? ts : null);
      void openSession(linkSession);
      void loadSessions();
    } else if (params.get("tab") === "sessions") {
      openSessions();
    }
  }, [openSession, openSessions, loadSessions]);

  useEffect(() => {
    const el = mediaContainerRef.current;
    if (!el) return;
    const obs = new ResizeObserver(([e]) => {
      setContainerSize({ w: e.contentRect.width, h: e.contentRect.height });
    });
    obs.observe(el);
    return () => obs.disconnect();
  }, [view]);

  useEffect(() => {
    const v = videoRef.current;
    if (v) v.playbackRate = playbackRate;
  }, [playbackRate, selectedIdx]);

  const setZoomWithSync = useCallback((val: number | "webcam" | null) => {
    const v = videoRef.current;
    if (v && v.currentTime > 0) {
      savedTimeRef.current = v.currentTime;
    }
    setZoomedMonitor(val);
  }, []);

  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    const onLoaded = () => {
      v.playbackRate = playbackRate;
      if (savedTimeRef.current > 0) {
        v.currentTime = savedTimeRef.current;
      }
    };
    v.addEventListener("loadedmetadata", onLoaded);
    return () => v.removeEventListener("loadedmetadata", onLoaded);
  }, [playbackRate, selectedIdx, zoomedMonitor]);

  const openDay = useCallback(async (date: string) => {
    setLoading(true);
    setSelectedDate(date);
    try {
      const data = await fetchDateEntries(date);
      setEntries(data.entries);
    } catch {
      setEntries([]);
    }
    setView("gallery");
    setLoading(false);
  }, []);

  const openEntry = useCallback((idx: number) => {
    savedTimeRef.current = 0;
    setSelectedIdx(idx);
    setZoomedMonitor(null);
    setView("viewer");
  }, []);

  const goPrev = useCallback(() => {
    setSelectedIdx((i) => Math.max(0, i - 1));
  }, []);
  const goNext = useCallback(() => {
    setSelectedIdx((i) => Math.min(entries.length - 1, i + 1));
  }, [entries.length]);

  const handleDelete = useCallback(async () => {
    if (!selectedDate || !entries[selectedIdx]) return;
    const entry = entries[selectedIdx];
    if (!confirm(`Delete capture at ${entry.display_time}?`)) return;
    await deleteEntry(selectedDate, entry.timestamp);
    const newEntries = entries.filter((_, i) => i !== selectedIdx);
    setEntries(newEntries);
    if (newEntries.length === 0) {
      setView("gallery");
    } else {
      setSelectedIdx(Math.min(selectedIdx, newEntries.length - 1));
    }
  }, [selectedDate, entries, selectedIdx]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (view === "viewer") {
        if (e.key === "ArrowLeft") goPrev();
        else if (e.key === "ArrowRight") goNext();
        else if (e.key === "Escape") {
          if (zoomedMonitor !== null) setZoomWithSync(null);
          else setView("gallery");
        }
        else if (e.key === "Delete" || e.key === "Backspace") handleDelete();
        else if (e.key === "0" || e.key === "`") setZoomWithSync(null);
        else if (e.key === "w" || e.key === "W") setZoomWithSync("webcam");
        else if (e.key >= "1" && e.key <= "9") {
          const id = parseInt(e.key);
          if (monitorConfig?.monitors.some((m) => m.id === id)) setZoomWithSync(id);
        }
      } else if (view === "gallery" && e.key === "Escape") {
        setView("calendar");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [view, goPrev, goNext, handleDelete, zoomedMonitor, monitorConfig, setZoomWithSync]);

  const calendarDays = useMemo(() => getCalendarDays(calYear, calMonth), [calYear, calMonth]);

  const prevMonth = () => {
    if (calMonth === 0) { setCalYear(calYear - 1); setCalMonth(11); }
    else setCalMonth(calMonth - 1);
  };
  const nextMonth = () => {
    if (calMonth === 11) { setCalYear(calYear + 1); setCalMonth(0); }
    else setCalMonth(calMonth + 1);
  };
  const goToday = () => { setCalYear(now.getFullYear()); setCalMonth(now.getMonth()); };

  const currentEntry = entries[selectedIdx] ?? null;
  const isToday = (day: number) => {
    return day === now.getDate() && calMonth === now.getMonth() && calYear === now.getFullYear();
  };

  const selectedDateDisplay = useMemo(() => {
    if (!selectedDate) return "";
    const [y, m, d] = selectedDate.split("-");
    return `${MONTH_NAMES[parseInt(m) - 1]} ${parseInt(d)}, ${y}`;
  }, [selectedDate]);

  const zoomStyle = useMemo((): React.CSSProperties | null => {
    if (!monitorConfig || zoomedMonitor === null || zoomedMonitor === "webcam" || !containerSize.w) return null;
    const mon = monitorConfig.monitors.find((m) => m.id === zoomedMonitor);
    if (!mon) return null;

    // We need to fit the monitor region into the container.
    // The video/image will be stretched (object-fit: fill) to exactly match the wrapper size.
    // So 1 CSS pixel of the wrapper = 1 unit of the totalWidth/totalHeight coordinate space.

    // Scale factor: how much to scale the full virtual desktop so the monitor fills the container
    const scaleX = containerSize.w / mon.w;
    const scaleY = containerSize.h / mon.h;
    const scale = Math.min(scaleX, scaleY);

    const renderedW = monitorConfig.totalWidth * scale;
    const renderedH = monitorConfig.totalHeight * scale;

    // Center the monitor region in the container
    const offsetX = -mon.x * scale + (containerSize.w - mon.w * scale) / 2;
    const offsetY = -mon.y * scale + (containerSize.h - mon.h * scale) / 2;

    return {
      position: "absolute" as const,
      width: renderedW,
      height: renderedH,
      left: offsetX,
      top: offsetY,
    };
  }, [monitorConfig, zoomedMonitor, containerSize]);

  return (
    <div className="h-screen overflow-hidden bg-slate-50 text-slate-900 flex flex-col">
      {/* ---- CALENDAR VIEW ---- */}
      {view === "calendar" && (
        <div className="flex-1 min-h-0 flex flex-col max-w-[960px] w-full mx-auto px-8">
          <div className="pt-6 flex items-center">
            <BrandMark />
            <div className="flex-1" />
            <ViewerTabs tab="clips" onClips={() => setView("calendar")} onSessions={openSessions} />
          </div>

          <div className="flex items-center gap-2 pt-5 pb-4">
            <button className="w-9 h-9 flex items-center justify-center border border-slate-200 bg-white text-slate-500 rounded-[10px] cursor-pointer hover:bg-slate-100 hover:text-slate-900 hover:border-slate-300 transition-all" onClick={prevMonth}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
            </button>
            <div className="flex items-baseline gap-2 flex-1 px-2">
              <span className="text-2xl font-bold text-slate-900 tracking-tight">{MONTH_NAMES[calMonth]}</span>
              <span className="text-base font-medium text-slate-400">{calYear}</span>
            </div>
            <button className="w-9 h-9 flex items-center justify-center border border-slate-200 bg-white text-slate-500 rounded-[10px] cursor-pointer hover:bg-slate-100 hover:text-slate-900 hover:border-slate-300 transition-all" onClick={nextMonth}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M9 18l6-6-6-6"/></svg>
            </button>
            <button className="px-4 py-1.5 rounded-lg text-[13px] font-semibold border border-slate-200 bg-white text-slate-600 cursor-pointer hover:bg-slate-100 hover:border-slate-300 hover:text-slate-900 transition-all" onClick={goToday}>Today</button>
          </div>

          <div className="grid grid-cols-7 gap-1">
            {DAY_NAMES.map((d) => (
              <div key={d} className="py-2 text-center text-[11px] font-semibold text-slate-400 uppercase tracking-wide">{d}</div>
            ))}
            {calendarDays.map((day, i) => {
              if (day === null) return <div key={`e${i}`} className="aspect-[1.4]" />;
              const dateStr = formatDateStr(calYear, calMonth, day);
              const hasCaptures = availableDates.has(dateStr);
              const count = dateCounts.get(dateStr) ?? 0;
              const today = isToday(day);
              return (
                <button
                  key={i}
                  className={`aspect-[1.4] flex flex-col items-center justify-center gap-1 rounded-xl border transition-all ${
                    hasCaptures
                      ? "bg-white border-slate-200 shadow-sm cursor-pointer hover:border-sky-500 hover:shadow-[0_2px_8px_rgba(14,165,233,0.15)] hover:-translate-y-px"
                      : "bg-transparent border-transparent cursor-default"
                  }`}
                  onClick={() => hasCaptures && openDay(dateStr)}
                  disabled={!hasCaptures}
                >
                  <span className={`text-sm font-semibold ${
                    today
                      ? "bg-sky-500 text-white w-7 h-7 flex items-center justify-center rounded-full"
                      : hasCaptures ? "text-slate-700" : "text-slate-300"
                  }`}>{day}</span>
                  {hasCaptures && count > 0 && (
                    <span className="text-[10px] font-semibold text-sky-500">{count}</span>
                  )}
                </button>
              );
            })}
          </div>

          <div className="flex-1 min-h-0 py-4 border-t border-slate-200 mt-4 overflow-y-auto">
            <div className="text-[11px] font-semibold text-slate-400 uppercase tracking-wide mb-2">Recent Days</div>
            <div className="flex flex-wrap gap-1.5">
              {Array.from(availableDates).slice(0, 8).map((date) => {
                const [y, m, d] = date.split("-");
                const count = dateCounts.get(date) ?? 0;
                return (
                  <button key={date} className="flex items-center gap-2 px-3.5 py-2 rounded-[10px] border border-slate-200 bg-white cursor-pointer text-[13px] hover:border-sky-500 hover:shadow-[0_2px_8px_rgba(14,165,233,0.1)] transition-all" onClick={() => openDay(date)}>
                    <span className="font-semibold text-slate-900">{MONTH_NAMES[parseInt(m) - 1].slice(0, 3)} {parseInt(d)}</span>
                    <span className="text-slate-400 text-xs">{y}</span>
                    {count > 0 && <span className="text-sky-500 text-xs font-medium ml-auto">{count} captures</span>}
                  </button>
                );
              })}
            </div>
          </div>
        </div>
      )}

      {/* ---- SESSION BROWSER ---- */}
      {view === "sessions" && (
        <div className="flex-1 min-h-0 flex flex-col">
          <div className="flex items-center max-w-[960px] w-full mx-auto px-8 pt-6 pb-4">
            <BrandMark />
            <div className="flex-1" />
            <ViewerTabs tab="sessions" onClips={() => setView("calendar")} onSessions={openSessions} />
          </div>
          <div className="max-w-[960px] w-full mx-auto px-8 flex items-baseline gap-3 pb-1">
            <h2 className="text-xl font-bold tracking-tight text-slate-900">Sessions</h2>
            <span className="text-[13px] text-slate-400 font-medium">{sessions.length} recorded</span>
          </div>
          <SessionBrowser
            sessions={sessions}
            loading={sessionsLoading}
            error={sessionsError}
            onOpen={openSession}
            liveSessionId={liveSessionId}
          />
        </div>
      )}

      {/* ---- SESSION PLAYER ---- */}
      {view === "session" && manifest && (
        <SessionPlayer
          manifest={manifest}
          onBack={openSessions}
          initialEpochMs={sessionSeekMs}
          onRebuild={() => void saveRebuiltIndex()}
        />
      )}

      {/* ---- GALLERY VIEW ---- */}
      {view === "gallery" && (
        <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
          <div className="flex items-center gap-4 px-8 py-4 bg-white border-b border-slate-100">
            <button className="flex items-center gap-1 px-3 py-2 rounded-[10px] border border-slate-200 bg-white text-slate-600 text-[13px] font-medium cursor-pointer hover:bg-slate-50 hover:border-slate-300 hover:text-slate-900 transition-all" onClick={() => setView("calendar")}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
              <span>Back</span>
            </button>
            <div className="flex items-baseline gap-3">
              <h2 className="text-xl font-bold tracking-tight text-slate-900">{selectedDateDisplay}</h2>
              <span className="text-[13px] text-slate-400 font-medium">{entries.length} captures</span>
            </div>
          </div>

          {loading ? (
            <div className="flex-1 flex items-center justify-center text-slate-400 text-sm">Loading...</div>
          ) : entries.length === 0 ? (
            <div className="flex-1 flex flex-col items-center justify-center gap-3 text-slate-400 text-sm">
              <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#CBD5E1" strokeWidth="1.5">
                <rect x="3" y="3" width="18" height="18" rx="2" />
                <circle cx="8.5" cy="8.5" r="1.5" />
                <path d="M21 15l-5-5L5 21" />
              </svg>
              <p>No captures for this day</p>
            </div>
          ) : (
            <div className="flex-1 min-h-0 overflow-y-auto">
              <div className="p-6 px-8 grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-4 content-start">
                {entries.map((entry, idx) => {
                  const thumbSrc = entry.screenshot ? mediaUrl(entry.screenshot) : null;
                  const hasVideo = !!(entry.combined_video ?? entry.screen_video);
                  const hasWebcam = !!entry.webcam_video;
                  const timeAmPm = formatTimeAmPm(entry.display_time);
                  return (
                    <button key={entry.timestamp} className="border border-slate-200 rounded-[14px] overflow-hidden bg-white cursor-pointer text-left p-0 hover:border-sky-500 hover:shadow-[0_4px_16px_rgba(14,165,233,0.12)] hover:-translate-y-0.5 transition-all" onClick={() => openEntry(idx)}>
                      <div className="aspect-video bg-slate-100 overflow-hidden relative">
                        {thumbSrc ? (
                          <img src={thumbSrc} alt="" loading="lazy" className="w-full h-full object-cover block" />
                        ) : (
                          <div className="w-full h-full flex items-center justify-center bg-slate-100">
                            <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#94A3B8" strokeWidth="1.5">
                              {hasVideo ? (
                                <polygon points="5 3 19 12 5 21 5 3" />
                              ) : (
                                <>
                                  <rect x="3" y="3" width="18" height="18" rx="2" />
                                  <circle cx="8.5" cy="8.5" r="1.5" />
                                  <path d="M21 15l-5-5L5 21" />
                                </>
                              )}
                            </svg>
                          </div>
                        )}
                        {hasVideo && thumbSrc && (
                          <div className="absolute bottom-2 right-2 w-8 h-8 flex items-center justify-center bg-black/55 backdrop-blur-sm rounded-full">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="white">
                              <polygon points="6 3 20 12 6 21 6 3" />
                            </svg>
                          </div>
                        )}
                      </div>
                      <div className="flex items-center justify-between px-3.5 py-2.5">
                        <div className="flex flex-col gap-px">
                          <span className="text-sm font-semibold text-slate-900">{timeAmPm}</span>
                          <span className="text-[11px] text-slate-400 tabular-nums font-mono">{entry.display_time}</span>
                        </div>
                        <div className="flex gap-1">
                          {hasVideo && <span className="text-[10px] font-semibold px-2 py-0.5 rounded-md bg-blue-50 text-blue-600">Video</span>}
                          {hasWebcam && <span className="text-[10px] font-semibold px-2 py-0.5 rounded-md bg-purple-50 text-purple-500">Cam</span>}
                          {!hasVideo && <span className="text-[10px] font-semibold px-2 py-0.5 rounded-md bg-slate-50 text-slate-400">Photo</span>}
                        </div>
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}

      {/* ---- MEDIA VIEWER ---- */}
      {view === "viewer" && currentEntry && (
        <div className="flex-1 min-h-0 flex flex-col bg-[#0A0A0F]">
          <div className="flex items-center px-5 py-3 gap-4 bg-white/[0.03] border-b border-white/[0.06]">
            <button className="flex items-center gap-1 px-3 py-2 rounded-[10px] border border-white/10 bg-white/[0.06] text-white/70 text-[13px] font-medium cursor-pointer hover:bg-white/10 hover:text-white hover:border-white/20 transition-all" onClick={() => setView("gallery")}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
              <span>Back</span>
            </button>
            <div className="flex-1 flex items-center gap-3">
              <span className="text-base font-bold text-white tabular-nums font-mono">{formatTimeAmPm(currentEntry.display_time)}</span>
              <span className="text-[13px] text-white/40">{selectedIdx + 1} of {entries.length}</span>
            </div>

            {/* Monitor zoom buttons */}
            {monitorConfig && monitorConfig.monitors.length > 1 && (
              <div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
                <button
                  className={`px-3 py-1 rounded-md text-xs font-semibold border-none cursor-pointer transition-all ${
                    zoomedMonitor === null ? "text-sky-500 bg-sky-500/15" : "text-white/40 bg-transparent hover:text-white/70 hover:bg-white/[0.06]"
                  }`}
                  onClick={() => setZoomWithSync(null)}
                >All</button>
                {monitorConfig.monitors.map((mon) => (
                  <button
                    key={mon.id}
                    className={`px-3 py-1 rounded-md text-xs font-semibold border-none cursor-pointer transition-all ${
                      zoomedMonitor === mon.id ? "text-sky-500 bg-sky-500/15" : "text-white/40 bg-transparent hover:text-white/70 hover:bg-white/[0.06]"
                    }`}
                    onClick={() => setZoomWithSync(mon.id)}
                    title={mon.label}
                  >{mon.id}</button>
                ))}
                {currentEntry.webcam_video && (
                  <button
                    className={`px-3 py-1 rounded-md text-xs font-semibold border-none cursor-pointer transition-all ${
                      zoomedMonitor === "webcam" ? "text-purple-400 bg-purple-500/15" : "text-white/40 bg-transparent hover:text-white/70 hover:bg-white/[0.06]"
                    }`}
                    onClick={() => setZoomWithSync("webcam")}
                    title="Webcam (W)"
                  >Cam</button>
                )}
              </div>
            )}

            {/* Speed control */}
            <div className="flex gap-0.5 p-0.5 bg-white/[0.04] rounded-lg border border-white/[0.06]">
              {SPEEDS.map((s) => (
                <button
                  key={s}
                  className={`px-2.5 py-1 rounded-md text-[11px] font-semibold font-mono border-none cursor-pointer transition-all ${
                    playbackRate === s ? "text-white bg-white/10" : "text-white/40 bg-transparent hover:text-white/70 hover:bg-white/[0.06]"
                  }`}
                  onClick={() => setPlaybackRate(s)}
                >{s}x</button>
              ))}
            </div>

            <button className="w-9 h-9 flex items-center justify-center rounded-[10px] border border-white/[0.08] bg-transparent text-white/40 cursor-pointer hover:text-red-500 hover:bg-red-500/10 hover:border-red-500/30 transition-all" onClick={handleDelete}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <polyline points="3 6 5 6 21 6" />
                <path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" />
              </svg>
            </button>
          </div>

          <div className="flex-1 min-h-0 flex items-center justify-center relative p-2" ref={mediaContainerRef}>
            {selectedIdx > 0 && (
              <button className="absolute left-4 top-1/2 -translate-y-1/2 w-11 h-11 flex items-center justify-center rounded-full border-none bg-white/[0.08] backdrop-blur-lg text-white/70 cursor-pointer hover:bg-white/15 hover:text-white transition-all z-10" onClick={goPrev}>
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
              </button>
            )}
            {selectedIdx < entries.length - 1 && (
              <button className="absolute right-4 top-1/2 -translate-y-1/2 w-11 h-11 flex items-center justify-center rounded-full border-none bg-white/[0.08] backdrop-blur-lg text-white/70 cursor-pointer hover:bg-white/15 hover:text-white transition-all z-10" onClick={goNext}>
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><path d="M9 18l6-6-6-6"/></svg>
              </button>
            )}

            {(() => {
              const isWebcam = zoomedMonitor === "webcam";
              const videoSrc = isWebcam
                ? currentEntry.webcam_video
                : (currentEntry.combined_video ?? currentEntry.screen_video);
              const isMonitorZoom = zoomedMonitor !== null && zoomedMonitor !== "webcam" && zoomStyle !== null;

              const mediaEl = videoSrc ? (
                <video
                  ref={videoRef}
                  key={videoSrc}
                  src={mediaUrl(videoSrc)}
                  controls
                  autoPlay
                  style={isMonitorZoom ? { width: "100%", height: "100%", display: "block", objectFit: "fill" } : undefined}
                  className={isMonitorZoom ? undefined : "max-w-full max-h-full rounded-lg block object-contain"}
                  onEnded={goNext}
                />
              ) : currentEntry.screenshot ? (
                <img
                  key={currentEntry.screenshot}
                  src={mediaUrl(currentEntry.screenshot)}
                  style={isMonitorZoom ? { width: "100%", height: "100%", display: "block", objectFit: "fill" } : undefined}
                  className={isMonitorZoom ? undefined : "max-w-full max-h-full rounded-lg block object-contain"}
                  alt=""
                />
              ) : (
                <div className="text-white/30 text-sm">No media available</div>
              );

              if (isMonitorZoom) {
                return (
                  <div className="w-full h-full relative overflow-hidden rounded-lg">
                    <div style={zoomStyle!}>
                      {mediaEl}
                    </div>
                  </div>
                );
              }
              return mediaEl;
            })()}
          </div>

          {/* Filmstrip */}
          <div className="flex items-center gap-1 px-4 py-2 overflow-x-auto bg-white/[0.03] border-t border-white/[0.06] dark-scroll">
            {entries.map((entry, idx) => {
              const thumb = entry.screenshot ? mediaUrl(entry.screenshot) : null;
              const active = idx === selectedIdx;
              return (
                <button
                  key={entry.timestamp}
                  className={`shrink-0 w-[72px] h-[42px] rounded-md overflow-hidden border-2 cursor-pointer p-0 transition-all ${
                    active ? "border-sky-500" : "border-transparent"
                  }`}
                  style={{ background: "#1E1E2A" }}
                  onClick={() => setSelectedIdx(idx)}
                >
                  {thumb ? (
                    <img src={thumb} alt="" loading="lazy" className={`w-full h-full object-cover block transition-opacity ${active ? "opacity-100" : "opacity-50 hover:opacity-80"}`} />
                  ) : (
                    <div className="w-full h-full" style={{ background: "#1E1E2A" }} />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
