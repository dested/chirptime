# ChirpTime — CliffNotes

> Living map of the project. Read this before any coding session.
> Last updated: 2026-08-01. Visual language → `ui.md` · verification → `verify.md`

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
- **Test:** `cargo test --lib`, `cargo test --test settings_persistence`, and
  `cargo test --test ffmpeg_install` — **never plain `cargo test`**, which would
  run `tests/live_session.rs` and record. The FFmpeg install's real end-to-end
  test is `#[ignore]`d (it pulls ~110 MB); run it with
  `cargo test --test ffmpeg_install -- --ignored`. No frontend test runner.
- **Settings:** `%APPDATA%\com.chirptime.app\settings.json` — output dir, capture
  config, session options, the first-run flag, and a hand-picked `ffmpeg_dir`.
  Missing/corrupt → defaults.
- **FFmpeg:** not bundled. Found on PATH / `C:\ffmpeg\bin` / a picked folder, or
  fetched on first run into `%APPDATA%\com.chirptime.app\ffmpeg\bin`.
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
├── .github/workflows/release.yml   Tag-triggered (`v*`) Windows release: MSI +
│                               NSIS + portable zip, published with GITHUB_TOKEN.
│                               Separate from the Drydock-owned drydock.yml.
├── README.md                   Public-facing: what it is, download, FFmpeg, disk
├── index.html                  Vite entry — bare shell, mounts /src/ui/main.tsx
├── vite.config.ts              React + Tailwind plugins, port 5173 strict
├── package.json                Scripts: dev/build (tauri), vite:dev/vite:build
├── monitor-config.json         Fallback monitor layout (auto-detection is preferred)
├── CLIFFNOTES.md · ui.md · updates.md · decisions.md · verify.md
│
├── src/ui/                     THE app frontend
│   ├── main.tsx                Reads ?mode=viewer, renders CompactApp or ViewerApp
│   ├── app.css                 Tailwind import + base resets, .dot-active, scrollbars
│   ├── CompactApp.tsx          Widget: Clips | Session mode switch; clips pane
│   │                           (interval capture) and session pane (continuous
│   │                           recording + track health + inline settings)
│   ├── ViewerApp.tsx           Viewer: calendar → gallery → media viewer (self-contained)
│   ├── types.ts                All shared TS interfaces (mirror the Rust serde structs)
│   ├── hooks/
│   │   ├── useApi.ts           Thin invoke() wrapper per Tauri command
│   │   └── mediaUrl.ts         rel path → http://localhost:9120/files/...
│   └── session/                Continuous-session browser + multi-track player
│       ├── engine.ts           SyncEngine + TrackController — master clock, resync loop
│       ├── SessionPlayer.tsx   Layouts (grid/focus/overlay), transport, sync panel
│       ├── TrackSurface.tsx    One track's two <video> elements (visible + pre-roll)
│       ├── Timeline.tsx        Session-wide scrubber: segments, gaps, markers
│       ├── SessionBrowser.tsx  Session list (light surface)
│       └── format.ts           Clock / elapsed / duration / bytes formatting
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
│       ├── lib.rs              Loads settings, spawns the file-server thread, registers 31 commands
│       ├── state.rs            CaptureConfig / CaptureStatus / AppState + defaults
│       ├── settings.rs         Persisted settings (%APPDATA%\<id>\settings.json),
│       │                       tolerant load, atomic save, legacy `output/` detection
│       ├── nativeui.rs         IFileOpenDialog folder picker, open-in-Explorer, disk space
│       ├── commands.rs         All #[tauri::command] handlers
│       ├── ffmpeg_install.rs   First-run FFmpeg fetch: pinned URL + SHA-256,
│       │                       streaming hash, staged extract of ffmpeg/ffprobe
│       │                       only, smoke test, atomic promote
│       ├── capture.rs          chirp, monitor detection, FFmpeg capture cycle
│       ├── recorder.rs         Continuous session recorder: DXGI/encoder resolution,
│       │                       device selection, per-track supervisors, disk guard
│       ├── session.rs          Manifest types, atomic write, ffprobe, disk rebuild
│       ├── sysaudio.rs         WASAPI loopback capture (silence-padded)
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
| First-run onboarding panel | `src/ui/CompactApp.tsx` (`Onboarding`) |
| "Where are my recordings" (path, free space, picker, Explorer) | `src/ui/CompactApp.tsx` (`OutputFolder`) + titlebar folder button |
| What survives a restart | `src-tauri/src/settings.rs` |
| Folder picker / Explorer / disk space | `src-tauri/src/nativeui.rs` |
| Session start/stop, track health, markers, session settings | `src/ui/CompactApp.tsx` (`SessionPane` / `SessionSettings`) |
| Calendar / gallery / media viewer | `src/ui/ViewerApp.tsx` |
| Session list / multi-track player | `src/ui/session/` (mounted from `ViewerApp.tsx`) |
| Playback sync, drift correction, segment swapping | `src/ui/session/engine.ts` |
| Frontend↔backend calls | `src/ui/hooks/useApi.ts` |
| Shared types | `src/ui/types.ts` (must match `state.rs` + `commands.rs` serde) |
| Global CSS / animations | `src/ui/app.css` |
| Add/modify a Tauri command | `src-tauri/src/commands.rs` + register in `src-tauri/src/lib.rs` |
| FFmpeg args, chirp, monitor detection | `src-tauri/src/capture.rs` |
| Capture defaults (interval, duration, output dir) | `src-tauri/src/state.rs` → `CaptureConfig::default()` |
| Media HTTP serving / MIME / Range | `src-tauri/src/fileserver.rs` |
| "FFmpeg is missing" + install flow | `src/ui/CompactApp.tsx` (`FfmpegSetupCard`, `FfmpegBanner`) |
| Where FFmpeg is downloaded from / verified | `src-tauri/src/ffmpeg_install.rs` (the pin is 3 consts) |
| FFmpeg / ffprobe lookup order | `capture::find_ffmpeg` + `session::find_ffprobe` — **keep in step** |
| Window size, decorations, always-on-top | `src-tauri/tauri.conf.json` |
| Installer targets, publisher, version | `src-tauri/tauri.conf.json` → `bundle` |
| Cutting a release | tag `vX.Y.Z` → `.github/workflows/release.yml` |
| Window permissions (new window labels!) | `src-tauri/capabilities/default.json` |

