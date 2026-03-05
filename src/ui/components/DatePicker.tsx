import React from "react";

interface Props {
  dates: string[];
  selected: string | null;
  onSelect: (date: string) => void;
}

const MONTH_NAMES = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];

export function DatePicker({ dates, selected, onSelect }: Props) {
  return (
    <div className="date-sidebar" style={{ width: 176, minWidth: 176, display: "flex", flexDirection: "column", flex: 1 }}>
      <div style={{ padding: "12px 16px", fontSize: 11, fontWeight: 600, color: "#71717a", textTransform: "uppercase", letterSpacing: "0.1em", borderBottom: "1px solid rgba(255,255,255,0.04)" }}>
        Recordings
      </div>
      <div style={{ flex: 1, overflowY: "auto", padding: 6 }}>
        {dates.map((date) => {
          const isActive = date === selected;
          const [y, m, d] = date.split("-");
          const display = `${MONTH_NAMES[parseInt(m) - 1]} ${parseInt(d)}`;
          return (
            <button
              key={date}
              onClick={() => onSelect(date)}
              className={`date-btn ${isActive ? "date-btn-active" : ""}`}
            >
              <span style={{ fontWeight: 500 }}>{display}</span>
              <span style={{ color: "#52525b", fontSize: 11, marginLeft: "auto", fontFamily: "ui-monospace, monospace" }}>{y}</span>
            </button>
          );
        })}
        {dates.length === 0 && (
          <div style={{ color: "#52525b", fontSize: 12, padding: "24px 12px", textAlign: "center" }}>No captures found</div>
        )}
      </div>
    </div>
  );
}
