import React from "react";
import type { MonitorConfig } from "../types";

interface Props {
  config: MonitorConfig | null;
  zoomed: number | null;
  onSelect: (monitorId: number | null) => void;
}

export function MonitorSelector({ config, zoomed, onSelect }: Props) {
  if (!config) return <div />;

  return (
    <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
      <span style={{ color: "#71717a", fontSize: 12, fontWeight: 500, marginRight: 6 }}>Monitor</span>
      <button
        onClick={() => onSelect(null)}
        className={`monitor-btn ${zoomed === null ? "monitor-btn-active" : ""}`}
      >
        All
      </button>
      {config.monitors.map((mon) => (
        <button
          key={mon.id}
          onClick={() => onSelect(mon.id)}
          className={`monitor-btn ${zoomed === mon.id ? "monitor-btn-active" : ""}`}
        >
          {mon.id}
        </button>
      ))}
    </div>
  );
}