## Routes / URLs

There is no router. The frontend branches on the query params of a single page.

| URL | Serves | File |
| --- | --- | --- |
| `/` (window `main`) | Compact widget | `src/ui/main.tsx` → `CompactApp.tsx` |
| `/?mode=viewer` (window `viewer`) | Viewer, calendar view | `ViewerApp.tsx` (`view === "calendar"`) |
| `/?mode=viewer&date=YYYY-MM-DD` | Viewer, day gallery | `ViewerApp.tsx` (`view === "gallery"`) |
| `/?mode=viewer&date=…&ts=HH-MM-SS` | Viewer, that clip open | `ViewerApp.tsx` (`view === "viewer"`) |
| `/?mode=viewer&tab=sessions` | Session list | `session/SessionBrowser.tsx` |
| `/?mode=viewer&session=<id>` | Multi-track player for that session | `session/SessionPlayer.tsx` |
| `/?mode=viewer&session=<id>&ts=<epoch ms>` | That session, playhead at that instant | `SessionPlayer.initialEpochMs` → `engine.seekTo` |
| `http://localhost:9120/files/sessions/<id>/session.json` | Session manifest | `fileserver.rs` |
| `http://localhost:9120/files/sessions/<id>/<track>/<file>.mp4` | One 60s segment | `fileserver.rs` |
| `http://localhost:9120/files/<date>/screenshots/HH-MM-SS.png` | Screenshot bytes | `fileserver.rs` |
| `http://localhost:9120/files/<date>/video/HH-MM-SS_combined.mp4` | Video bytes (Range-capable) | `fileserver.rs` |

