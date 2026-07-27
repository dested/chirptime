# ChirpTime — CliffNotes

> Living map of the project. Read this before any coding session.
> Last updated: 2026-07-27. Visual language → `ui.md` · verification → `verify.md`

## What this is

ChirpTime is a Windows-only background life recorder: it chirps, then grabs a
screenshot plus a short screen+webcam video clip every N minutes, filing
everything under `output/YYYY-MM-DD/`. A small always-on-top widget (320×540,
Loom-style) is the whole app; a separate viewer window opens on demand to browse
the archive by calendar → day gallery → media viewer with per-monitor zoom.

Tauri v2 shell, React 19 + Tailwind v4 frontend, Rust backend that shells out to
FFmpeg for everything capture-related.

## Quick Reference

- **Dev:** `bun run dev` (Tauri dev — spawns Vite on http://localhost:5173)
- **Entry point:** `src/ui/main.tsx` → `CompactApp` or `ViewerApp` by `?mode=`
- **Rust entry:** `src-tauri/src/main.rs` → `chirptime_lib::run()` in `lib.rs`
- **Type-check:** `bunx tsc --noEmit` (frontend) · `cargo check` in `src-tauri/`
- **Build:** `bun run build` (Tauri release bundle)
- **Test:** no test runner configured
- **Media URLs:** everything under `output/` is served at
  `http://localhost:9120/files/<relative/path>` by an in-process Warp server —
  the Tauri asset protocol is NOT used.
- **On-disk timestamps use `HH-MM-SS`** (dashes); the UI's `display_time` is just
  that string with dashes swapped for colons.
- **Windows only** — gdigrab, dshow, PowerShell, and the Win32 `Beep` API.

## Stack

| Layer | Choice | Notes |
| --- | --- | --- |
| Runtime / pkg mgr | bun | `bun.lock` is authoritative (`package-lock.json` is incidental) |
| Desktop shell | Tauri v2 | `src-tauri/tauri.conf.json`, capabilities in `src-tauri/capabilities/` |
| Frontend | React 19 + TypeScript | Vite 6, `jsx: react-jsx`, strict mode |
| Styling | Tailwind CSS v4 | via `@tailwindcss/vite`; no config file — `@import "tailwindcss"` in `app.css` |
| Backend | Rust (tokio, warp, chrono, serde) | crate `chirptime`, lib `chirptime_lib` |
| Capture engine | FFmpeg subprocesses | gdigrab (screen), dshow (webcam/audio) |
| Media serving | Warp on port 9120 | own thread + tokio runtime, Range support |
| State | React `useState` only | no store; Rust `Arc<Mutex<AppState>>` is the source of truth |

## Directory structure

```
chirptime/
├── index.html                  Vite entry — bare shell, mounts /src/ui/main.tsx
├── vite.config.ts              React + Tailwind plugins, port 5173 strict
├── package.json                Scripts: dev/build (tauri), vite:dev/vite:build
├── monitor-config.json         Fallback monitor layout (auto-detection is preferred)
├── CLIFFNOTES.md · ui.md · updates.md · decisions.md · verify.md
│
├── src/ui/                     THE app frontend
│   ├── main.tsx                Reads ?mode=viewer, renders CompactApp or ViewerApp
│   ├── app.css                 Tailwind import + base resets, .dot-active, scrollbars
│   ├── CompactApp.tsx          Widget: titlebar, status, start/stop, recents, browse
│   ├── ViewerApp.tsx           Viewer: calendar → gallery → media viewer (self-contained)
│   ├── types.ts                All shared TS interfaces (mirror the Rust serde structs)
│   └── hooks/
│       ├── useApi.ts           Thin invoke() wrapper per Tauri command
│       └── mediaUrl.ts         rel path → http://localhost:9120/files/...
│
├── src/ui/App.tsx              DEAD — old viewer shell, nothing imports it
├── src/ui/components/          DEAD — Layout/ScreenView/Timeline/DatePicker/
│                               MonitorSelector/WebcamWindow/CaptureControls,
│                               only reachable from the dead App.tsx
│
├── src/index.ts                LEGACY Bun CLI entry (no package.json script anymore)
├── src/viewer.ts               LEGACY Bun web viewer server
├── src/capture/, src/utils/    LEGACY CLI capture + helpers
├── src/viewer-ui/              LEGACY viewer React app (Bun.build)
│
├── src-tauri/
│   ├── Cargo.toml              tauri 2, warp, tokio, chrono, serde, urlencoding
│   ├── tauri.conf.json         main window 320×540, undecorated, always-on-top
│   ├── capabilities/default.json  Permissions — lists windows ["main","viewer"]
│   └── src/
│       ├── main.rs             Calls chirptime_lib::run()
│       ├── lib.rs              Spawns the file-server thread, registers 17 commands
│       ├── state.rs            CaptureConfig / CaptureStatus / AppState + defaults
│       ├── commands.rs         All #[tauri::command] handlers
│       ├── capture.rs          chirp, monitor detection, FFmpeg capture cycle
│       └── fileserver.rs       Warp /files/<tail> with Range + traversal guard
│
└── output/                     Capture archive (gitignored)
    └── YYYY-MM-DD/
        ├── screenshots/HH-MM-SS.png
        ├── video/HH-MM-SS_{screen,webcam,combined}.mp4
        └── webcam/HH-MM-SS.jpg   (legacy CLI only — the Tauri app makes the dir, never fills it)
```

## File map (concept → path)

| Concept / task | Location |
| --- | --- |
| Which UI renders | `src/ui/main.tsx` |
| Widget UI + recording controls | `src/ui/CompactApp.tsx` |
| Calendar / gallery / media viewer | `src/ui/ViewerApp.tsx` |
| Frontend↔backend calls | `src/ui/hooks/useApi.ts` |
| Shared types | `src/ui/types.ts` (must match `state.rs` + `commands.rs` serde) |
| Global CSS / animations | `src/ui/app.css` |
| Add/modify a Tauri command | `src-tauri/src/commands.rs` + register in `src-tauri/src/lib.rs` |
| FFmpeg args, chirp, monitor detection | `src-tauri/src/capture.rs` |
| Capture defaults (interval, duration, output dir) | `src-tauri/src/state.rs` → `CaptureConfig::default()` |
| Media HTTP serving / MIME / Range | `src-tauri/src/fileserver.rs` |
| Window size, decorations, always-on-top | `src-tauri/tauri.conf.json` |
| Window permissions (new window labels!) | `src-tauri/capabilities/default.json` |

## Routes / URLs

There is no router. The frontend branches on the query params of a single page.

| URL | Serves | File |
| --- | --- | --- |
| `/` (window `main`) | Compact widget | `src/ui/main.tsx` → `CompactApp.tsx` |
| `/?mode=viewer` (window `viewer`) | Viewer, calendar view | `ViewerApp.tsx` (`view === "calendar"`) |
| `/?mode=viewer&date=YYYY-MM-DD` | Viewer, day gallery | `ViewerApp.tsx` (`view === "gallery"`) |
| `/?mode=viewer&date=…&ts=HH-MM-SS` | Viewer, that clip open | `ViewerApp.tsx` (`view === "viewer"`) |
| `http://localhost:9120/files/<date>/screenshots/HH-MM-SS.png` | Screenshot bytes | `fileserver.rs` |
| `http://localhost:9120/files/<date>/video/HH-MM-SS_combined.mp4` | Video bytes (Range-capable) | `fileserver.rs` |

The viewer URL is assembled by `open_viewer` in `commands.rs`.

## Architecture

```
CompactApp / ViewerApp ──invoke()──► commands.rs ──► capture.rs ──► ffmpeg.exe
        │                                 │                            │
        │                       AppState (Arc<Mutex<…>>)               ▼
        └──<img>/<video> src──► :9120 warp ────────────────────► output/YYYY-MM-DD/
```

Two windows share one bundle. All mutable app state (config, status, the capture
loop's cancel channel) lives in a single `Arc<Mutex<AppState>>` managed by Tauri;
the frontend keeps no durable state and polls `get_capture_status` every 3s.
Media never crosses the IPC bridge — it's fetched over HTTP from the local Warp
server, which is what makes video seeking (Range requests) work.

## Key types

| Type | Where | Purpose |
| --- | --- | --- |
| `CaptureConfig` | `state.rs` / `types.ts` | interval, video mode/duration/fps/preset, audio, PiP, devices, output dir |
| `CaptureStatus` | `state.rs` / `types.ts` | `running`, `last_capture_time`, `next_capture_time`, `captures_today` |
| `AppState` | `state.rs` | config + status + `capture_cancel: Option<oneshot::Sender<()>>` |
| `CaptureEntry` | `commands.rs` / `types.ts` | one timestamp: screenshot + screen/webcam/combined video (relative paths) |
| `DateEntries` | `commands.rs` / `types.ts` | a day's entries (BTreeMap-sorted) + optional `*_full.mp4` |
| `MonitorConfig` / `MonitorRegion` | `commands.rs` / `types.ts` | monitor rects **in the 3840px-wide scaled space**; `camelCase` on the wire |
| `DetectedMonitor` | `capture.rs` | PowerShell-detected screen, pre-mapped into that scaled space |

## Systems

### Capture cycle — `src-tauri/src/capture.rs` (`run_capture_cycle`)

1. `chirp()` — two-tone Win32 `Beep` (1200Hz 80ms → 1600Hz 120ms) on a detached thread.
2. Ensure `output/YYYY-MM-DD/{screenshots,video,webcam}`.
3. `find_ffmpeg()` — PATH, then `C:\ffmpeg\bin`, then `C:\Program Files\ffmpeg\bin`.
4. Auto-pick the first dshow video device (webcam) and first audio device unless configured.
5. **Spawn screenshot + screen video + webcam video threads simultaneously** so the
   three assets stay in sync. Screen and webcam get a burned-in `drawtext` localtime
   stamp; screen is scaled `3840:-2`.
6. If both videos succeeded and `pip_overlay` is on, a second FFmpeg pass overlays
   the webcam at 320px wide, top-right → `HH-MM-SS_combined.mp4`.
7. Update `last_capture_time`, `captures_today`, and `next_capture_time`.

### Capture loop — `commands.rs::start_capture`

Runs one cycle immediately, then `tokio::spawn`s a `tokio::time::interval` loop
`select!`ed against a `oneshot` cancel receiver stored in `AppState.capture_cancel`.
`stop_capture` fires that channel.

### Monitor detection — `capture.rs::detect_monitors`

Shells out to PowerShell (`System.Windows.Forms.Screen::AllScreens`), computes the
virtual-desktop bounding box, and rescales every monitor rect by `3840 / vd_width`
so the regions line up with FFmpeg's scaled output. `get_monitor_config` always
prefers this; `monitor-config.json` and a single 3840×2160 rect are fallbacks.

### Monitor zoom — `ViewerApp.tsx` (`zoomStyle`)

To zoom one monitor, the media element is stretched with `object-fit: fill` to
exactly fill an absolutely-positioned wrapper sized to the whole virtual desktop,
then offset so the chosen rect centers in the container — so 1 wrapper CSS px == 1
unit of `totalWidth`/`totalHeight`. Playback position survives zoom changes via
`savedTimeRef` plus a `loadedmetadata` listener.

### File server — `fileserver.rs`

Warp on 127.0.0.1:9120, on its own thread with its own tokio runtime (started in
`lib.rs` before the Tauri builder). Serves `/files/<tail>` from the *current*
output dir, honours `Range` for video seeking, sets `Access-Control-Allow-Origin: *`,
and refuses paths that canonicalize outside the output dir.

### Windowing — `commands.rs::open_viewer`

Builds a `viewer` webview window (1280×800) at `/?mode=viewer&…`. If it already
exists: focus it when no date was passed, otherwise close + 100ms sleep + rebuild
so the new deep link takes effect.

## Common tasks (how to modify)

### Add a Tauri command
1. Write the `#[tauri::command] pub async fn` in `src-tauri/src/commands.rs`.
2. Add it to `tauri::generate_handler![…]` in `src-tauri/src/lib.rs`.
3. Add a typed wrapper in `src/ui/hooks/useApi.ts` (and the type in `src/ui/types.ts`).

### Change capture behaviour
FFmpeg args live in `capture.rs::run_capture_cycle`; the defaults (10 min interval,
5s clips, 30fps, `ultrafast`, PiP on, audio on) live in
`state.rs::CaptureConfig::default()`.

### Add a window
Build it in a command, then add its label to `windows` in
`src-tauri/capabilities/default.json` — otherwise every `@tauri-apps/api` call
from that window is denied.

## Gotchas & hard rules

- **`CaptureConfig::default().output_dir` is hardcoded to `G:/code/screener/output`.**
  Machine-specific, and nothing persists a user-chosen directory yet.
- **Nothing is persisted.** Config and status live only in memory — restarting the
  app resets the interval, the output dir, and `captures_today` (which also never
  rolls over at midnight).
- **`start_capture` snapshots `interval_minutes` when spawning the loop.** Changing
  the interval mid-run does nothing until stop → start.
- **Timestamp formats differ by surface:** filenames `HH-MM-SS`, `display_time`
  `HH:MM:SS`, status fields `HH:MM:SS`. `delete_entry` takes the dashed form.
- **Monitor rects are in the 3840-wide scaled space,** not desktop pixels. Anything
  that changes `scale=3840:-2` must change `detect_monitors` too.
- **`get_monitor_config` shells out to PowerShell on every call** (~100ms+); never
  call it from a render path.
- **`save_monitor_config` writes to the output dir's *parent*,** which is where the
  repo-root `monitor-config.json` lives. Move the output dir and that file moves too.
- **New window labels must be added to `capabilities/default.json`.**
- **Tailwind v4, no config file.** Custom CSS goes in `src/ui/app.css`; there is no
  `tailwind.config.js` to extend.
- **`src/ui/App.tsx` and all of `src/ui/components/` are dead code** — edit
  `CompactApp.tsx` / `ViewerApp.tsx` instead. The `src/index.ts` + `src/viewer*`
  Bun CLI is likewise legacy and has no npm script left.
- **Windows-only APIs everywhere** (`gdigrab`, `dshow`, `Beep`, `CREATE_NO_WINDOW`,
  PowerShell). Every subprocess must go through `silent_command()` or a console
  window flashes on screen — and lands in the capture.
- **Port 9120 is hardcoded** in both `lib.rs` and `src/ui/hooks/mediaUrl.ts`.

## Status

- **Done** — capture cycle (screenshot + screen/webcam video + PiP combine + audio),
  chirp, compact widget, viewer (calendar → gallery → media viewer), per-monitor and
  webcam zoom, playback speed, filmstrip, keyboard shortcuts, delete, deep-linking
  from the widget's recent list, Range-capable file server, automatic monitor
  detection.
- **Uncommitted on `master`** — the Tailwind v4 migration (the previous build carried
  ~800 lines of inline `<style>` in `index.html`), audio recording, burned-in
  timestamps, and monitor auto-detection.
- **Not built** — a settings UI (`get/set_capture_config`, `list_devices`,
  `set_output_dir`, `get_ffmpeg_path` are backend-only with no caller), config
  persistence, the `*_full.mp4` day compile (the viewer reads `full_day_video` but
  nothing produces it), any tests, tray icon / autostart.
- **Next** — decide the fate of the legacy Bun CLI and the dead `src/ui/components/`,
  then add a settings surface so the hardcoded output dir stops being a gotcha.
