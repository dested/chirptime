import React, { useState, useEffect, useCallback, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  startCapture,
  stopCapture,
  runSingleCapture,
  getCaptureStatus,
  fetchDates,
  fetchDateEntries,
  openViewer,
} from "./hooks/useApi";
import type { CaptureStatus, CaptureEntry } from "./types";

const appWindow = getCurrentWindow();

export function CompactApp() {
  const [status, setStatus] = useState<CaptureStatus>({
    running: false,
    last_capture_time: null,
    next_capture_time: null,
    captures_today: 0,
  });
  const [countdown, setCountdown] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [recentCaptures, setRecentCaptures] = useState<CaptureEntry[]>([]);
  const [todayDate, setTodayDate] = useState<string | null>(null);

  // Load initial data
  useEffect(() => {
    const load = async () => {
      try {
        const s = await getCaptureStatus();
        setStatus(s);
        const dates = await fetchDates();
        if (dates[0]) {
          setTodayDate(dates[0]);
          const entries = await fetchDateEntries(dates[0]);
          setRecentCaptures(entries.entries.slice(-6).reverse());
        }
      } catch {}
    };
    load();
  }, []);

  // Poll status every 3 seconds
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const s = await getCaptureStatus();
        setStatus(s);
      } catch {}
    }, 3000);
    return () => clearInterval(interval);
  }, []);

  // Refresh recent captures when status changes (new capture happened)
  useEffect(() => {
    const refresh = async () => {
      try {
        const dates = await fetchDates();
        if (dates[0]) {
          setTodayDate(dates[0]);
          const entries = await fetchDateEntries(dates[0]);
          setRecentCaptures(entries.entries.slice(-6).reverse());
        }
      } catch {}
    };
    refresh();
  }, [status.captures_today]);

  // Countdown timer
  useEffect(() => {
    if (!status.running || !status.next_capture_time) {
      setCountdown(null);
      return;
    }
    const update = () => {
      const now = new Date();
      const [h, m, s] = status.next_capture_time!.split(":").map(Number);
      const target = new Date();
      target.setHours(h, m, s, 0);
      const diffMs = target.getTime() - now.getTime();
      if (diffMs <= 0) {
        setCountdown("now");
        return;
      }
      const mins = Math.floor(diffMs / 60000);
      const secs = Math.floor((diffMs % 60000) / 1000);
      setCountdown(`${mins}:${secs.toString().padStart(2, "0")}`);
    };
    update();
    const interval = setInterval(update, 1000);
    return () => clearInterval(interval);
  }, [status.running, status.next_capture_time]);

  const handleStart = useCallback(async () => {
    setBusy(true);
    try {
      await startCapture();
      setStatus(await getCaptureStatus());
    } catch {}
    setBusy(false);
  }, []);

  const handleStop = useCallback(async () => {
    setBusy(true);
    try {
      await stopCapture();
      setStatus(await getCaptureStatus());
    } catch {}
    setBusy(false);
  }, []);

  const handleSingle = useCallback(async () => {
    setBusy(true);
    try {
      await runSingleCapture();
      setStatus(await getCaptureStatus());
    } catch {}
    setBusy(false);
  }, []);

  const handleClose = useCallback(() => { appWindow.close(); }, []);
  const handleMinimize = useCallback(() => { appWindow.minimize(); }, []);
  const handleDrag = useCallback((e: React.MouseEvent) => {
    // Don't drag if clicking a button
    if ((e.target as HTMLElement).closest("button")) return;
    appWindow.startDragging();
  }, []);

  return (
    <div className="compact-root">
      {/* Custom titlebar */}
      <div className="compact-titlebar" onMouseDown={handleDrag}>
        <div className="titlebar-brand">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" style={{ flexShrink: 0 }}>
            <circle cx="12" cy="12" r="10" fill="#0EA5E9" opacity="0.15" />
            <path d="M8 14c0-2.2 1.8-4 4-4s4 1.8 4 4" stroke="#0EA5E9" strokeWidth="2" strokeLinecap="round" />
            <circle cx="12" cy="8" r="2" fill="#0EA5E9" />
          </svg>
          <span className="titlebar-text">ChirpTime</span>
        </div>
        <div className="titlebar-controls">
          <button className="titlebar-btn" onClick={handleMinimize} title="Minimize">
            <svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" strokeWidth="1.5" /></svg>
          </button>
          <button className="titlebar-btn titlebar-btn-close" onClick={handleClose} title="Close">
            <svg width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" strokeWidth="1.5" /><line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" strokeWidth="1.5" /></svg>
          </button>
        </div>
      </div>

      {/* Status section */}
      <div className="compact-status-section">
        <div className={`status-indicator ${status.running ? "status-active" : "status-idle"}`}>
          <span className={`status-dot-compact ${status.running ? "dot-active" : "dot-idle"}`} />
          <span className="status-label">{status.running ? "Recording" : "Idle"}</span>
        </div>
        {status.running && countdown && (
          <div className="countdown-display">
            <span className="countdown-label">Next capture in</span>
            <span className="countdown-value">{countdown}</span>
          </div>
        )}
      </div>

      {/* Big action button */}
      <div className="compact-action-section">
        {status.running ? (
          <button
            className="big-action-btn big-action-stop"
            onClick={handleStop}
            disabled={busy}
          >
            <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
              <rect x="4" y="4" width="12" height="12" rx="2" />
            </svg>
            <span>Stop Recording</span>
          </button>
        ) : (
          <button
            className="big-action-btn big-action-start"
            onClick={handleStart}
            disabled={busy}
          >
            <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
              <circle cx="10" cy="10" r="7" />
            </svg>
            <span>Start Recording</span>
          </button>
        )}

        <button
          className="secondary-action-btn"
          onClick={handleSingle}
          disabled={busy}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <circle cx="12" cy="12" r="10" />
            <polyline points="12 6 12 12 16 14" />
          </svg>
          <span>Capture Now</span>
        </button>
      </div>

      {/* Stats */}
      <div className="compact-stats">
        <div className="stat-item">
          <span className="stat-value">{status.captures_today}</span>
          <span className="stat-label">Today</span>
        </div>
        <div className="stat-divider" />
        <div className="stat-item">
          <span className="stat-value">{status.last_capture_time ?? "--:--"}</span>
          <span className="stat-label">Last capture</span>
        </div>
      </div>

      {/* Recent captures */}
      {recentCaptures.length > 0 && (
        <div className="compact-recent">
          <div className="recent-header">
            <span className="recent-title">Recent</span>
            <span className="recent-date">{todayDate}</span>
          </div>
          <div className="recent-list">
            {recentCaptures.map((entry) => (
              <div key={entry.timestamp} className="recent-item">
                <div className="recent-dot-wrapper">
                  {entry.screen_video ? (
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#0EA5E9" strokeWidth="2.5">
                      <polygon points="5 3 19 12 5 21 5 3" fill="#0EA5E9" opacity="0.2" />
                    </svg>
                  ) : (
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#64748B" strokeWidth="2.5">
                      <rect x="3" y="3" width="18" height="18" rx="2" fill="#64748B" opacity="0.15" />
                      <circle cx="8.5" cy="8.5" r="1.5" fill="#64748B" />
                      <path d="M21 15l-5-5L5 21" />
                    </svg>
                  )}
                </div>
                <span className="recent-time">{entry.display_time}</span>
                <span className="recent-type">
                  {entry.screen_video ? "Video" : "Screenshot"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Browse button */}
      <div className="compact-footer">
        <button className="browse-btn" onClick={() => openViewer()}>
          <span>Browse All Captures</span>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M5 12h14M12 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </div>
  );
}
