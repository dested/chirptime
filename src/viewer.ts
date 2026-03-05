import path from "node:path";
import { readdir, stat, writeFile, readFile } from "node:fs/promises";

const OUTPUT_DIR = path.resolve("./output");
const UI_DIR = path.resolve("./src/viewer-ui");
const CONFIG_PATH = path.resolve("./monitor-config.json");

// ── Build React bundle ──────────────────────────────────────────────
async function buildBundle(): Promise<string> {
  const result = await Bun.build({
    entrypoints: [path.join(UI_DIR, "app.tsx")],
    outdir: path.join(UI_DIR, ".build"),
    minify: true,
    sourcemap: "inline",
    target: "browser",
  });

  if (!result.success) {
    console.error("Build failed:");
    for (const log of result.logs) {
      console.error(log);
    }
    throw new Error("Bun.build() failed");
  }

  // Read the built file
  const outFile = result.outputs[0];
  return await outFile.text();
}

// ── API: List available dates ───────────────────────────────────────
async function getDates(): Promise<string[]> {
  let entries: string[];
  try {
    entries = await readdir(OUTPUT_DIR);
  } catch {
    return [];
  }

  const dates: string[] = [];
  for (const name of entries) {
    // Match YYYY-MM-DD pattern
    if (/^\d{4}-\d{2}-\d{2}$/.test(name)) {
      const s = await stat(path.join(OUTPUT_DIR, name));
      if (s.isDirectory()) dates.push(name);
    }
  }

  return dates.sort().reverse();
}

// ── API: Get entries for a date ─────────────────────────────────────
interface CaptureEntry {
  timestamp: string;
  displayTime: string;
  screenVideo?: string;
  webcamVideo?: string;
  combinedVideo?: string;
  screenshot?: string;
}

async function getDateEntries(date: string) {
  const dayDir = path.join(OUTPUT_DIR, date);
  const screenshotDir = path.join(dayDir, "screenshots");
  const videoDir = path.join(dayDir, "video");
  const webcamDir = path.join(dayDir, "webcam");

  const map = new Map<string, CaptureEntry>();

  const getOrCreate = (ts: string): CaptureEntry => {
    if (!map.has(ts)) {
      const display = ts.replace(/-/g, ":");
      map.set(ts, { timestamp: ts, displayTime: display });
    }
    return map.get(ts)!;
  };

  // Scan screenshots
  try {
    const files = await readdir(screenshotDir);
    for (const f of files) {
      const m = f.match(/^(\d{2}-\d{2}-\d{2})\.(png|jpg)$/);
      if (m) {
        getOrCreate(m[1]).screenshot = `${date}/screenshots/${f}`;
      }
    }
  } catch {}

  // Scan videos
  try {
    const files = await readdir(videoDir);
    for (const f of files) {
      const m = f.match(/^(\d{2}-\d{2}-\d{2})_(screen|webcam|combined)\.mp4$/);
      if (m) {
        const entry = getOrCreate(m[1]);
        const relPath = `${date}/video/${f}`;
        if (m[2] === "screen") entry.screenVideo = relPath;
        else if (m[2] === "webcam") entry.webcamVideo = relPath;
        else if (m[2] === "combined") entry.combinedVideo = relPath;
      }
    }
  } catch {}

  // Scan webcam dir (for photo-only captures)
  try {
    const files = await readdir(webcamDir);
    for (const f of files) {
      const m = f.match(/^(\d{2}-\d{2}-\d{2})\.(png|jpg)$/);
      if (m) {
        // Only add if no webcam video already
        const entry = getOrCreate(m[1]);
        if (!entry.webcamVideo) {
          // Store as screenshot-like reference
        }
      }
    }
  } catch {}

  const entries = Array.from(map.values()).sort((a, b) =>
    a.timestamp.localeCompare(b.timestamp)
  );

  // Check for full day video
  let fullDayVideo: string | undefined;
  try {
    const dayFiles = await readdir(dayDir);
    const fullVid = dayFiles.find((f) => f.endsWith("_full.mp4"));
    if (fullVid) fullDayVideo = `${date}/${fullVid}`;
  } catch {}

  return { date, entries, fullDayVideo };
}

