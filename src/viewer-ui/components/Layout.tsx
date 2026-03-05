import React, { useEffect, useCallback, useRef, useState } from "react";
import { DatePicker } from "./DatePicker";
import { Timeline } from "./Timeline";
import { ScreenView } from "./ScreenView";
import { MonitorSelector } from "./MonitorSelector";
import { WebcamWindow } from "./WebcamWindow";
import type { AppState } from "../types";

const SPEEDS = [1, 2, 4, 8, 16];

interface Props {
  state: AppState;
  onSelectDate: (date: string) => void;
  onSelectTimestamp: (ts: string) => void;
  onSelectMonitor: (id: number | null) => void;
}

export function Layout({ state, onSelectDate, onSelectTimestamp, onSelectMonitor }: Props) {
  const mainVideoRef = useRef<HTMLVideoElement | null>(null);
  const [playbackRate, setPlaybackRate] = useState(1);

  const currentEntry = state.dateEntry?.entries.find(
    (e) => e.timestamp === state.selectedTimestamp
  ) ?? null;

  const webcamSrc =
    currentEntry?.webcamVideo && currentEntry?.screenVideo
      ? `/files/${currentEntry.webcamVideo}`
      : null;

  // Navigate timeline entries
  const navigateTimeline = useCallback(
    (dir: -1 | 1) => {
      if (!state.dateEntry || !state.selectedTimestamp) return;
      const entries = state.dateEntry.entries;
      const idx = entries.findIndex((e) => e.timestamp === state.selectedTimestamp);
      const next = idx + dir;
      if (next >= 0 && next < entries.length) {
        onSelectTimestamp(entries[next].timestamp);
      }
    },
    [state.dateEntry, state.selectedTimestamp, onSelectTimestamp]
  );

  // Auto-advance when video ends
  const onVideoEnded = useCallback(() => {
    navigateTimeline(1);
  }, [navigateTimeline]);

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === "INPUT") return;

      if (e.key === "Escape") {
        onSelectMonitor(null);
      } else if (e.key >= "1" && e.key <= "9") {
        const monId = parseInt(e.key);
        if (state.monitorConfig?.monitors.some((m) => m.id === monId)) {
          onSelectMonitor(monId);
        }
      } else if (e.key === "ArrowLeft") {
        navigateTimeline(-1);
      } else if (e.key === "ArrowRight") {
        navigateTimeline(1);
      } else if (e.key === "`" || e.key === "0") {
        onSelectMonitor(null);
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onSelectMonitor, state.monitorConfig, navigateTimeline]);

  // Current position indicator
  const currentIndex = state.dateEntry?.entries.findIndex(
    (e) => e.timestamp === state.selectedTimestamp
  ) ?? -1;
  const totalEntries = state.dateEntry?.entries.length ?? 0;

  return (
    <div className="flex h-screen overflow-hidden bg-zinc-950">
      {/* Sidebar */}
      <DatePicker
        dates={state.dates}
        selected={state.selectedDate}
        onSelect={onSelectDate}
      />

      {/* Main content */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Timeline strip */}
        <Timeline
          entries={state.dateEntry?.entries ?? []}
          selected={state.selectedTimestamp}
          onSelect={onSelectTimestamp}
        />

        {/* Loading indicator */}
        {state.loading && (
          <div className="px-4 py-2 text-sky-400 text-xs font-medium tracking-wide animate-pulse">
            Loading...
          </div>
        )}

        {/* Video area */}
        <ScreenView
          entry={currentEntry}
          monitorConfig={state.monitorConfig}
          zoomedMonitor={state.zoomedMonitor}
          mainVideoRef={mainVideoRef}
          onEnded={onVideoEnded}
          playbackRate={playbackRate}
        />

        {/* Controls bar */}
        <div className="controls-bar flex items-center justify-between px-3 py-2 gap-3">
          {/* Monitor selector */}
          <MonitorSelector
            config={state.monitorConfig}
            zoomed={state.zoomedMonitor}
            onSelect={onSelectMonitor}
          />

          {/* Position indicator */}
          <div className="text-zinc-500 text-xs font-mono tabular-nums tracking-wide">
            {totalEntries > 0 && (
              <span>{currentIndex + 1} / {totalEntries}</span>
            )}
          </div>

          {/* Speed controls */}
          <div className="flex items-center gap-1">
            <span className="text-zinc-500 text-xs mr-1.5 font-medium">Speed</span>
            {SPEEDS.map((s) => (
              <button
                key={s}
                onClick={() => setPlaybackRate(s)}
                className={`speed-btn ${
                  playbackRate === s ? "speed-btn-active" : ""
                }`}
              >
                {s}x
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* Floating webcam */}
      <WebcamWindow src={webcamSrc} mainVideoRef={mainVideoRef} />
    </div>
  );
}