The viewer URL is assembled by `open_viewer` in `commands.rs`, which takes
`date` / `timestamp` / `session` / `tab`. **`ts` is `HH-MM-SS` alongside `date`
and epoch-ms alongside `session`** — `date` vs `session` is the discriminator.

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
| `SessionManifest` / `SessionTrack` / `SessionSegment` | `types.ts` | Continuous-session manifest — the frontend/backend contract (spec §3) |
| `SessionSummary` | `types.ts` | One row in the session list; derivable from a manifest via `summarizeSession` |
| `SyncEngine` / `TrackDebug` | `session/engine.ts` | Master clock + per-track drift state |
| `SessionStatus` / `TrackSummary` | `commands.rs` / `types.ts` | Live recorder health: elapsed, per-track status, segments, bytes, warnings, `disk_free_bytes` / `min_free_bytes` |
| `SessionOptions` | `recorder.rs` / `types.ts` | What `start_session` takes. `#[serde(default)]`, read once at start; `screens` holds `screen_index` values, empty = all |
| `DisplayOutputInfo` | `commands.rs` / `types.ts` | `list_display_outputs` row: DXGI output + the recorder's `screen_index` + a real `capturable` flag |
| `Settings` | `settings.rs` / `types.ts` | Everything persisted: `capture`, `session` (opaque JSON), `onboarded`, `version` |
| `EnvironmentInfo` | `commands.rs` / `types.ts` | What the first-run panel states: resolved output dir, free/total bytes, ffmpeg path, NVENC, legacy `output/`, settings path |
| `LegacyOutput` | `settings.rs` / `types.ts` | A pre-move `output/` that still holds footage — path + day/session counts, reported only |

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

### Multi-track playback sync — `src/ui/session/engine.ts`

Implements layer 3 of the sync strategy in
`plans/2026-07-27-continuous-recording.md` §4. One `SyncEngine` per open session
owns a master virtual clock in **epoch-ms** and a `TrackController` per track.
A single `requestAnimationFrame` loop advances the clock by `dt * rate`, checks
segment boundaries and transport every frame, and runs drift correction every
250ms:

| `|drift|` | action |
| --- | --- |
| > 150ms | hard seek (300ms cooldown so one seek is not re-issued before the decoder answers) |
| 30–150ms | `playbackRate = rate * (1 ∓ 0.03)` |
| ≤ 30ms | leave alone |

`drift = el.currentTime*1000 - (masterEpochMs - segment.start_epoch_ms)`. While
paused the engine snaps instead of nudging (>20ms).

`segmentAt()` binary-searches on each segment's own `start_epoch_ms` /
`duration_ms` — **nothing anywhere computes `index * segment_seconds`**, because
real segment durations are ragged (4s/14s/16s observed for a nominal 15s) and
consecutive segments can overlap by seconds. A hole shorter than
`SEAM_TOLERANCE_MS` (250ms) is treated as a keyframe artifact and rolls onto the
next segment; only a longer hole renders as a gap. Where two segments overlap the
search picks the later one.

**Every track holds two media elements.** The visible one plays; the other
pre-rolls the next segment from 8s before the boundary, so a crossing is an
opacity swap plus a `play()`, not a `load()`. `segmentAt()` binary-searches a
track's segments, returning `null` inside a gap / before the track started /
after it ended — the surface then renders "No footage" and the controller parks
the upcoming segment on the active element so resuming is instant.

The engine exposes two subscriptions: `subscribeFrame` (60Hz, for the imperative
playhead and clock readout) and `subscribeState` (5Hz, for React). The **inter-track
spread** — max minus min of where each element believes it is — is the metric
that says the burned-in timecodes agree; the Sync panel shows it live.

### Inactivity trimming — `recorder.rs` (session recorder only)

The continuous recorder can delete footage of idle time as it records. A
background `idle_poll_loop` samples the **system-wide** last keyboard/mouse input
(`poll_last_input_ms` → Win32 `GetLastInputInfo` + `GetTickCount`) into
`Inner.last_input_ms` every 2s. When a closed segment is about to be indexed,
`index_output_segment` asks `should_drop_idle(last_input, seg_start, threshold)`:
if the whole segment span had no input **and** the idle stretch already exceeded
the threshold before the segment began, the file is `remove_file`d and never
added to the manifest — ffmpeg is left running untouched (the cost being cut is
disk, not CPU). Up to one threshold's worth of footage after the last activity is
always kept. Consecutive drops collapse into one `IdleTail`, flushed by the next
kept segment as a single `"idle — no keyboard or mouse activity"` gap; that
segment chains across the hole via `index_segment`'s `chain_override` so the
drift path doesn't also record a "dropped time" gap. A failed delete falls back
to keeping the segment. Threshold: `SessionOptions.drop_idle_after_seconds`
(default 60, 0 = off); the **interval-clip flow in `capture.rs` is unaffected**.
See decisions.md (2026-08-01) for why delete-on-close beats pause/resume.

### Settings + first run — `settings.rs`, `commands.rs`, `CompactApp.tsx`

