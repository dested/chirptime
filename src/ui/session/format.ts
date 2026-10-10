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

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/** `Saturday, August 1` — the day as a title */
export function fmtDayTitle(epochMs: number): string {
  const d = new Date(epochMs);
  return `${WEEKDAYS[d.getDay()]}, ${MONTHS[d.getMonth()]} ${d.getDate()}`;
}

/** `11:31:04 AM` */
export function fmtClock12(epochMs: number): string {
  const d = new Date(epochMs);
  const h = d.getHours();
  return `${h % 12 || 12}:${p2(d.getMinutes())}:${p2(d.getSeconds())} ${h >= 12 ? "PM" : "AM"}`;
}

/** `11:20` — 12-hour clock with no suffix, for dense ticks */
export function fmtTick(epochMs: number): string {
  const d = new Date(epochMs);
  return `${d.getHours() % 12 || 12}:${p2(d.getMinutes())}`;
}

/** `11 am`, `noon`, `1 pm` — hour labels over the day ribbon */
export function fmtHourLabel(epochMs: number): string {
  const h = new Date(epochMs).getHours();
  if (h === 12) return "noon";
  if (h === 0) return "midnight";
  return h < 12 ? `${h} am` : `${h - 12} pm`;
}

/** `1h 43m`, `8h 05m`, `24m`, `40s` — a span of time in words */
export function fmtSpan(ms: number): string {
  const totalMin = Math.round(ms / 60_000);
  if (totalMin < 1) return `${Math.max(0, Math.round(ms / 1000))}s`;
  const h = Math.floor(totalMin / 60);
  return h ? `${h}h ${p2(totalMin % 60)}m` : `${totalMin}m`;
}

/** `1:00:51` — always with hours, for elapsed / total */
export function fmtHms(ms: number): string {
  const t = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(t / 3600)}:${p2(Math.floor((t % 3600) / 60))}:${p2(t % 60)}`;
}

/** `48.6 GB`, `812 MB` */
export function fmtSize(n: number): string {
  if (n >= 1e12) return `${(n / 1e12).toFixed(1)} TB`;
  if (n >= 1e11) return `${Math.round(n / 1e9)} GB`;
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}
