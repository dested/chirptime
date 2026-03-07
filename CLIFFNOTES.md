# ChirpTime - Background Life Recorder

A Windows desktop application that periodically captures screenshots and short video clips (screen + webcam) of your desktop, organized by date. Built as a **Tauri v2** app with a **React** frontend and **Rust** backend.

The app uses a **compact widget UI** (320x540, Loom-style) as the main window, with a separate **viewer window** (1280x800) that opens on demand for browsing and scrubbing through captures. A chirp sound plays before each capture.

## Architecture Overview

The project has two distinct layers:

1. **Tauri Desktop App** (`src-tauri/` + `src/ui/`) - The main application. Rust backend handles capture logic, file serving, and state management. React frontend provides two UIs:
   - **Compact Widget** (`CompactApp.tsx`) - Small always-on-top window for recording control
   - **Viewer** (`ViewerApp.tsx` + `components/Layout.tsx`) - Full-size window for browsing captures

2. **Legacy Bun CLI** (`src/index.ts`, `src/capture/`, `src/utils/`, `src/viewer.ts`, `src/viewer-ui/`) - The original standalone CLI tool. Still functional but being superseded by the Tauri app.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop framework | Tauri v2 |
| Frontend | React 19, TypeScript, Vite 6 |
| Backend | Rust (tokio, warp, chrono, serde) |
| Capture engine | FFmpeg (gdigrab for screen, dshow for webcam) |
| Package manager | bun |
| Platform | Windows only (gdigrab, dshow, Windows Beep API) |

## Project Structure

```
chirptime/
├── src/
│   ├── ui/                    # Tauri React frontend
│   │   ├── main.tsx           # Entry point - routes compact vs viewer mode
│   │   ├── CompactApp.tsx     # Compact widget UI (main window)
│   │   ├── ViewerApp.tsx      # Viewer window (browse captures)
│   │   ├── App.tsx            # (Legacy, unused)
│   │   ├── types.ts           # All TypeScript interfaces
│   │   ├── hooks/
│   │   │   ├── useApi.ts      # Tauri invoke() wrappers for all commands
│   │   │   └── mediaUrl.ts    # Media URL helpers
│   │   └── components/
│   │       ├── Layout.tsx     # Viewer layout with sidebar, video area, controls
│   │       ├── ScreenView.tsx # Screenshot/video display with monitor zoom
│   │       ├── Timeline.tsx   # Horizontal timeline strip of captures
│   │       ├── DatePicker.tsx # Date selector sidebar
│   │       ├── MonitorSelector.tsx  # Monitor region selector
│   │       ├── WebcamWindow.tsx     # Floating draggable webcam PiP
│   │       └── CaptureControls.tsx  # Start/stop capture, status display
│   │
│   ├── index.ts               # Legacy CLI entry (Bun)
│   ├── viewer.ts              # Legacy web viewer server (Bun)
│   ├── capture/               # Legacy capture modules (Bun)
│   ├── utils/                 # Legacy utilities (Bun)
│   └── viewer-ui/             # Legacy viewer React app (Bun.build)
│
├── src-tauri/
│   ├── Cargo.toml             # Rust dependencies (package: chirptime)
│   ├── tauri.conf.json        # Tauri config (compact 320x540 window, no decorations)
│   ├── build.rs
│   └── src/
│       ├── main.rs            # Entry point, calls chirptime_lib::run()
│       ├── lib.rs             # Tauri builder setup, registers all commands
│       ├── state.rs           # AppState, CaptureConfig, CaptureStatus structs
│       ├── commands.rs        # All Tauri command handlers (17 commands incl. open_viewer)
│       ├── capture.rs         # FFmpeg capture logic + chirp sound
│       └── fileserver.rs      # Warp HTTP file server on port 9120
│
├── output/                    # Default capture output directory
│   └── YYYY-MM-DD/
│       ├── screenshots/       # HH-MM-SS.png
│       ├── video/             # HH-MM-SS_screen.mp4, _webcam.mp4, _combined.mp4
│       └── webcam/            # HH-MM-SS.jpg
│
├── package.json
├── vite.config.ts
├── tsconfig.json
└── monitor-config.json        # Auto-generated monitor region definitions
```

## Tauri Commands (Backend API)

All frontend-backend communication goes through `invoke()` calls. Defined in `src-tauri/src/commands.rs`:

| Command | Purpose |
|---------|---------|
| `get_dates` | List available capture dates (YYYY-MM-DD dirs) |
| `get_date_entries` | Get all captures for a date (screenshots, videos) |
| `get_monitor_config` | Get monitor region layout |
| `save_monitor_config` | Save monitor region layout |
| `delete_entry` | Delete all files for a timestamp |
| `delete_file` | Delete a specific file (with path traversal protection) |
| `get_output_dir` / `set_output_dir` | Get/set the output directory |
| `get_capture_config` / `set_capture_config` | Get/set capture settings |
| `start_capture` / `stop_capture` | Start/stop the periodic capture loop |
| `get_capture_status` | Get running state, last/next capture times |
| `run_single_capture` | Execute one capture cycle immediately |
| `get_ffmpeg_path` | Check if FFmpeg is available |
| `list_devices` | List DirectShow video/audio devices |
| `open_viewer` | Open the viewer window (or focus if already open) |

## UI Architecture

### Compact Widget (main window, 320x540)
- Custom titlebar with drag region (no system decorations)
- Always-on-top
- Shows recording status with pulsing indicator
- Big Start/Stop recording button
- "Capture Now" button for instant capture
- Stats: captures today + last capture time
- Recent captures list
- "Browse All Captures" button to open viewer

### Viewer Window (1280x800, opens on demand)
- Dark theme for media viewing
- Date picker sidebar
- Timeline strip for navigating captures
- Full video/screenshot viewer with monitor zoom
- Floating webcam PiP window
- Keyboard shortcuts (1-9 for monitors, arrows for navigation, Del to delete)

## Capture Flow (Rust)

Defined in `src-tauri/src/capture.rs`:

1. **Chirp** - Play two-tone ascending beep via Windows Beep API (1200Hz + 1600Hz)
2. Create day directories (`YYYY-MM-DD/{screenshots,video,webcam}`)
3. Find FFmpeg (PATH or common Windows locations)
4. Auto-detect webcam via dshow device listing
5. Capture screenshot via `ffmpeg -f gdigrab -i desktop` (scaled to 3840px wide)
6. If video mode: record screen and webcam in parallel threads
7. If both succeed and PiP enabled: combine into overlay video
8. Update capture status (last time, next time, count)
9. On Windows: uses `CREATE_NO_WINDOW` flag to hide console windows

## Key Design Decisions

- **Compact-first UI**: Main window is a small widget, viewer opens separately
- **Chirp before capture**: Two-tone ascending beep alerts user before recording
- **All timestamps use HH-MM-SS format** (dashes not colons) for filesystem compatibility
- **Videos scaled to 3840px wide** regardless of actual resolution
- **Monitor config** defines rectangular regions within the captured desktop for per-monitor zoom
- **Default capture interval**: 10 minutes
- **Default video duration**: 5 seconds per clip
- **Webcam auto-detection**: picks first dshow video device
- **File server separate from Tauri**: Warp runs on port 9120 for Range request support

## Development

```bash
bun install          # Install dependencies
bun run dev          # Start Tauri dev mode (Vite + Rust)
bun run build        # Build production Tauri app
```

Requires:
- Bun (package manager + Vite runner)
- Rust toolchain (for Tauri backend)
- FFmpeg in PATH (for capture functionality)
