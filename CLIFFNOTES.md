# Screener - Background Life Recorder

A Windows desktop application that periodically captures screenshots and short video clips (screen + webcam) of your desktop, organized by date. Built as a **Tauri v2** app with a **React** frontend and **Rust** backend.

## Architecture Overview

The project has two distinct layers:

1. **Tauri Desktop App** (`src-tauri/` + `src/ui/`) - The main application. Rust backend handles capture logic, file serving, and state management. React frontend provides a viewer/control UI rendered in a Tauri webview.

2. **Legacy Bun CLI** (`src/index.ts`, `src/capture/`, `src/utils/`, `src/viewer.ts`, `src/viewer-ui/`) - The original standalone CLI tool that runs via `bun run src/index.ts`. Still functional but being superseded by the Tauri app. Uses `ffmpeg-static` npm package and Bun APIs directly.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop framework | Tauri v2 |
| Frontend | React 19, TypeScript, Vite 6 |
| Backend | Rust (tokio, warp, chrono, serde) |
| Capture engine | FFmpeg (gdigrab for screen, dshow for webcam) |
| Package manager | bun |
| Platform | Windows only (gdigrab, dshow, PowerShell calls) |

## Project Structure

```
screener/
├── src/
│   ├── ui/                    # Tauri React frontend (THE ACTIVE UI)
│   │   ├── main.tsx           # React entry point
│   │   ├── App.tsx            # Root component, state management
│   │   ├── types.ts           # All TypeScript interfaces
│   │   ├── hooks/
│   │   │   ├── useApi.ts      # Tauri invoke() wrappers for all commands
│   │   │   └── mediaUrl.ts    # Media URL helpers
│   │   └── components/
│   │       ├── Layout.tsx     # Main layout with sidebar, video area, controls
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
│   │   ├── screenshot.ts      # FFmpeg gdigrab screenshot
│   │   ├── recorder.ts        # Screen + webcam video recording + PiP combine
│   │   └── webcam.ts          # Webcam photo capture
│   ├── utils/
│   │   ├── ffmpeg.ts          # FFmpeg runner, device detection (Bun)
│   │   ├── config.ts          # ScreenerConfig type + defaults
│   │   └── storage.ts         # Day directory structure, paths, disk space
│   ├── viewer-ui/             # Legacy viewer React app (Bun.build)
│   └── types/
│       └── screenshot-desktop.d.ts
│
├── src-tauri/
│   ├── Cargo.toml             # Rust dependencies
│   ├── tauri.conf.json        # Tauri config (window size, plugins, build)
│   ├── build.rs
│   └── src/
│       ├── main.rs            # Entry point, calls lib::run()
│       ├── lib.rs             # Tauri builder setup, registers all commands
│       ├── state.rs           # AppState, CaptureConfig, CaptureStatus structs
│       ├── commands.rs        # All Tauri command handlers (16 commands)
│       ├── capture.rs         # FFmpeg capture logic in Rust (screen, webcam, PiP)
│       └── fileserver.rs      # Warp HTTP file server on port 9120 (serves media)
│
├── output/                    # Default capture output directory
│   └── YYYY-MM-DD/
│       ├── screenshots/       # HH-MM-SS.png
│       ├── video/             # HH-MM-SS_screen.mp4, _webcam.mp4, _combined.mp4
│       └── webcam/            # HH-MM-SS.jpg (photo-only mode)
│
├── package.json
├── vite.config.ts
├── tsconfig.json              # Only includes src/ui
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

## File Server

A Warp HTTP server runs on `localhost:9120` in the background (started in `lib.rs`). It serves media files from the output directory with:
- Path traversal protection
- HTTP Range request support (for video seeking)
- CORS headers
- Proper MIME types

The frontend references media via `http://localhost:9120/files/{relative_path}`.

## Capture Flow (Rust)

Defined in `src-tauri/src/capture.rs`:

1. Create day directories (`YYYY-MM-DD/{screenshots,video,webcam}`)
2. Find FFmpeg (PATH or common Windows locations)
3. Auto-detect webcam via dshow device listing
4. Capture screenshot via `ffmpeg -f gdigrab -i desktop` (scaled to 3840px wide)
5. If video mode: record screen and webcam in parallel threads
6. If both succeed and PiP enabled: combine into overlay video
7. Update capture status (last time, next time, count)
8. On Windows: uses `CREATE_NO_WINDOW` flag to hide console windows

## Key Design Decisions

- **All timestamps use HH-MM-SS format** (dashes not colons) for filesystem compatibility
- **Videos scaled to 3840px wide** regardless of actual resolution
- **Monitor config** defines rectangular regions within the captured desktop for per-monitor zoom
- **Default capture interval**: 10 minutes with mouse-activity detection (CLI only)
- **Default video duration**: 5 seconds per clip
- **Webcam auto-detection**: picks first dshow video device
- **File server separate from Tauri**: Warp runs on its own port because Tauri's asset protocol doesn't handle Range requests well for video

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

## Keyboard Shortcuts (Viewer UI)

- `1-9` - Zoom to monitor region
- `0` / `` ` `` / `Escape` - Show full desktop
- `Left/Right` arrows - Navigate between captures
- `Delete` / `Backspace` - Delete current capture

## State Management

Frontend uses a single `AppState` object in `App.tsx` with `useState`. No external state library. State flows down through props to `Layout` and child components. Capture status is polled every 5 seconds.

Backend state is `Arc<Mutex<AppState>>` in Rust, containing `CaptureConfig`, `CaptureStatus`, and a oneshot channel for cancelling the capture loop.