`run()` calls `settings::load()` **before** the state, the file server thread or
any window exists, so the very first media request already resolves against the
saved output dir. Every mutating command (`set_output_dir`, `set_capture_config`,
`set_session_options`, `set_onboarded`, `pick_output_dir`) writes the whole file
back through `commands::persist`, which logs a write failure rather than
rejecting the user's change.

The widget reads `get_settings` once at mount and renders **nothing but the
first-run panel** until `onboarded` is true — inline, never a modal (ui.md). The
panel states the resolved absolute path, free space and the hours of recording it
buys, ~5–8 GB/hour, that nothing is ever auto-deleted, and whether FFmpeg and
NVENC actually exist on this machine (`get_environment`). `pick_output_dir` opens
`IFileOpenDialog` owned by the widget's HWND — unowned it would open *behind* the
always-on-top window.

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

- **The default output dir is `%USERPROFILE%\Videos\ChirpTime`** (`state::default_output_dir`),
  not the old hardcoded `G:/code/screener/output`. Anything recorded before that
  change is still on disk and is no longer listed — `settings::find_legacy_output`
  surfaces it in the first-run panel and in settings. **Nothing moves or deletes
  those files; the user decides.**
- **Config persists, status does not.** `settings.json` holds the output dir, the
  capture config, the session options and the onboarding flag; `CaptureStatus`
  (including `captures_today`, which still never rolls over at midnight) is
  memory-only.
- **Settings load can never fail.** `settings::load_from` degrades field by field:
  bad JSON → all defaults, one bad key → that key's default and every other key
  kept. A settings file must never keep the app from starting, and losing the
  output dir to an unrelated bad key would send recordings somewhere unasked for.
- **`SessionOptions` is stored as opaque JSON** (`serde_json::Value`) in both
  `Settings` and `AppState`, because `recorder::SessionOptions` is deserialize-only
  and the recording pipeline is not to be touched for a settings feature. The blob
  is validated by parsing it on load and on `set_session_options`, and the frontend
  merges it over `DEFAULT_OPTIONS` rather than trusting it whole.
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
  Bun CLI is likewise legacy and has no npm script left. `src/ui/session/` is the
  one live multi-file directory; it is not `components/`.
- **`main.tsx` dynamic-imports the two apps.** `CompactApp.tsx` calls
  `getCurrentWindow()` at module scope, which throws outside Tauri — so the
  viewer must never evaluate that module, and the plain `bun run vite:dev`
  browser flow depends on it.
- **The session player never re-renders on the clock.** Playhead and clock text
  are written imperatively from `subscribeFrame`; putting `masterEpochMs` in
  React state re-renders four `<video>` trees at 60Hz.
- **Three 4K/5K decoders cannot sustain 4x.** Grid layout at 4x drops frames and
  the spread blows out to a few hundred ms; 1x and 2x are clean. Use Focus at 4x.
- **Segments are never uniform, contiguous, or non-overlapping.** Measured on real
  sessions: 4s/14s/16s for a nominal 15s, 0–50ms holes at most seams, and 41 seams
  that overlap by up to 4.2s. Anything computing a position from
  `index * segment_seconds` is wrong. `segment_seconds` is metadata only — the
  frontend reads it nowhere.
- **`SessionSummary` in `types.ts` must match `recorder::SessionSummary` exactly.**
  It once carried two invented fields and rendered `undefined + NaN` against the
  real backend.
- **A force-killed recording leaves an unfinalized final segment on disk** (no moov
  atom, unplayable). The manifest correctly omits it. If a rebuild ever lists one,
  the player shows "Segment unreadable" for that track and keeps the rest in sync.
- **Windows-only APIs everywhere** (`gdigrab`, `dshow`, `Beep`, `CREATE_NO_WINDOW`,
  PowerShell). Every subprocess must go through `silent_command()` or a console
  window flashes on screen — and lands in the capture.
- **Port 9120 is hardcoded** in both `lib.rs` and `src/ui/hooks/mediaUrl.ts`.
- **The version lives in three files** — `src-tauri/tauri.conf.json`,
  `src-tauri/Cargo.toml`, `package.json` (all at **1.0.0**). Bump all three before tagging: the
  release workflow names its uploads from the git tag, but Tauri names the
  bundles it produces from `tauri.conf.json`, so a mismatch ships a file whose
  name lies about its contents.
- **The release binary is `chirptime.exe`, not `ChirpTime.exe`.** Cargo names it
  after the crate; `productName` only names the installer and the installed app.
