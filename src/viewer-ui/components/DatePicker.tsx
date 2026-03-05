import React from "react";

interface Props {
  dates: string[];
  selected: string | null;
  onSelect: (date: string) => void;
}

export function DatePicker({ dates, selected, onSelect }: Props) {
  return (
    <div className="date-sidebar w-44 min-w-44 flex flex-col h-full">
      <div className="px-4 py-3 text-xs font-semibold text-zinc-500 uppercase tracking-widest border-b border-zinc-800/50">
        Recordings
      </div>
      <div className="flex-1 overflow-y-auto p-1.5 space-y-0.5">
        {dates.map((date) => {
          const isActive = date === selected;
          // Format nicely: "Feb 27" style
          const [y, m, d] = date.split("-");
          const monthNames = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
          const display = `${monthNames[parseInt(m) - 1]} ${parseInt(d)}`;
          return (
            <button
              key={date}
              onClick={() => onSelect(date)}
              className={`date-btn w-full text-left ${isActive ? "date-btn-active" : ""}`}
            >
              <span className="font-medium">{display}</span>
              <span className="text-zinc-600 text-xs ml-auto font-mono">{y}</span>
            </button>
          );
        })}
        {dates.length === 0 && (
          <div className="text-zinc-600 text-xs px-3 py-6 text-center">No captures found</div>
        )}
      </div>
    </div>
  );
}
