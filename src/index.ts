import { parseArgs } from "node:util";
import path from "node:path";
import { readdir, writeFile, unlink } from "node:fs/promises";
import { DEFAULT_CONFIG, type ScreenerConfig } from "./utils/config.js";
import { ensureDayDirectory, generateTimestamp, checkDiskSpace, webcamPhotoPath } from "./utils/storage.js";
import { listDshowDevices, detectFirstWebcam } from "./utils/ffmpeg.js";
import { runFFmpeg } from "./utils/ffmpeg.js";
import { captureScreenshot } from "./capture/screenshot.js";
import { captureWebcamPhoto } from "./capture/webcam.js";
import { recordAll } from "./capture/recorder.js";

// ── CLI Parsing ──────────────────────────────────────────────────────
function parseCliArgs(): ScreenerConfig & { compile?: string } {
  const { values } = parseArgs({
    args: Bun.argv.slice(2),
    options: {
      interval: { type: "string", short: "i" },
      video: { type: "boolean", short: "v", default: true },
      duration: { type: "string", short: "d" },
      once: { type: "boolean", default: false },
      "list-devices": { type: "boolean", default: false },
      compile: { type: "string", short: "c" },
      output: { type: "string", short: "o" },
      webcam: { type: "string" },
      framerate: { type: "string" },
      "no-audio": { type: "boolean", default: false },
      "no-pip": { type: "boolean", default: false },
    },
    strict: true,
  });

  return {
    ...DEFAULT_CONFIG,
    intervalMinutes: values.interval ? parseInt(values.interval, 10) : DEFAULT_CONFIG.intervalMinutes,
    videoMode: values.video ?? DEFAULT_CONFIG.videoMode,
    videoDurationSeconds: values.duration ? parseInt(values.duration, 10) : DEFAULT_CONFIG.videoDurationSeconds,
    once: values.once ?? DEFAULT_CONFIG.once,
    listDevices: values["list-devices"] ?? DEFAULT_CONFIG.listDevices,
    compile: values.compile,
    outputDir: values.output ?? DEFAULT_CONFIG.outputDir,
    webcamDevice: values.webcam ?? DEFAULT_CONFIG.webcamDevice,
    videoFramerate: values.framerate ? parseInt(values.framerate, 10) : DEFAULT_CONFIG.videoFramerate,
    recordAudio: values["no-audio"] ? false : DEFAULT_CONFIG.recordAudio,
    pipOverlay: values["no-pip"] ? false : DEFAULT_CONFIG.pipOverlay,
  };
}

// ── Logging ──────────────────────────────────────────────────────────
function log(msg: string) {
  const ts = new Date().toLocaleTimeString("en-US", { hour12: false });
  console.log(`[${ts}] ${msg}`);
}

function logError(msg: string) {
  const ts = new Date().toLocaleTimeString("en-US", { hour12: false });
  console.error(`[${ts}] ERROR: ${msg}`);
}

// ── List Devices ─────────────────────────────────────────────────────
async function handleListDevices() {
  console.log("\n=== DirectShow Devices ===\n");

  const devices = await listDshowDevices();

  if (devices.video.length === 0 && devices.audio.length === 0) {
    console.log("No DirectShow devices found.");
    console.log("This may mean FFmpeg couldn't enumerate devices.\n");
  } else {
    if (devices.video.length > 0) {
      console.log("Video devices:");
      devices.video.forEach((d, i) => console.log(`  [${i}] ${d}`));
    } else {
      console.log("No video devices found.");
    }

    console.log();

    if (devices.audio.length > 0) {
      console.log("Audio devices:");
      devices.audio.forEach((d, i) => console.log(`  [${i}] ${d}`));
    } else {
      console.log("No audio devices found.");
    }
  }

  console.log();
}

// ── Compile Day ──────────────────────────────────────────────────────
async function compileDay(outputDir: string, date: string) {
  // date is "today", "yesterday", or "YYYY-MM-DD"
  let dateStr: string;
  const localDate = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  if (date === "today") {
    dateStr = localDate(new Date());
  } else if (date === "yesterday") {
    const d = new Date();
    d.setDate(d.getDate() - 1);
    dateStr = localDate(d);
  } else {
    dateStr = date;
  }

  const videoDir = path.join(outputDir, dateStr, "video");
  log(`Compiling ${dateStr}...`);

  let files: string[];
  try {
    const all = await readdir(videoDir);
    files = all
      .filter((f) => f.endsWith("_combined.mp4"))
      .sort();
  } catch {
    logError(`No video directory found at ${videoDir}`);
    return;
  }

  if (files.length === 0) {
    // Fall back to screen-only clips
    try {
      const all = await readdir(videoDir);
      files = all.filter((f) => f.endsWith("_screen.mp4")).sort();
    } catch {
      // already logged
    }
    if (!files || files.length === 0) {
      logError(`No video clips found in ${videoDir}`);
      return;
    }
    log(`  No combined clips found, using screen-only clips`);
  }

  log(`  Found ${files.length} clips to concatenate`);

  // Write concat list file
  const listPath = path.join(videoDir, "_concat.txt");
  const listContent = files.map((f) => `file '${f.replace(/'/g, "'\\''")}'`).join("\n");
  await writeFile(listPath, listContent);

  const outPath = path.join(outputDir, dateStr, `${dateStr}_full.mp4`);

  const { exitCode, stderr } = await runFFmpeg(
    [
      "-f", "concat",
      "-safe", "0",
      "-i", listPath,
      "-filter_complex", "[0:v]setpts=0.5*PTS[v];[0:a]atempo=2.0[a]",
      "-map", "[v]",
      "-map", "[a]",
      "-c:v", "libx264",
      "-preset", "fast",
      "-crf", "23",
      "-c:a", "aac",
      "-y",
      outPath,
    ],
    600_000 // 10 min timeout for big compiles
  );

  await unlink(listPath).catch(() => {});

  if (exitCode !== 0) {
    logError(`Compile failed: ${stderr.slice(-500)}`);
    return;
  }

  // Get file size
  const stat = await Bun.file(outPath).stat();
  const sizeMB = ((stat?.size ?? 0) / 1024 / 1024).toFixed(1);

  log(`Compiled ${files.length} clips → ${outPath} (${sizeMB}MB)`);
}

