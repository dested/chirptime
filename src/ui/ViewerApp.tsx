import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { fetchDates, fetchDateEntries, deleteEntry } from "./hooks/useApi";
import { mediaUrl } from "./hooks/mediaUrl";
import type { CaptureEntry, DateEntries } from "./types";

type View = "calendar" | "gallery" | "viewer";

const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

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
  const videoRef = useRef<HTMLVideoElement>(null);

  // Load available dates
  useEffect(() => {
    fetchDates().then((dates) => {
      setAvailableDates(new Set(dates));
      // Preload counts per date
      const counts = new Map<string, number>();
      dates.forEach(async (d) => {
        try {
          const e = await fetchDateEntries(d);
          counts.set(d, e.entries.length);
          setDateCounts(new Map(counts));
        } catch {}
      });
    });
  }, []);

  // Navigate to a day
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

  // Open media viewer
  const openEntry = useCallback((idx: number) => {
    setSelectedIdx(idx);
    setView("viewer");
  }, []);

  // Navigate within viewer
  const goPrev = useCallback(() => {
    setSelectedIdx((i) => Math.max(0, i - 1));
  }, []);
  const goNext = useCallback(() => {
    setSelectedIdx((i) => Math.min(entries.length - 1, i + 1));
  }, [entries.length]);

  // Delete current entry
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

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (view === "viewer") {
        if (e.key === "ArrowLeft") goPrev();
        else if (e.key === "ArrowRight") goNext();
        else if (e.key === "Escape") setView("gallery");
        else if (e.key === "Delete" || e.key === "Backspace") handleDelete();
      } else if (view === "gallery" && e.key === "Escape") {
        setView("calendar");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [view, goPrev, goNext, handleDelete]);

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

  // Parse selected date for display
  const selectedDateDisplay = useMemo(() => {
    if (!selectedDate) return "";
    const [y, m, d] = selectedDate.split("-");
    return `${MONTH_NAMES[parseInt(m) - 1]} ${parseInt(d)}, ${y}`;
  }, [selectedDate]);

  return (
    <div className="v-root">
      {/* ---- CALENDAR VIEW ---- */}
      {view === "calendar" && (
        <div className="v-calendar">
          <div className="v-cal-header">
            <div className="v-cal-brand">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none">
                <circle cx="12" cy="12" r="10" fill="#0EA5E9" opacity="0.15" />
                <path d="M8 14c0-2.2 1.8-4 4-4s4 1.8 4 4" stroke="#0EA5E9" strokeWidth="2" strokeLinecap="round" />
                <circle cx="12" cy="8" r="2" fill="#0EA5E9" />
              </svg>
              <span>ChirpTime</span>
            </div>
          </div>

          <div className="v-cal-nav">
            <button className="v-cal-nav-btn" onClick={prevMonth}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
            </button>
            <div className="v-cal-month-label">
              <span className="v-cal-month">{MONTH_NAMES[calMonth]}</span>
              <span className="v-cal-year">{calYear}</span>
            </div>
            <button className="v-cal-nav-btn" onClick={nextMonth}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M9 18l6-6-6-6"/></svg>
            </button>
            <button className="v-cal-today-btn" onClick={goToday}>Today</button>
          </div>

          <div className="v-cal-grid">
            {DAY_NAMES.map((d) => (
              <div key={d} className="v-cal-day-name">{d}</div>
            ))}
            {calendarDays.map((day, i) => {
              if (day === null) return <div key={`e${i}`} className="v-cal-cell v-cal-cell-empty" />;
              const dateStr = formatDateStr(calYear, calMonth, day);
              const hasCaptures = availableDates.has(dateStr);
              const count = dateCounts.get(dateStr) ?? 0;
              const today = isToday(day);
              return (
                <button
                  key={i}
                  className={`v-cal-cell ${hasCaptures ? "v-cal-cell-has" : ""} ${today ? "v-cal-cell-today" : ""}`}
                  onClick={() => hasCaptures && openDay(dateStr)}
                  disabled={!hasCaptures}
                >
                  <span className="v-cal-day-num">{day}</span>
                  {hasCaptures && (
                    <span className="v-cal-count">{count || ""}</span>
                  )}
                </button>
              );
            })}
          </div>

          {/* Recent days quick list */}
          <div className="v-cal-recent">
            <div className="v-cal-recent-title">Recent Days</div>
            <div className="v-cal-recent-list">
              {Array.from(availableDates).slice(0, 8).map((date) => {
                const [y, m, d] = date.split("-");
                const count = dateCounts.get(date) ?? 0;
                return (
                  <button key={date} className="v-cal-recent-item" onClick={() => openDay(date)}>
                    <span className="v-cal-recent-date">
                      {MONTH_NAMES[parseInt(m) - 1].slice(0, 3)} {parseInt(d)}
                    </span>
                    <span className="v-cal-recent-year">{y}</span>
                    {count > 0 && <span className="v-cal-recent-count">{count} captures</span>}
                  </button>
                );
              })}
            </div>
          </div>
        </div>
      )}

      {/* ---- GALLERY VIEW ---- */}
      {view === "gallery" && (
        <div className="v-gallery">
          <div className="v-gal-header">
            <button className="v-back-btn" onClick={() => setView("calendar")}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
              <span>Back</span>
            </button>
            <div className="v-gal-title">
              <h2>{selectedDateDisplay}</h2>
              <span className="v-gal-count">{entries.length} captures</span>
            </div>
          </div>

          {loading ? (
            <div className="v-gal-loading">Loading...</div>
          ) : entries.length === 0 ? (
            <div className="v-gal-empty">
              <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#CBD5E1" strokeWidth="1.5">
                <rect x="3" y="3" width="18" height="18" rx="2" />
                <circle cx="8.5" cy="8.5" r="1.5" />
                <path d="M21 15l-5-5L5 21" />
              </svg>
              <p>No captures for this day</p>
            </div>
          ) : (
            <div className="v-gal-grid">
              {entries.map((entry, idx) => {
                const thumbSrc = entry.screenshot ? mediaUrl(entry.screenshot) : null;
                const hasVideo = !!(entry.combined_video ?? entry.screen_video);
                return (
                  <button
                    key={entry.timestamp}
                    className="v-gal-card"
                    onClick={() => openEntry(idx)}
                  >
                    <div className="v-gal-thumb">
                      {thumbSrc ? (
                        <img src={thumbSrc} alt="" loading="lazy" />
                      ) : (
                        <div className="v-gal-thumb-placeholder">
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
                        <div className="v-gal-play-badge">
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="white">
                            <polygon points="6 3 20 12 6 21 6 3" />
                          </svg>
                        </div>
                      )}
                    </div>
                    <div className="v-gal-card-info">
                      <span className="v-gal-card-time">{entry.display_time}</span>
                      <span className="v-gal-card-type">{hasVideo ? "Video" : "Photo"}</span>
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      )}

      {/* ---- MEDIA VIEWER ---- */}
      {view === "viewer" && currentEntry && (
        <div className="v-viewer">
          <div className="v-viewer-header">
            <button className="v-back-btn v-back-btn-dark" onClick={() => setView("gallery")}>
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
              <span>Back</span>
            </button>
            <div className="v-viewer-info">
              <span className="v-viewer-time">{currentEntry.display_time}</span>
              <span className="v-viewer-pos">{selectedIdx + 1} of {entries.length}</span>
            </div>
            <button className="v-viewer-delete" onClick={handleDelete}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <polyline points="3 6 5 6 21 6" />
                <path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" />
              </svg>
            </button>
          </div>

          <div className="v-viewer-media">
            {/* Nav arrows */}
            {selectedIdx > 0 && (
              <button className="v-viewer-nav v-viewer-nav-left" onClick={goPrev}>
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><path d="M15 18l-6-6 6-6"/></svg>
              </button>
            )}
            {selectedIdx < entries.length - 1 && (
              <button className="v-viewer-nav v-viewer-nav-right" onClick={goNext}>
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><path d="M9 18l6-6-6-6"/></svg>
              </button>
            )}

            {(() => {
              const videoSrc = currentEntry.combined_video ?? currentEntry.screen_video;
              if (videoSrc) {
                return (
                  <video
                    ref={videoRef}
                    key={videoSrc}
                    src={mediaUrl(videoSrc)}
                    controls
                    autoPlay
                    className="v-viewer-content"
                    onEnded={goNext}
                  />
                );
              }
              if (currentEntry.screenshot) {
                return (
                  <img
                    key={currentEntry.screenshot}
                    src={mediaUrl(currentEntry.screenshot)}
                    className="v-viewer-content"
                    alt=""
                  />
                );
              }
              return <div className="v-viewer-empty">No media available</div>;
            })()}
          </div>

          {/* Filmstrip at bottom */}
          <div className="v-viewer-filmstrip">
            {entries.map((entry, idx) => {
              const thumb = entry.screenshot ? mediaUrl(entry.screenshot) : null;
              const active = idx === selectedIdx;
              return (
                <button
                  key={entry.timestamp}
                  className={`v-film-item ${active ? "v-film-active" : ""}`}
                  onClick={() => setSelectedIdx(idx)}
                >
                  {thumb ? (
                    <img src={thumb} alt="" loading="lazy" />
                  ) : (
                    <div className="v-film-placeholder" />
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
