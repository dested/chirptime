/**
 * Stand-in for the app's Warp file server (`fileserver.rs`), for the browser dev
 * harness: serves `fixtures/` on http://localhost:9120 with the same two routes.
 *
 *   GET /files/<rel path>                          bytes, with Range
 *   GET /thumb/<session>/<track>/<epoch_ms>.jpg?w=  one JPEG frame (S1b contract)
 *
 * Refuses to start when 9120 is taken: that means the real app is running, and
 * the UI should be talking to it instead.
 *
 *   bun scripts/fixture-server.ts
 */
import { mkdir, rename, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join, resolve, sep } from "node:path";
import type { SessionManifest } from "../src/ui/types";
import { segmentAt } from "../src/ui/session/engine";
import { trackHoles } from "../src/ui/session/cuts";

const PORT = 9120;
const ROOT = resolve(import.meta.dir, "..", "fixtures");
const SESSIONS = join(ROOT, "sessions");
const ID_RE = /^[A-Za-z0-9_-]+$/;
const MAX_FFMPEG = 3;

const CORS = { "Access-Control-Allow-Origin": "*" };

function notFound(why: string): Response {
  return new Response(why, { status: 404, headers: CORS });
}

const MIME: Record<string, string> = {
  mp4: "video/mp4",
  json: "application/json",
  jpg: "image/jpeg",
  png: "image/png",
};

/* ---- /files ------------------------------------------------------------- */

async function serveFile(req: Request, rel: string): Promise<Response> {
  const path = resolve(ROOT, decodeURIComponent(rel));
  if (!path.startsWith(ROOT + sep)) return new Response("outside the fixtures", { status: 403, headers: CORS });
  const file = Bun.file(path);
  if (!(await file.exists())) return notFound("no such file");
  const size = file.size;
  const type = MIME[path.split(".").pop() ?? ""] ?? "application/octet-stream";
  const range = req.headers.get("range");
  const m = range ? /^bytes=(\d*)-(\d*)$/.exec(range) : null;
  if (!m) {
    return new Response(file, {
      headers: { ...CORS, "Content-Type": type, "Accept-Ranges": "bytes", "Cache-Control": "no-store" },
    });
  }
  let start = m[1] ? Number(m[1]) : size - Number(m[2]);
  let end = m[1] && m[2] ? Number(m[2]) : size - 1;
  start = Math.max(0, start);
  end = Math.min(size - 1, end);
  if (start > end) {
    return new Response(null, { status: 416, headers: { ...CORS, "Content-Range": `bytes */${size}` } });
  }
  return new Response(file.slice(start, end + 1), {
    status: 206,
    headers: {
      ...CORS,
      "Content-Type": type,
      "Accept-Ranges": "bytes",
      "Content-Range": `bytes ${start}-${end}/${size}`,
      "Content-Length": String(end - start + 1),
    },
  });
}

/* ---- /thumb ------------------------------------------------------------- */

let running = 0;
const queue: (() => void)[] = [];

async function withSlot<T>(fn: () => Promise<T>): Promise<T> {
  if (running >= MAX_FFMPEG) await new Promise<void>((r) => queue.push(r));
  running++;
  try {
    return await fn();
  } finally {
    running--;
    queue.shift()?.();
  }
}

/** Same frames for the same request: one extraction even when ten arrive at once. */
const inflight = new Map<string, Promise<boolean>>();

async function extract(segment: string, offsetMs: number, width: number, out: string): Promise<boolean> {
  await mkdir(resolve(out, ".."), { recursive: true });
  const tmp = `${out}.${process.pid}.tmp.jpg`;
  const ok = await withSlot(async () => {
    const p = Bun.spawn(
      [
        "ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
        "-ss", (offsetMs / 1000).toFixed(3),
        "-i", segment,
        "-frames:v", "1",
        "-vf", `scale=${width}:-2`,
        "-q:v", "6",
        tmp,
      ],
      { stdout: "ignore", stderr: "ignore" },
    );
    return (await p.exited) === 0 && existsSync(tmp);
  });
  if (!ok) return false;
  await rename(tmp, out);
  return true;
}

async function serveThumb(sessionId: string, trackId: string, epochName: string, url: URL): Promise<Response> {
  const em = /^(\d+)\.jpg$/.exec(epochName);
  if (!ID_RE.test(sessionId) || !ID_RE.test(trackId) || !em) return notFound("bad thumb path");
  const epochMs = Number(em[1]);
  const w = Math.min(960, Math.max(64, Number(url.searchParams.get("w") ?? 320) || 320));

  const dir = join(SESSIONS, sessionId);
  const manifestFile = Bun.file(join(dir, "session.json"));
  if (!(await manifestFile.exists())) return notFound("no such session");
  const m = (await manifestFile.json()) as SessionManifest;
  const track = m.tracks.find((t) => t.id === trackId);
  if (!track) return notFound("no such track");
  const hit = segmentAt(track, epochMs, trackHoles({ ...m, cuts: m.cuts ?? [] }, trackId));
  if (!hit) return notFound("no footage at that instant");

  const out = join(dir, "thumbs", trackId, String(w), `${epochMs}.jpg`);
  if (!existsSync(out)) {
    const key = out;
    let job = inflight.get(key);
    if (!job) {
      const seg = join(dir, track.dir, track.segments[hit.index].file);
      job = extract(seg, hit.offsetMs, w, out).finally(() => inflight.delete(key));
      inflight.set(key, job);
    }
    if (!(await job)) return notFound("could not read a frame there");
  }
  const s = await stat(out);
  return new Response(Bun.file(out), {
    headers: { ...CORS, "Content-Type": "image/jpeg", "Content-Length": String(s.size), "Cache-Control": "max-age=3600" },
  });
}

/* ---- server ------------------------------------------------------------- */

if (!existsSync(SESSIONS)) {
  console.error(`No fixtures at ${SESSIONS}. Run: bun scripts/make-fixture.ts`);
  process.exit(1);
}

try {
  Bun.serve({
    port: PORT,
    hostname: "127.0.0.1",
    async fetch(req) {
      const url = new URL(req.url);
      if (req.method === "OPTIONS") return new Response(null, { headers: { ...CORS, "Access-Control-Allow-Headers": "range" } });
      if (url.pathname.startsWith("/files/")) return serveFile(req, url.pathname.slice("/files/".length));
      const t = /^\/thumb\/([^/]+)\/([^/]+)\/([^/]+)$/.exec(url.pathname);
      if (t) return serveThumb(t[1], t[2], t[3], url);
      return notFound("unknown route");
    },
  });
} catch (e) {
  console.error(`Port ${PORT} is taken, so the real ChirpTime is probably running. Not starting. (${String(e)})`);
  process.exit(1);
}
console.log(`fixture server: http://localhost:${PORT}/files/ and /thumb/ from ${ROOT}`);
