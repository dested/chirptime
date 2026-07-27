const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

const p2 = (n: number) => String(n).padStart(2, "0");

/** wall clock HH:MM:SS */
export function fmtClock(epochMs: number): string {
  const d = new Date(epochMs);
  return `${p2(d.getHours())}:${p2(d.getMinutes())}:${p2(d.getSeconds())}`;
}

/** wall clock HH:MM */
export function fmtClockShort(epochMs: number): string {
  const d = new Date(epochMs);
  return `${p2(d.getHours())}:${p2(d.getMinutes())}`;
}

/** elapsed H:MM:SS.d — the number that must agree across every track */
export function fmtElapsed(ms: number, tenths = false): string {
  const neg = ms < 0;
  const t = Math.abs(ms);
  const h = Math.floor(t / 3_600_000);
  const m = Math.floor((t % 3_600_000) / 60_000);
  const s = Math.floor((t % 60_000) / 1000);
  const base = h > 0 ? `${h}:${p2(m)}:${p2(s)}` : `${p2(m)}:${p2(s)}`;
  const frac = tenths ? `.${Math.floor((t % 1000) / 100)}` : "";
  return `${neg ? "-" : ""}${base}${frac}`;
}

export function fmtDuration(ms: number): string {
  const h = Math.floor(ms / 3_600_000);
  const m = Math.round((ms % 3_600_000) / 60_000);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return `${Math.round(ms / 1000)}s`;
}

export function fmtBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(0)} MB`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(0)} KB`;
  return `${n} B`;
}

export function fmtDateLong(epochMs: number): string {
  const d = new Date(epochMs);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}`;
}

export function fmtTimeAmPm(epochMs: number): string {
  const d = new Date(epochMs);
  const h = d.getHours();
  const ampm = h >= 12 ? "PM" : "AM";
  const h12 = h === 0 ? 12 : h > 12 ? h - 12 : h;
  return `${h12}:${p2(d.getMinutes())} ${ampm}`;
}
