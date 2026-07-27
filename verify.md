# ChirpTime — Verify

> How to prove the app works. Scale to the change: [cheap] always,
> flow recipes when touched, [heavy] only after asking.

## Commands

| What | Command | Cost |
| --- | --- | --- |
| Frontend type-check | `bunx tsc --noEmit` | [cheap] |
| Rust check | `cargo check --manifest-path src-tauri/Cargo.toml` | [medium] — first run compiles the whole dep tree |
| Frontend-only build | `bun run vite:build` | [cheap] |
| Dev app | `bun run dev` → widget window + Vite on http://localhost:5173 | [medium] — spawns a Rust build and a real window |
| Release bundle | `bun run build` | [heavy — ask first] — full optimized Rust build + installer |
| FFmpeg present? | `ffmpeg -version` | [cheap] |
| Media server up? | `curl -I http://localhost:9120/files/` (needs the app running) | [cheap] |

No test runner is configured — verification here is by running the app.

## Environment / data

- **FFmpeg** must be on PATH (confirmed present: 7.1.1-full_build) or at
  `C:\ffmpeg\bin\ffmpeg.exe` / `C:\Program Files\ffmpeg\bin\ffmpeg.exe`.
- **Capture archive** lives at `output/` (gitignored) and already contains real
  days going back to 2026-02-27 — good fixture data for viewer work; no seeding
  needed. Deleting from the viewer deletes real files, permanently.
- **Output directory** is `CaptureConfig::default().output_dir`, currently
  hardcoded to `G:/code/screener/output` in `src-tauri/src/state.rs`.
- No credentials or secrets of any kind.

## Critical flows

### Widget starts and stops recording [medium]
Touchpoints: `CompactApp.tsx`, `commands.rs` (`start_capture`/`stop_capture`/
`get_capture_status`), `capture.rs`, `state.rs`

1. `bun run dev` → the 320×540 widget appears, always on top, status chip reads **Idle**.
2. Click **Start Recording** → chirp sounds, chip turns red and reads **Recording**
   with a pulsing dot, "Next capture in" counts down from the interval.
3. Within a few seconds a new row appears in **Recent**, and the **Today** stat
   increments. A new `output/<today>/screenshots/HH-MM-SS.png` plus
   `output/<today>/video/HH-MM-SS_{screen,webcam,combined}.mp4` exist on disk.
4. Click **Stop Recording** → chip returns to Idle, countdown disappears.

Expect **no console window to flash** at any point — if one does, a subprocess is
bypassing `silent_command()`.

### Capture Now [cheap] (with the app already running)
Touchpoints: `commands.rs::run_single_capture`, `capture.rs::run_capture_cycle`

1. Click **Capture Now** → chirp, button disables for the clip duration (~5s).
2. Today's count increments and a new Recent row appears with the right timestamp.

### Browse the archive [cheap] (with the app already running)
Touchpoints: `ViewerApp.tsx`, `commands.rs` (`get_dates`/`get_date_entries`), `fileserver.rs`

1. Click **Browse All Captures** → viewer window opens on the calendar.
2. Days with captures render as white cards with a sky capture count; empty days
   are inert.
3. Click a day → gallery of thumbnails with `Video` / `Cam` / `Photo` badges.
   Thumbnails loading at all proves the :9120 server is serving.
4. Click a card → media viewer autoplays the combined clip; the filmstrip
   highlights the current cell; ← / → move between captures; `Escape` steps back.

### Per-monitor zoom [cheap] (multi-monitor machines)
Touchpoints: `ViewerApp.tsx` (`zoomStyle`), `capture.rs::detect_monitors`, `commands.rs::get_monitor_config`

1. Open any clip in the media viewer with more than one monitor detected.
2. Press `1`…`9` (or click a numbered segment) → the view crops to exactly that
   monitor, **with no letterboxing or squashed aspect** and playback continuing
   from the same position.
3. Press `W` → webcam-only clip. Press `0` / `` ` `` / **All** → back to full desktop.

### Deep link from the widget [cheap]
Touchpoints: `CompactApp.tsx`, `commands.rs::open_viewer`

1. With the viewer closed, click a row in the widget's **Recent** list.
2. The viewer opens straight into the media viewer on that exact capture.
3. Click another Recent row → the viewer window closes and reopens on the new one.

### Delete a capture [medium — destructive, deletes real files]
Touchpoints: `commands.rs::delete_entry`, `ViewerApp.tsx`, `CompactApp.tsx`

1. In the media viewer press `Delete` (or the trash button) → `confirm()` names the time.
2. Accept → the entry leaves the filmstrip, the neighbour is selected, and all four
   files for that timestamp are gone from `output/<date>/`.

Use a throwaway capture you just took, never an archive day.