// ── API: Monitor config ─────────────────────────────────────────────
interface MonitorRegion {
  id: number;
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

interface MonitorConfig {
  totalWidth: number;
  totalHeight: number;
  monitors: MonitorRegion[];
}

async function getMonitorConfig(): Promise<MonitorConfig> {
  // Try to read existing config
  try {
    const raw = await readFile(CONFIG_PATH, "utf-8");
    return JSON.parse(raw);
  } catch {}

  // Auto-generate: probe first screen video for dimensions
  let width = 3840;
  let height = 1296;

  try {
    const dates = await getDates();
    outer: for (const date of dates) {
      const videoDir = path.join(OUTPUT_DIR, date, "video");
      try {
        const files = await readdir(videoDir);
        const screenFile = files.find((f) => f.endsWith("_screen.mp4"));
        if (screenFile) {
          const filePath = path.join(videoDir, screenFile);
          const proc = Bun.spawn(
            [
              "ffprobe",
              "-v", "error",
              "-select_streams", "v:0",
              "-show_entries", "stream=width,height",
              "-of", "csv=p=0",
              filePath,
            ],
            { stdout: "pipe", stderr: "pipe" }
          );
          await proc.exited;
          const text = (await new Response(proc.stdout).text()).trim();
          const parts = text.split(",");
          if (parts.length === 2) {
            width = parseInt(parts[0]);
            height = parseInt(parts[1]);
          }
          break outer;
        }
      } catch {}
    }
  } catch {}

  // Default: divide into 4 equal columns
  const monW = Math.floor(width / 4);
  const config: MonitorConfig = {
    totalWidth: width,
    totalHeight: height,
    monitors: [
      { id: 1, label: "Mon 1", x: 0, y: 0, w: monW, h: height },
      { id: 2, label: "Mon 2", x: monW, y: 0, w: monW, h: height },
      { id: 3, label: "Mon 3", x: monW * 2, y: 0, w: monW, h: height },
      { id: 4, label: "Mon 4", x: monW * 3, y: 0, w: width - monW * 3, h: height },
    ],
  };

  // Write for future use
  await writeFile(CONFIG_PATH, JSON.stringify(config, null, 2));
  console.log(`Generated monitor config: ${width}x${height} → ${CONFIG_PATH}`);

  return config;
}

// ── Start Server ────────────────────────────────────────────────────
async function main() {
  console.log("Building React bundle...");
  const bundleJS = await buildBundle();
  console.log(`Bundle built (${(bundleJS.length / 1024).toFixed(1)}KB)`);

  // Read HTML shell
  const htmlShell = await readFile(path.join(UI_DIR, "index.html"), "utf-8");

  const server = Bun.serve({
    port: 3000,
    async fetch(req) {
      const url = new URL(req.url);
      const pathname = url.pathname;

      // ── API routes ──
      if (pathname === "/api/dates") {
        const dates = await getDates();
        return Response.json({ dates });
      }

      if (pathname.startsWith("/api/dates/")) {
        const date = pathname.slice("/api/dates/".length);
        if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) {
          return Response.json({ error: "Invalid date format" }, { status: 400 });
        }
        const data = await getDateEntries(date);
        return Response.json(data);
      }

      if (pathname === "/api/monitor-config") {
        const config = await getMonitorConfig();
        return Response.json(config);
      }

      // ── Static files from output/ ──
      if (pathname.startsWith("/files/")) {
        const relPath = decodeURIComponent(pathname.slice("/files/".length));
        // Security: prevent path traversal
        const resolved = path.resolve(OUTPUT_DIR, relPath);
        if (!resolved.startsWith(OUTPUT_DIR)) {
          return new Response("Forbidden", { status: 403 });
        }

        const file = Bun.file(resolved);
        const exists = await file.exists();
        if (!exists) {
          return new Response("Not found", { status: 404 });
        }

        // Bun.file() handles Range requests automatically
        return new Response(file);
      }

      // ── App bundle ──
      if (pathname === "/app.js") {
        return new Response(bundleJS, {
          headers: { "Content-Type": "application/javascript" },
        });
      }

      // ── HTML shell (SPA fallback) ──
      if (pathname === "/" || pathname === "/index.html") {
        return new Response(htmlShell, {
          headers: { "Content-Type": "text/html" },
        });
      }

      return new Response("Not found", { status: 404 });
    },
  });

  console.log(`\nScreener Viewer running at http://localhost:${server.port}`);
  console.log(`Serving output from: ${OUTPUT_DIR}\n`);
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
