import { mkdir } from "node:fs/promises";
import path from "node:path";

export interface DayPaths {
  root: string;
  screenshots: string;
  webcam: string;
  video: string;
}

export async function ensureDayDirectory(baseDir: string, date: Date): Promise<DayPaths> {
  // Use local date, not UTC (toISOString is UTC which causes wrong date folders at night)
  const dateStr = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
  const root = path.join(baseDir, dateStr);
  const screenshots = path.join(root, "screenshots");
  const webcam = path.join(root, "webcam");
  const video = path.join(root, "video");

  await Promise.all([
    mkdir(screenshots, { recursive: true }),
    mkdir(webcam, { recursive: true }),
    mkdir(video, { recursive: true }),
  ]);

  return { root, screenshots, webcam, video };
}

export function generateTimestamp(date: Date): string {
  const h = String(date.getHours()).padStart(2, "0");
  const m = String(date.getMinutes()).padStart(2, "0");
  const s = String(date.getSeconds()).padStart(2, "0");
  return `${h}-${m}-${s}`;
}

export function screenshotPath(dayPaths: DayPaths, timestamp: string, monitorIndex: number): string {
  return path.join(dayPaths.screenshots, `${timestamp}_monitor-${monitorIndex}.png`);
}

export function stitchedScreenshotPath(dayPaths: DayPaths, timestamp: string): string {
  return path.join(dayPaths.screenshots, `${timestamp}_all-monitors.png`);
}

export function webcamPhotoPath(dayPaths: DayPaths, timestamp: string): string {
  return path.join(dayPaths.webcam, `${timestamp}.jpg`);
}

export function videoPath(dayPaths: DayPaths, timestamp: string, type: "screen" | "webcam" | "combined"): string {
  return path.join(dayPaths.video, `${timestamp}_${type}.mp4`);
}

export async function checkDiskSpace(dir: string): Promise<{ freeBytes: number; ok: boolean }> {
  try {
    const resolved = path.resolve(dir);
    const driveLetter = resolved.charAt(0);
    const proc = Bun.spawn(["powershell", "-Command", `(Get-PSDrive ${driveLetter}).Free`], {
      stdout: "pipe",
      stderr: "ignore",
    });
    await proc.exited;
    const text = await new Response(proc.stdout).text();
    const freeBytes = parseInt(text.trim(), 10);
    if (isNaN(freeBytes)) return { freeBytes: Infinity, ok: true };
    const MIN_FREE = 500 * 1024 * 1024; // 500MB
    return { freeBytes, ok: freeBytes > MIN_FREE };
  } catch {
    // If we can't check, assume OK
    return { freeBytes: Infinity, ok: true };
  }
}
