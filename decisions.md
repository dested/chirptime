# ChirpTime — Decisions

> Why things are the way they are. A recorded decision is settled unless the
> user reopens it. Newest first.
>
> Entries below dated 2026-03-xx were reconstructed from the code on 2026-07-27
> when this file was created; the reasoning is inferred from the implementation.

## 2026-07-27 — Tailwind v4 over the inline `<style>` block in index.html
**Why:** the previous build carried ~800 lines of hand-written CSS inside
`index.html`, which meant no scoping, no reuse between the widget and the viewer,
and every class name being invented twice. Tailwind's `@tailwindcss/vite` plugin
needs no config file, so the migration cost was one `@import` in `app.css`.
**Rejected:** CSS modules (another build concept for a two-file frontend),
styled-components (runtime cost in a video-heavy view), keeping the inline block
(unmaintainable at its current size).
**Consequence:** all styling is utility classes inline in `CompactApp.tsx` /
`ViewerApp.tsx`; only keyframes, resets, and scrollbars stay in `app.css`.

## 2026-03-07 — Auto-detect monitors via PowerShell instead of trusting monitor-config.json
**Why:** the hand-maintained `monitor-config.json` went stale whenever a display
was added, moved, or rescaled, silently breaking per-monitor zoom.
`System.Windows.Forms.Screen::AllScreens` gives the real layout, and rescaling by
`3840 / virtualDesktopWidth` maps it exactly onto FFmpeg's scaled output.
**Rejected:** a Rust winapi enumeration crate (extra dependency for one call),
asking the user to draw regions in the UI (the old flow — nobody kept it current).
**Consequence:** `monitor-config.json` is now only a fallback, and
`get_monitor_config` pays a PowerShell spawn per call.

## 2026-03-07 — Viewer is a separate window opened on demand, not a route in the widget
**Why:** the widget is 320×540 and always-on-top; browsing an archive of video
needs 1280×800 and must not drag the always-on-top pill around with it. Tauri can
build a second webview against the same bundle for free.
**Rejected:** resizing the main window between modes (loses the always-on-top pill
identity), a single window with an in-app router (unusable at 320px).
**Consequence:** `?mode=viewer` in `main.tsx` chooses the root component, and
deep links (`&date=&ts=`) are passed as query params by `open_viewer`. Every new
window label must also be added to `capabilities/default.json`.

## 2026-03-05 — Serve media over a local Warp server on :9120, not the Tauri asset protocol
**Why:** video scrubbing needs HTTP `Range` responses. A tiny Warp server in its
own thread gives byte-range support, correct MIME types, and normal `<video src>`
semantics, with a canonicalize check to block path traversal outside the output dir.
**Rejected:** Tauri's asset protocol / `convertFileSrc` (awkward Range support),
base64 over IPC (unusable for multi-MB clips).
**Consequence:** port 9120 is hardcoded in both `lib.rs` and `hooks/mediaUrl.ts`,
and the frontend is useless if that server fails to bind.

## 2026-03-05 — Shell out to FFmpeg rather than link a capture library
**Why:** one binary already handles screen (gdigrab), webcam+audio (dshow),
encoding, burned-in timestamps, and PiP compositing. Three parallel `Command`
spawns keep the screenshot and both videos in sync, and each capture is
independently debuggable by copying the args into a terminal.
**Rejected:** `ffmpeg-next` bindings (heavy build, no gdigrab convenience),
`screenshots`/`scrap` crates (no video, no webcam).
**Consequence:** FFmpeg must be on PATH (or in two hardcoded Windows locations),
and every spawn must use `silent_command()` so `CREATE_NO_WINDOW` suppresses the
console flash — which would otherwise appear in the capture itself.

## 2026-03-05 — Windows-only, deliberately
**Why:** gdigrab, dshow, the Win32 `Beep` chirp, `CREATE_NO_WINDOW`, and
PowerShell monitor detection are all Windows APIs, and this is a single-user tool
on a Windows machine. Abstracting them would triple the capture code for zero
current users.
**Rejected:** an x11grab/avfoundation abstraction layer (speculative), Electron +
web `getDisplayMedia` (no background capture without a visible prompt).

## 2026-03-05 — Tauri app supersedes the Bun CLI; the CLI stays in-tree
**Why:** the original `src/index.ts` + `src/viewer.ts` CLI proved the capture
pipeline and is kept as a working reference, but the Tauri app owns the product.
Deleting it wasn't worth the churn while the Rust port was still settling.
**Rejected:** deleting it immediately (reference value), maintaining both (double
work).
**Consequence:** `src/index.ts`, `src/capture/`, `src/utils/`, `src/viewer.ts`,
and `src/viewer-ui/` are legacy with no `package.json` script pointing at them,
and `src/ui/App.tsx` + `src/ui/components/` are dead leftovers of the first Tauri
viewer. Do not extend any of them.