// ── Mouse Activity Detection ─────────────────────────────────────────
let lastMousePos: { x: number; y: number } | null = null;

async function getMousePos(): Promise<{ x: number; y: number } | null> {
  try {
    const proc = Bun.spawn(
      ["powershell", "-NoProfile", "-Command",
        "Add-Type -AssemblyName System.Windows.Forms; $p = [System.Windows.Forms.Cursor]::Position; \"$($p.X),$($p.Y)\""],
      { stdout: "pipe", stderr: "ignore" }
    );
    await proc.exited;
    const text = (await new Response(proc.stdout).text()).trim();
    const [x, y] = text.split(",").map(Number);
    if (isNaN(x) || isNaN(y)) return null;
    return { x, y };
  } catch {
    return null;
  }
}

async function hasMouseMoved(): Promise<boolean> {
  const pos = await getMousePos();
  if (!pos) return true; // can't detect → assume active
  if (!lastMousePos) {
    lastMousePos = pos;
    return true; // first check → always capture
  }
  const moved = pos.x !== lastMousePos.x || pos.y !== lastMousePos.y;
  lastMousePos = pos;
  return moved;
}

// ── Single Capture Cycle ─────────────────────────────────────────────
async function runCaptureCycle(config: ScreenerConfig, webcamDevice: string | null) {
  const now = new Date();
  const timestamp = generateTimestamp(now);
  const dayPaths = await ensureDayDirectory(config.outputDir, now);

  log(`Starting capture cycle - ${timestamp}`);

  // Disk space check
  const disk = await checkDiskSpace(config.outputDir);
  if (!disk.ok) {
    logError(`Low disk space (${(disk.freeBytes / 1024 / 1024).toFixed(0)}MB free). Skipping cycle.`);
    return;
  }

  // ── Screenshot ──
  const screenshotResult = await captureScreenshot(dayPaths, timestamp);
  if (screenshotResult.success) {
    log(`  Screenshot: captured`);
  } else {
    logError(`  Screenshot failed: ${screenshotResult.error}`);
  }

  // ── Webcam Photo ──
  if (webcamDevice && !config.videoMode) {
    const photoPath = webcamPhotoPath(dayPaths, timestamp);
    const result = await captureWebcamPhoto(webcamDevice, photoPath);
    if (result.success) {
      log(`  Webcam photo: captured`);
    } else {
      logError(`  Webcam photo failed: ${result.error}`);
    }
  }

  // ── Video Mode ──
  if (config.videoMode) {
    log(`  Recording ${config.videoDurationSeconds}s video...`);
    const videoResults = await recordAll(
      dayPaths,
      timestamp,
      webcamDevice,
      config.videoDurationSeconds,
      config.videoFramerate,
      config.videoPreset,
      config.recordAudio
    );

    for (const r of videoResults) {
      if (r.success) {
        log(`  Video (${r.type}): recorded`);
      } else {
        logError(`  Video (${r.type}) failed: ${r.error}`);
      }
    }
  }

  log(`Capture cycle complete.`);
}

// ── Main ─────────────────────────────────────────────────────────────
async function main() {
  const config = parseCliArgs();

  console.log("\n  Screener - Background Life Recorder\n");

  // List devices mode
  if (config.listDevices) {
    await handleListDevices();
    process.exit(0);
  }

  // Compile mode
  if (config.compile) {
    await compileDay(config.outputDir, config.compile);
    process.exit(0);
  }

  // Detect webcam
  let webcamDevice = config.webcamDevice;
  if (!webcamDevice) {
    log("Auto-detecting webcam...");
    webcamDevice = await detectFirstWebcam();
    if (webcamDevice) {
      log(`Found webcam: ${webcamDevice}`);
    } else {
      log("No webcam detected. Webcam captures will be skipped.");
    }
  } else {
    log(`Using webcam: ${webcamDevice}`);
  }

  // Print config summary
  log(`Mode: ${config.videoMode ? `video (${config.videoDurationSeconds}s clips)` : "snapshot"}`);
  log(`Interval: ${config.once ? "once" : `every ${config.intervalMinutes} minutes`}`);
  log(`Output: ${config.outputDir}`);

  // Run once mode
  if (config.once) {
    await runCaptureCycle(config, webcamDevice);
    process.exit(0);
  }

  // ── Interval Loop ──
  // Run immediately, then on interval
  await hasMouseMoved(); // seed initial mouse position
  await runCaptureCycle(config, webcamDevice);

  const intervalMs = config.intervalMinutes * 60 * 1000;
  const intervalId = setInterval(async () => {
    try {
      const active = await hasMouseMoved();
      if (!active) {
        log("No mouse activity since last capture. Skipping.");
        return;
      }
      await runCaptureCycle(config, webcamDevice);
    } catch (err) {
      logError(`Unexpected error in capture cycle: ${err}`);
    }
  }, intervalMs);

  log(`Scheduler running. Next capture in ${config.intervalMinutes} minutes. Press Ctrl+C to stop.\n`);

  // Graceful shutdown
  const shutdown = () => {
    log("Shutting down...");
    clearInterval(intervalId);
    process.exit(0);
  };

  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
