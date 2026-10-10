/**
 * Prints the player's day log for a session manifest and checks the rules the
 * fixture was built to exercise. Pure: no app, no browser.
 *
 *   bun scripts/check-log.ts [path/to/session.json]
 *
 * Defaults to the main fixture session (`bun scripts/make-fixture.ts` first).
 */
import { resolve } from "node:path";
import type { SessionManifest } from "../src/ui/types";
import { deriveLog } from "../src/ui/player/log";
import { humanReason, numberList, outageHeadline } from "../src/ui/player/timeline";
import { fmtTimeAmPm } from "../src/ui/session/format";

const path =
  process.argv[2] ?? resolve(import.meta.dir, "..", "fixtures", "sessions", "2026-08-01_10-30-12", "session.json");
const m = (await Bun.file(path).json()) as SessionManifest;
const manifest: SessionManifest = { ...m, cuts: m.cuts ?? [] };

const lines = deriveLog(manifest, { live: false, now: Date.now() });
for (const l of lines) {
  console.log(`${fmtTimeAmPm(l.at).padStart(8)}  ${l.title}${l.sub ? `  ·  ${l.sub}` : ""}`);
}

let failed = 0;
const check = (ok: boolean, what: string) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failed++;
};

check(numberList([1, 2, 3]) === "1–3", "numberList 1–3");
check(numberList([1, 3]) === "1, 3", "numberList 1, 3");
check(numberList([2]) === "2", "numberList single");
check(humanReason("DXGI error 887a0026") === "The display went dark", "887a0026 reads as the display went dark");
check(humanReason("Display was locked or asleep") === "Display was locked or asleep", "recorder sentences pass through");
check(humanReason("ffmpeg exited (exit code: 0xffffffff): boom") === "Recording failed", "stderr reads as recording failed");

if (!process.argv[2]) {
  const titles = lines.map((l) => l.title);
  check(titles[0] === "Started recording", "starts with Started recording");
  check(lines[0]?.sub === "4 screens, camera, mic and system audio", "track summary line");
  check(titles.includes("Away 20m"), "idle gap reads Away 20m");
  check(titles.includes("Screen 4 stopped recording"), "screen 4 stops on its own");
  check(titles.includes("Screens 1–3 stopped recording"), "screens 1–3 grouped");
  check(titles.includes("Screens 1–4 back"), "all four come back together");
  check(titles.filter((t) => t === "Mark").length === 2, "two marks");
  check(lines.some((l) => l.kind === "mark" && l.sub === "Add a note" && l.subMuted === true), "unlabelled mark prompts for a note");
  check(titles[titles.length - 1] === "Stopped recording", "ends with Stopped recording");
  const h = outageHeadline(manifest);
  check(h !== null && h.screens.length === 4, "headline names several screens");

  // a cut over the idle gap removes its line and adds a Cut line with a countdown
  const away = lines.find((l) => l.kind === "away");
  if (away?.range) {
    const now = Date.now();
    const cut: SessionManifest = {
      ...manifest,
      cuts: [{ id: `cut-${now}`, from_epoch_ms: away.range.from, to_epoch_ms: away.range.to, created_epoch_ms: now - 2_000, state: "pending", freed_bytes: 0 }],
    };
    const after = deriveLog(cut, { live: false, now });
    check(!after.some((l) => l.kind === "away"), "a cut away stretch leaves the log");
    const c = after.find((l) => l.kind === "cut");
    check(c?.title === "Cut 20m" && c.sub === "Undo for 8 s", `cut line reads "Cut 20m · Undo for 8 s" (got ${c?.title} · ${c?.sub})`);
  }

  // an unfinished session that is not live reads Interrupted
  const open = deriveLog({ ...manifest, ended_epoch_ms: null }, { live: false, now: Date.now() });
  check(open[open.length - 1]?.title === "Interrupted", "no end and not live reads Interrupted");
}

if (failed) {
  console.error(`${failed} check(s) failed`);
  process.exit(1);
}