- **Releases are unsigned.** No certificate exists, so SmartScreen warns on
  download and first run. That is expected output, not a broken build.
- **`src-tauri/icons/` holds exactly one 16×16 `icon.ico`.** It builds, but
  Windows upscales it everywhere it matters — the installer, the taskbar, the
  Programs list. A proper multi-size .ico (16/32/48/256) is a real missing asset,
  not a config problem; `bundle.icon` already points at the one file that exists.
- **`find_ffmpeg` and `find_ffprobe` must stay in lockstep.** Both look in the
  same five places, in the same order: hand-picked `settings.ffmpeg_dir` → PATH →
  `C:\ffmpeg\bin` → `C:\Program Files\ffmpeg\bin` → the app-managed install under
  `%APPDATA%`. The recorder resolves both, and an ffmpeg found without a matching
  ffprobe records footage it then cannot index. The managed install is **last**
  on purpose — a machine that already had FFmpeg keeps using the exact binary it
  always used.
- **`ffmpeg_install` never downloads on its own.** `commands::install_ffmpeg` is
  reachable only from a button, and the size is shown before the press. The
  pinned SHA-256 is checked *while* streaming, before anything is extracted or
  executed; a mismatch deletes the file. Do not add a "fetch it automatically if
  missing" path.
- **The pin is three constants that move together** — `FFMPEG_URL`,
  `FFMPEG_SHA256`, `FFMPEG_ZIP_BYTES`. Bumping the version means re-deriving the
  hash; the module doc says how it was cross-checked.
- **`tests/ffmpeg_install.rs` is `#[ignore]`d and must stay that way** — it pulls
  ~110 MB and writes ~200 MB into `%APPDATA%`. Run it deliberately with
  `cargo test --test ffmpeg_install -- --ignored`.
- **`SessionOptions` is read once, when `start_session` spawns the tracks.** The
  settings panel is disabled while recording for that reason; changing anything
  mid-session does nothing until stop → start.
- **`ended_epoch_ms === null` does not mean "recording".** A force-killed session
  never got one written — two of the real sessions on disk are like this. The
  browser cross-checks `get_session_status` and shows *Interrupted* instead.
- **The widget cannot be server-rendered or opened in a plain browser** —
  `CompactApp.tsx` calls `getCurrentWindow()` at module scope. `SessionPane` is
  exported so the session UI can be rendered outside the Tauri shell; the module
  still needs `@tauri-apps/api/window` stubbed first.
- **The webcam and mic are ONE ffmpeg process and must stay that way.** They are
  captured by a single `-f dshow -i "video=X:audio=Y"` input so both streams share
  one DirectShow clock; splitting them into two processes — or even two `-f dshow`
  inputs in the same process — puts the audio up to 1.2s ahead of the picture.
  They remain two tracks and two files (`-map 0:v` / `-map 0:a`); never mux them
  together. See `TrackSource::WebcamMic` and decisions.md.
- **Webcam and mic media lengths differ by ~200ms and that is NOT desync.** Video
  segments cut on keyframes (2s GOP), audio segments cut on clock ticks, so the
  webcam's first segment runs ~2s long and its last ~1.7s short — visible in the
  filenames (mic cuts at `…700`, webcam at `…701`). Measured per segment, the two
  full mid-session segments differ by −6.3ms and +17.0ms with alternating sign
  and no accumulation. Judge A/V sync by the **per-segment start labels**, never
  by comparing total media duration: each segment carries its own
  `start_epoch_ms`, so unequal segment lengths cost nothing at playback.
- **Manifest holes at segment seams are label jitter, NOT lost media — verified,
  and it is a recurring false alarm.** Seam-gap size is uncorrelated with media
  loss. Measured across 7 tracks × 2 sessions: `screen-3` (h264_qsv) has the
  *largest* gaps (55/109/76ms) and **exactly 0.0ms** of loss, while `screen-2`
  (gaps 0–4ms) and every other video track are also 0.0ms. The mic loses
  −6.0/+5.5ms per interior boundary — under half an AAC frame (21.3ms).
  The mechanism: `decide_segment_start` pulls each label 1/DRIFT_ALPHA toward the
  poll-scan file-appearance time, and that observation lags true content start by
  however long the encoder/muxer buffers before its first write (QSV and AAC
  buffer far more than NVENC). The bias lands in the *label*, never in the media.
  Gaps can also be **negative** (measured −31, −42ms) — overlap, which loss cannot
  produce. All observed holes stay under `SEAM_TOLERANCE_MS`, so the player rolls
  through them and nothing renders or is audible.
  **To test this claim, never compare seam gaps.** Compare a track's interior
  media against the wall-clock span its `-strftime` filenames cover (those cut on
  clock ticks, so second granularity is exact), or count packets: two consecutive
  10s mic segments held **469 + 468** AAC packets = 19989ms for 20000ms of wall
  clock. Dropping 110ms/boundary would need ~464 per segment.
