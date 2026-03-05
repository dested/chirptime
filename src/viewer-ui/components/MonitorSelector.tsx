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
    <div className="flex items-center gap-1">
      <span className="text-zinc-500 text-xs mr-1.5 font-medium">Monitor</span>
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
