import { invoke } from "@tauri-apps/api/core";
import type { DateEntries, MonitorConfig, CaptureConfig, CaptureStatus } from "../types";

export async function fetchDates(): Promise<string[]> {
  return invoke<string[]>("get_dates");
}

export async function fetchDateEntries(date: string): Promise<DateEntries> {
  return invoke<DateEntries>("get_date_entries", { date });
}

export async function fetchMonitorConfig(): Promise<MonitorConfig> {
  return invoke<MonitorConfig>("get_monitor_config");
}

export async function fetchOutputDir(): Promise<string> {
  return invoke<string>("get_output_dir");
}

export async function deleteEntry(date: string, timestamp: string): Promise<void> {
  return invoke("delete_entry", { date, timestamp });
}

export async function getCaptureConfig(): Promise<CaptureConfig> {
  return invoke<CaptureConfig>("get_capture_config");
}

export async function setCaptureConfig(config: CaptureConfig): Promise<void> {
  return invoke("set_capture_config", { config });
}

export async function startCapture(): Promise<void> {
  return invoke("start_capture");
}

export async function stopCapture(): Promise<void> {
  return invoke("stop_capture");
}

export async function getCaptureStatus(): Promise<CaptureStatus> {
  return invoke<CaptureStatus>("get_capture_status");
}

export async function runSingleCapture(): Promise<string> {
  return invoke<string>("run_single_capture");
}

export async function listDevices(): Promise<{ video: string[]; audio: string[] }> {
  return invoke<{ video: string[]; audio: string[] }>("list_devices");
}

export async function openViewer(): Promise<void> {
  return invoke("open_viewer");
}