- **Segment length is per-OUTPUT, not per-session.** `mic` and `sysaudio` cut at
  `AUDIO_SEGMENT_SECONDS` (600s); everything with a picture uses the session's
  `segment_seconds`. The shared webcam+mic process passes a different
  `-segment_time` for each of its two outputs, and the watchdog is per-output for
  the same reason. `manifest.segment_seconds` is therefore the **video** length,
  not a promise about every track — the player never reads it (it derives
  everything from each segment's own `start_epoch_ms`/`duration_ms`), so only the
  rebuild's unreadable-segment gap width still assumes it.
- **A supervisor can own more than one manifest track.** `TrackSpec.outputs` is a
  list, and the watchdog, gap accounting, status and restart counters all iterate
  it. Anything assuming one process == one track is wrong for the webcam/mic pair.
- **A track that never recorded has zero segments and zero gaps.** A gap spans a
  hole *between* content, so a device that never opened produces none — its
  `restarts` count is the evidence instead. This is deliberate: the manifest never
  claims footage it does not have.
- **`list_display_outputs` refines `capturable` itself.** `enumerate_dxgi_outputs`
  always reports `true` (enumeration cannot know which encoders exist), so never
  read that flag straight off the recorder's own enumeration.
- **Idle gaps in a session manifest are intentional, not failures.** With
  inactivity trimming on (default), the session recorder deletes segments that
  covered a stretch with no keyboard/mouse input and records an `"idle — no
  keyboard or mouse activity"` gap. So a manifest hole can mean "the user was
  away," not "a device died" or "time was lost" — any coverage/health check must
  treat that reason as expected. Trimming is **session-only, Windows-only**, and
  keyboard/mouse-only: passive viewing (video, a call) with no input reads as
  idle and *will* be dropped. Toggle/threshold: the "Inactivity" control in
  `SessionSettings` → `SessionOptions.drop_idle_after_seconds` (0 disables).

## Status

- **Done** — capture cycle (screenshot + screen/webcam video + PiP combine + audio),
  chirp, compact widget, viewer (calendar → gallery → media viewer), per-monitor and
  webcam zoom, playback speed, filmstrip, keyboard shortcuts, delete, deep-linking
  from the widget's recent list, Range-capable file server, automatic monitor
  detection.
- **Uncommitted on `master`** — the Tailwind v4 migration (the previous build carried
  ~800 lines of inline `<style>` in `index.html`), audio recording, burned-in
  timestamps, and monitor auto-detection.
- **Done (2026-07-27)** — config persistence, the first-run onboarding panel, the
  native folder picker, "open my recordings" from the titlebar and settings, and
  legacy-`output/` detection.
- **Done (2026-07-27)** — Tauri bundler configured (MSI + NSIS), tag-triggered
  Windows release workflow, MIT `LICENSE`, a real README, and the first-run
  FFmpeg install (pinned + hash-verified download, or point at your own). The
  workflow has **never run** — the repo has no GitHub remote yet.
- **Not built** — code signing (deliberately: no certificate, installers are
  unsigned and SmartScreen warns), a settings UI for the
  *interval clip* capture
  (`get/set_capture_config`, `get_ffmpeg_path` still have no caller), the
  `*_full.mp4` day compile (the viewer reads `full_day_video` but nothing produces
  it), a frontend test runner, tray icon / autostart.
- **Not yet verified live** — the whole continuous-recording path has only been
  exercised against sessions already on disk. Starting a session, per-track
  health while running, markers, and the settings panel's device pickers have
  never been run against live hardware from the UI.
- **Next** — a supervised live recording to verify the widget end to end
  (including the folder picker and Explorer button, which have only been
  exercised through a mocked-IPC browser harness), then the fate of the legacy
  Bun CLI and the dead `src/ui/components/`.
