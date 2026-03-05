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

  // Auto-scroll to keep active entry visible
  useEffect(() => {
    if (activeRef.current && scrollRef.current) {
      activeRef.current.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
    }
  }, [selected]);

  return (
    <div
      ref={scrollRef}
      className="timeline-bar flex items-center gap-1 px-3 py-2 overflow-x-auto min-h-[42px]"
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
            {entry.displayTime}
          </button>
        );
      })}
      {entries.length === 0 && (
        <span className="text-zinc-600 text-xs">No captures for this date</span>
      )}
    </div>
  );
}
