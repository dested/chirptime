import { runFFmpeg } from "../utils/ffmpeg.js";
import { videoPath, type DayPaths } from "../utils/storage.js";
import { detectFirstAudioDevice } from "../utils/ffmpeg.js";

export interface RecordResult {
  type: "screen" | "webcam" | "combined";
  filePath: string;
  success: boolean;
  error?: string;
}

/** drawtext filter that burns a live timestamp into the top-left corner */
const TIMESTAMP_FILTER = "drawtext=text='%{localtime\\:%Y-%m-%d %H\\\\\\:%M\\\\\\:%S}':fontsize=24:fontcolor=white:borderw=2:bordercolor=black:x=10:y=10";

/**
 * Record screen video via gdigrab (captures entire virtual desktop).
 * Scales down to 3840 wide max so VLC doesn't choke on huge multi-monitor resolutions.
 */
export async function recordScreen(
  dayPaths: DayPaths,
  timestamp: string,
  durationSeconds: number,
  framerate: number,
  preset: string
): Promise<RecordResult> {
  const outPath = videoPath(dayPaths, timestamp, "screen");
  const timeoutMs = (durationSeconds + 10) * 1000;

  try {
    const { exitCode, stderr } = await runFFmpeg(
      [
        "-f", "gdigrab",
        "-framerate", String(framerate),
        "-i", "desktop",
        "-t", String(durationSeconds),
        "-vf", `scale=3840:-2,${TIMESTAMP_FILTER}`,
        "-c:v", "libx264",
        "-preset", preset,
        "-crf", "23",
        "-pix_fmt", "yuv420p",
        "-y",
        outPath,
      ],
      timeoutMs
    );

    if (exitCode !== 0) {
      return { type: "screen", filePath: outPath, success: false, error: `FFmpeg exited ${exitCode}: ${stderr.slice(-500)}` };
    }
    return { type: "screen", filePath: outPath, success: true };
  } catch (err) {
    return { type: "screen", filePath: outPath, success: false, error: err instanceof Error ? err.message : String(err) };
  }
}

/**
 * Record webcam video via dshow.
 */
export async function recordWebcam(
  dayPaths: DayPaths,
  timestamp: string,
  deviceName: string,
  durationSeconds: number,
  preset: string,
  recordAudio: boolean
): Promise<RecordResult> {
  const outPath = videoPath(dayPaths, timestamp, "webcam");
  const timeoutMs = (durationSeconds + 10) * 1000;

  try {
    const args: string[] = [];

    // Video input
    args.push("-f", "dshow");

    // Build input string - video only or video+audio
    let inputStr = `video=${deviceName}`;
    if (recordAudio) {
      const audioDevice = await detectFirstAudioDevice();
      if (audioDevice) {
        inputStr += `:audio=${audioDevice}`;
      }
    }
    args.push("-i", inputStr);

    args.push(
      "-t", String(durationSeconds),
      "-vf", TIMESTAMP_FILTER,
      "-c:v", "libx264",
      "-preset", preset,
      "-crf", "23",
      "-pix_fmt", "yuv420p",
    );

    if (recordAudio) {
      args.push("-c:a", "aac", "-b:a", "128k");
    }

    args.push("-y", outPath);

    const { exitCode, stderr } = await runFFmpeg(args, timeoutMs);

    if (exitCode !== 0) {
      return { type: "webcam", filePath: outPath, success: false, error: `FFmpeg exited ${exitCode}: ${stderr.slice(-500)}` };
    }
    return { type: "webcam", filePath: outPath, success: true };
  } catch (err) {
    return { type: "webcam", filePath: outPath, success: false, error: err instanceof Error ? err.message : String(err) };
  }
}

/**
 * Combine screen + webcam into a single video with webcam PiP in the top-right corner.
 * Runs as a post-process after both recordings finish.
 */
export async function combinePiP(
  screenPath: string,
  webcamPath: string,
  dayPaths: DayPaths,
  timestamp: string,
  preset: string
): Promise<RecordResult> {
  const outPath = videoPath(dayPaths, timestamp, "combined");
  try {
    const { exitCode, stderr } = await runFFmpeg(
      [
        "-i", screenPath,
        "-i", webcamPath,
        "-filter_complex",
        "[1:v]scale=320:-2[pip];[0:v][pip]overlay=W-w-10:10",
        "-c:v", "libx264",
        "-preset", preset,
        "-crf", "23",
        "-pix_fmt", "yuv420p",
        "-c:a", "copy",
        "-shortest",
        "-y",
        outPath,
      ],
      120_000
    );

    if (exitCode !== 0) {
      return { type: "combined", filePath: outPath, success: false, error: `FFmpeg exited ${exitCode}: ${stderr.slice(-500)}` };
    }
    return { type: "combined", filePath: outPath, success: true };
  } catch (err) {
    return { type: "combined", filePath: outPath, success: false, error: err instanceof Error ? err.message : String(err) };
  }
}

/**
 * Record both screen and webcam in parallel, then combine into PiP.
 * Uses Promise.allSettled so partial failure is OK.
 */
export async function recordAll(
  dayPaths: DayPaths,
  timestamp: string,
  webcamDevice: string | null,
  durationSeconds: number,
  framerate: number,
  preset: string,
  recordAudio: boolean
): Promise<RecordResult[]> {
  const tasks: Promise<RecordResult>[] = [
    recordScreen(dayPaths, timestamp, durationSeconds, framerate, preset),
  ];

  if (webcamDevice) {
    tasks.push(recordWebcam(dayPaths, timestamp, webcamDevice, durationSeconds, preset, recordAudio));
  }

  const settled = await Promise.allSettled(tasks);
  const results: RecordResult[] = settled.map((r) => {
    if (r.status === "fulfilled") return r.value;
    return { type: "screen" as const, filePath: "", success: false, error: r.reason?.message ?? String(r.reason) };
  });

  // If both screen and webcam succeeded, combine into PiP
  const screenResult = results.find((r) => r.type === "screen" && r.success);
  const webcamResult = results.find((r) => r.type === "webcam" && r.success);
  if (screenResult && webcamResult) {
    const pipResult = await combinePiP(screenResult.filePath, webcamResult.filePath, dayPaths, timestamp, preset);
    results.push(pipResult);
  }

  return results;
}
