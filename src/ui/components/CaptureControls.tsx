import React, { useState, useEffect } from "react";
import { startCapture, stopCapture, runSingleCapture, getCaptureStatus } from "../hooks/useApi";
import type { CaptureStatus } from "../types";

interface Props {
  status: CaptureStatus;
  onStatusChange: (status: CaptureStatus) => void;
  onRefresh: () => void;
}

export function CaptureControls({ status, onStatusChange, onRefresh }: Props) {
  const [countdown, setCountdown] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Calculate countdown to next capture
  useEffect(() => {
    if (!status.running || !status.next_capture_time) {
      setCountdown(null);
      return;
    }

    const updateCountdown = () => {
      const now = new Date();
      const [h, m, s] = status.next_capture_time!.split(":").map(Number);
      const target = new Date();
      target.setHours(h, m, s, 0);

      const diffMs = target.getTime() - now.getTime();
      if (diffMs <= 0) {
        setCountdown("Now");
        return;
      }

      const mins = Math.floor(diffMs / 60000);
      const secs = Math.floor((diffMs % 60000) / 1000);
      setCountdown(`${mins}:${secs.toString().padStart(2, "0")}`);
    };

    updateCountdown();
    const interval = setInterval(updateCountdown, 1000);
    return () => clearInterval(interval);
  }, [status.running, status.next_capture_time]);

  const handleStart = async () => {
    setBusy(true);
    try {
      await startCapture();
      const s = await getCaptureStatus();
      onStatusChange(s);
      onRefresh();
    } catch (err) {
      console.error("Failed to start capture:", err);
    }
    setBusy(false);
  };

  const handleStop = async () => {
    setBusy(true);
    try {
      await stopCapture();
      const s = await getCaptureStatus();
      onStatusChange(s);
    } catch (err) {
      console.error("Failed to stop capture:", err);
    }
    setBusy(false);
  };

  const handleSingle = async () => {
    setBusy(true);
    try {
      await runSingleCapture();
      const s = await getCaptureStatus();
      onStatusChange(s);
      onRefresh();
    } catch (err) {
      console.error("Capture failed:", err);
    }
    setBusy(false);
  };

  return (
    <div className="date-sidebar" style={{ borderTop: "1px solid rgba(255,255,255,0.04)", padding: 8 }}>
      {/* Status indicator */}
      <div className="capture-status" style={{ marginBottom: 8 }}>
        <span className={`status-dot ${status.running ? "status-dot-active" : "status-dot-inactive"}`} />
        <span style={{ color: status.running ? "#22c55e" : "#71717a", fontWeight: 500 }}>
          {status.running ? "Recording" : "Stopped"}
        </span>
      </div>

      {/* Timer */}
      {status.running && countdown && (
        <div style={{ padding: "4px 12px", marginBottom: 8, fontSize: 11, color: "#a1a1aa" }}>
          Next: <span style={{ fontFamily: "ui-monospace, monospace", color: "#38bdf8", fontVariantNumeric: "tabular-nums" }}>{countdown}</span>
        </div>
      )}

      {/* Stats */}
      {status.last_capture_time && (
        <div style={{ padding: "0 12px 8px", fontSize: 10, color: "#52525b" }}>
          Last: {status.last_capture_time} | Today: {status.captures_today}
        </div>
      )}

      {/* Buttons */}
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {status.running ? (
          <button className="action-btn action-btn-danger" onClick={handleStop} disabled={busy}>
            Stop Capture
          </button>
        ) : (
          <button className="action-btn action-btn-primary" onClick={handleStart} disabled={busy}>
            Start Capture
          </button>
        )}
        <button className="action-btn action-btn-neutral" onClick={handleSingle} disabled={busy}>
          Capture Now
        </button>
      </div>
    </div>
  );
}
