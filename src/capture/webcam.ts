import { runFFmpeg } from "../utils/ffmpeg.js";

export async function captureWebcamPhoto(
  deviceName: string,
  outputPath: string
): Promise<{ success: boolean; error?: string }> {
  // -ss 2 skips first 2 seconds so webcam can warm up (auto-exposure/focus)
  const { exitCode, stderr } = await runFFmpeg(
    [
      "-f", "dshow",
      "-i", `video=${deviceName}`,
      "-ss", "2",
      "-vframes", "1",
      "-q:v", "2",
      "-y",
      outputPath,
    ],
    15_000 // 15s timeout - webcam init can be slow
  );

  if (exitCode !== 0) {
    return { success: false, error: `FFmpeg exited ${exitCode}: ${stderr.slice(-500)}` };
  }
  return { success: true };
}
