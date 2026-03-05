import ffmpegPath from "ffmpeg-static";

export function getFFmpegPath(): string {
  if (!ffmpegPath) {
    throw new Error("ffmpeg-static binary not found. Run 'bun install' first.");
  }
  return ffmpegPath;
}

export interface FFmpegResult {
  exitCode: number;
  stderr: string;
}

export async function runFFmpeg(args: string[], timeoutMs = 60_000): Promise<FFmpegResult> {
  const proc = Bun.spawn([getFFmpegPath(), ...args], {
    stdout: "ignore",
    stderr: "pipe",
    stdin: "ignore",
  });

  const exitCode = await Promise.race([
    proc.exited,
    new Promise<never>((_, reject) =>
      setTimeout(() => {
        proc.kill();
        reject(new Error(`FFmpeg timed out after ${timeoutMs}ms`));
      }, timeoutMs)
    ),
  ]);

  const stderr = await new Response(proc.stderr).text();
  return { exitCode, stderr };
}

export interface DshowDevices {
  video: string[];
  audio: string[];
}

export async function listDshowDevices(): Promise<DshowDevices> {
  // FFmpeg always exits with code 1 for device listing - that's expected
  const proc = Bun.spawn(
    [getFFmpegPath(), "-list_devices", "true", "-f", "dshow", "-i", "dummy"],
    { stdout: "ignore", stderr: "pipe", stdin: "ignore" }
  );
  await proc.exited;
  const stderr = await new Response(proc.stderr).text();

  const video: string[] = [];
  const audio: string[] = [];

  for (const line of stderr.split("\n")) {
    // Match lines like: [dshow @ ...] "Device Name" (video)
    const match = line.match(/\[dshow.*?\]\s+"(.+?)"\s+\((video|audio)\)/);
    if (match) {
      if (match[2] === "video") video.push(match[1]);
      else audio.push(match[1]);
    }
  }

  return { video, audio };
}

let cachedWebcam: string | null | undefined;
let cachedAudio: string | null | undefined;

export async function detectFirstWebcam(): Promise<string | null> {
  if (cachedWebcam !== undefined) return cachedWebcam;
  const devices = await listDshowDevices();
  cachedWebcam = devices.video[0] ?? null;
  cachedAudio = devices.audio[0] ?? null;
  return cachedWebcam;
}

export async function detectFirstAudioDevice(): Promise<string | null> {
  if (cachedAudio !== undefined) return cachedAudio;
  const devices = await listDshowDevices();
  cachedWebcam = devices.video[0] ?? null;
  cachedAudio = devices.audio[0] ?? null;
  return cachedAudio;
}
