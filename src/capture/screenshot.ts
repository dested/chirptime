import path from "node:path";
import { runFFmpeg } from "../utils/ffmpeg.js";
import { type DayPaths } from "../utils/storage.js";

export interface ScreenshotResult {
  filePath: string;
  success: boolean;
  error?: string;
}

/**
 * Capture a screenshot of the entire virtual desktop via FFmpeg gdigrab.
 * Produces a single PNG with all monitors, scaled to 3840 wide max.
 */
export async function captureScreenshot(
  dayPaths: DayPaths,
  timestamp: string
): Promise<ScreenshotResult> {
  const filePath = path.join(dayPaths.screenshots, `${timestamp}.png`);

  try {
    const { exitCode, stderr } = await runFFmpeg(
      [
        "-f", "gdigrab",
        "-framerate", "1",
        "-i", "desktop",
        "-vframes", "1",
        "-vf", "scale=3840:-2",
        "-y",
        filePath,
      ],
      15_000
    );

    if (exitCode !== 0) {
      return { filePath, success: false, error: `FFmpeg exited ${exitCode}: ${stderr.slice(-500)}` };
    }
    return { filePath, success: true };
  } catch (err) {
    return { filePath, success: false, error: err instanceof Error ? err.message : String(err) };
  }
}
