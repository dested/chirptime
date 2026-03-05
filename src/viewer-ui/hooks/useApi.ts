import type { DateEntry, MonitorConfig } from "../types";

export async function fetchDates(): Promise<string[]> {
  const res = await fetch("/api/dates");
  const data = await res.json();
  return data.dates;
}

export async function fetchDateEntries(date: string): Promise<DateEntry> {
  const res = await fetch(`/api/dates/${date}`);
  return res.json();
}

export async function fetchMonitorConfig(): Promise<MonitorConfig> {
  const res = await fetch("/api/monitor-config");
  return res.json();
}
