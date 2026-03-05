import React, { useEffect, useRef } from "react";
import type { CaptureEntry } from "../types";

interface Props {
  entries: CaptureEntry[];
  selected: string | null;
  onSelect: (timestamp: string) => void;
}

export function Timeline({ entries, selected, onSelect }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const activeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (activeRef.current && scrollRef.current) {
      activeRef.current.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
    }
  }, [selected]);

  return (
    <div
      ref={scrollRef}
      className="timeline-bar"
      style={{ display: "flex", alignItems: "center", gap: 4, padding: "8px 12px", overflowX: "auto", minHeight: 42 }}
    >
      {entries.map((entry) => {
        const isActive = entry.timestamp === selected;
        return (
          <button
            key={entry.timestamp}
            ref={isActive ? activeRef : undefined}
            onClick={() => onSelect(entry.timestamp)}
            className={`timeline-btn ${isActive ? "timeline-btn-active" : ""}`}
          >
            {entry.display_time}
          </button>
        );
      })}
      {entries.length === 0 && (
        <span style={{ color: "#52525b", fontSize: 12 }}>No captures for this date</span>
      )}
    </div>
  );
}
