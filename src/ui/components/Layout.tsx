import React, { useEffect, useCallback, useRef, useState } from "react";
import { DatePicker } from "./DatePicker";
import { Timeline } from "./Timeline";
import { ScreenView } from "./ScreenView";
import { MonitorSelector } from "./MonitorSelector";
import { WebcamWindow } from "./WebcamWindow";
import { CaptureControls } from "./CaptureControls";
import { deleteEntry } from "../hooks/useApi";
import type { AppState } from "../types";

const SPEEDS = [1, 2, 4, 8, 16];

interface Props {
  state: AppState;
  onSelectDate: (date: string) => void;
  onSelectTimestamp: (ts: string) => void;
  onSelectMonitor: (id: number | null) => void;
  onRefresh: () => void;
  onCaptureStatusChange: (status: AppState["captureStatus"]) => void;
}

export function Layout({ state, onSelectDate, onSelectTimestamp, onSelectMonitor, onRefresh, onCaptureStatusChange }: Props) {
  const mainVideoRef = useRef<HTMLVideoElement | null>(null);
  const [playbackRate, setPlaybackRate] = useState(1);

  const currentEntry = state.dateEntry?.entries.find(
    (e) => e.timestamp === state.selectedTimestamp
  ) ?? null;

  const webcamSrc = currentEntry?.webcam_video && currentEntry?.screen_video
    ? currentEntry.webcam_video
    : null;

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

  const onVideoEnded = useCallback(() => {
    navigateTimeline(1);
  }, [navigateTimeline]);

  const handleDelete = useCallback(async () => {
    if (!state.selectedDate || !state.selectedTimestamp) return;
    if (!confirm(`Delete capture at ${state.selectedTimestamp.replace(/-/g, ":")}?`)) return;

    await deleteEntry(state.selectedDate, state.selectedTimestamp);

    // Move to next/prev entry or clear
    const entries = state.dateEntry?.entries ?? [];
    const idx = entries.findIndex((e) => e.timestamp === state.selectedTimestamp);
    const nextEntry = entries[idx + 1] ?? entries[idx - 1];

    if (nextEntry) {
      onSelectTimestamp(nextEntry.timestamp);
    }

    onRefresh();
  }, [state.selectedDate, state.selectedTimestamp, state.dateEntry, onSelectTimestamp, onRefresh]);

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
      } else if (e.key === "Delete" || e.key === "Backspace") {
        handleDelete();
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onSelectMonitor, state.monitorConfig, navigateTimeline, handleDelete]);

  const currentIndex = state.dateEntry?.entries.findIndex(
    (e) => e.timestamp === state.selectedTimestamp
  ) ?? -1;
  const totalEntries = state.dateEntry?.entries.length ?? 0;

  return (
    <div style={{ display: "flex", height: "100vh", overflow: "hidden", background: "#09090b", color: "#fafafa" }}>
      {/* Sidebar */}
      <div style={{ display: "flex", flexDirection: "column" }}>
        <DatePicker
          dates={state.dates}
          selected={state.selectedDate}
          onSelect={onSelectDate}
        />
        <CaptureControls
          status={state.captureStatus}
          onStatusChange={onCaptureStatusChange}
          onRefresh={onRefresh}
        />
      </div>

      {/* Main content */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
        {/* Timeline strip */}
        <Timeline
          entries={state.dateEntry?.entries ?? []}
          selected={state.selectedTimestamp}
          onSelect={onSelectTimestamp}
        />

        {/* Loading */}
        {state.loading && (
          <div style={{ padding: "8px 16px", color: "#38bdf8", fontSize: 12, fontWeight: 500 }} className="animate-pulse">
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
          outputDir={state.outputDir}
        />

        {/* Controls bar */}
        <div className="controls-bar" style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "8px 12px", gap: 12 }}>
          {/* Monitor selector */}
          <MonitorSelector
            config={state.monitorConfig}
            zoomed={state.zoomedMonitor}
            onSelect={onSelectMonitor}
          />

          {/* Position + delete */}
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{ color: "#71717a", fontSize: 12, fontFamily: "ui-monospace, monospace", fontVariantNumeric: "tabular-nums" }}>
              {totalEntries > 0 && `${currentIndex + 1} / ${totalEntries}`}
            </span>
            {state.selectedTimestamp && (
              <button className="delete-btn" onClick={handleDelete} title="Delete this capture (Del)">
                Delete
              </button>
            )}
          </div>

          {/* Speed controls */}
          <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
            <span style={{ color: "#71717a", fontSize: 12, fontWeight: 500, marginRight: 6 }}>Speed</span>
            {SPEEDS.map((s) => (
              <button
                key={s}
                onClick={() => setPlaybackRate(s)}
                className={`speed-btn ${playbackRate === s ? "speed-btn-active" : ""}`}
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
