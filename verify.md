# ChirpTime — Verify

> How to prove the app works. Scale to the change: [cheap] always, flows when
> touched, [heavy] only after asking. Nothing here reads or writes
> `C:\Users\dested\Videos\ChirpTime` except the live checks Sal runs himself.

## Commands

| What | Command | Cost |
| --- | --- | --- |
| Frontend type-check | `bunx tsc --noEmit` | [cheap] |
| Frontend build | `bun run vite:build` | [cheap] |
| Library / follow / gap rules | `bun scripts/check-library.ts` (pure, no fixtures) | [cheap] |
| Day-log rules | `bun scripts/check-log.ts` (needs the fixture) | [cheap] |
| Rust check | `cargo check --manifest-path src-tauri/Cargo.toml` | [medium] |
| Rust unit tests | `cargo test --lib --manifest-path src-tauri/Cargo.toml` | [medium], about 150 pure tests |
| Delete / undo / finalize | `cargo test --test edit_cuts --manifest-path src-tauri/Cargo.toml` | [medium], synthetic sessions |
| Thumbnails | `cargo test --test thumbs --manifest-path src-tauri/Cargo.toml` | [medium] |
| Settings | `cargo test --test settings_persistence --manifest-path src-tauri/Cargo.toml` | [cheap], temp `%APPDATA%` |
| Export | `cargo test --test export --manifest-path src-tauri/Cargo.toml` | [medium], includes one small real-ffmpeg export |
| FFmpeg install (real download) | `cargo test --test ffmpeg_install -- --ignored` | [heavy — ask first], pulls about 110 MB |
| Release bundle | `bun run build` | [heavy — ask first] |

- **Never run plain `cargo test`.** It builds and lists `tests/live_session.rs`,
  which records the real screens, camera and mic when run with `--ignored`.
  Run the suites above by name.
- While Sal's `tauri dev` is running, build in your own target dir
  (`CARGO_TARGET_DIR=src-tauri/target-<you>`) so you don't fight its lock.
- `rebuild_check` (`--ignored`) is a read-only audit of the real archive. Ask before running it.

## Browser harness (no Tauri, no recording)

Fixtures only. It is the way to check any screen.

1. `bun scripts/make-fixture.ts` builds `G:\code\chirptime\fixtures\sessions\`
   (gitignored, about 370 MB). `--force` rebuilds it. If you change the
   fixtures, restart the fixture server.
2. `bun scripts/fixture-server.ts` serves `/files` (with Range) and
   `/thumb/<sid>/<track>/<epoch>.jpg?w=` on 9120. It refuses to start if 9120 is
   taken, which means the real app is running: leave it alone.
3. `bun run vite:dev` starts Vite on 7460.
4. Open the pages below. `?mock=1` (DEV only) answers `invoke()` from the
   fixtures, through `src/ui/dev/mockIpc.ts` plus the `dev/extra/*` handlers.
   Deletes, notes, exports and recorder actions change an in-memory copy, with
   the real undo and finalize timings.
5. Stop Vite and the fixture server when you're done.

App window (`?mode=viewer&mock=1`):
- Library: http://localhost:7460/?mode=viewer&mock=1#/library
- Player: http://localhost:7460/?mode=viewer&mock=1#/session/2026-08-01_10-30-12
- Export: http://localhost:7460/?mode=viewer&mock=1#/export/2026-08-01_10-30-12
- Marks: http://localhost:7460/?mode=viewer&mock=1#/marks
- Exports: http://localhost:7460/?mode=viewer&mock=1#/exports
- Settings: http://localhost:7460/?mode=viewer&mock=1#/settings
- First run: http://localhost:7460/?mode=viewer&mock=1&scene=firstrun#/library
- No FFmpeg found: http://localhost:7460/?mode=viewer&mock=1&scene=noffmpeg#/settings

Recorder widget (`?mock=1`, at 320×540). `?scene=` comes from `src/ui/dev/extra/widget.ts`:
- Idle: http://localhost:7460/?mock=1
- Recording, with screen 4 restarting: http://localhost:7460/?mock=1&scene=recording
- Outage alert, screens 1–3 down for 14 min: http://localhost:7460/?mock=1&scene=alert
- Paused for 3 min: http://localhost:7460/?mock=1&scene=paused
- First run (not onboarded, so the widget sends you to the app): http://localhost:7460/?mock=1&scene=firstrun
- No FFmpeg: http://localhost:7460/?mock=1&scene=noffmpeg

bx pins 1280×800. For 1920×1080, use a playwright viewport. In DEV,
`window.__chirpEngine` seeks the Player.

### The fixture: 7 sessions (`scripts/make-fixture.ts`)

| Id | Shape | What it exercises |
| --- | --- | --- |
| `2026-07-30_09-12-05` | 2h 30m, 1 screen + camera + mic | Idle 70–95 min; mark "Standup notes" at 31 min |
| `2026-07-31_13-05-40` | 1h 15m, 1 screen + camera + mic | A `"paused"` gap at 35–47 min; a mark with no note at 60 min |
| `2026-08-01_10-30-12` | about 2 h (10:30–12:32), 4 screens + camera + mic + system audio | The main Player session. Idle 11:10–11:30 on every picture track; screen 4 down 11:45–12:10 ("stopped reporting", shown as "The display went dark"); screens 1–3 down 11:52–12:10 ("Display was locked or asleep"); marks at 10:52 (no note) and 12:18 ("Bug repro") |
| `2026-08-02_09-04-40` | 3 min, one 5120×2160 screen + camera + mic | A short single-screen session |
| `2026-08-02_14-10-02` | 30 s | Short-session clean-up |
| `2026-08-02_14-11-09` | 20 s | Short-session clean-up (pressed Start twice) |
| `2026-08-03_23-20-30` | 1h 15m | Crosses midnight; mark "After midnight" |

## Flows to click through (harness)

- **Delete a block of time (Player):**
  1. Drag on the ribbon, or press I / O, or use Delete on an away line in the log.
  2. The selection bar shows the length chip, the range and the size. Press X.
  3. The confirm row reads "Delete 1h 43m? Frees … now. Undo for 10 seconds."
  4. Press Enter. The range collapses to a red seam, and the toast counts down.
  5. Undo (or Ctrl Z) brings it back. Let it run out and it finalizes at 12 s.
- **Library:**
  1. Click a block to open the inspector. Double-click to open the Player.
  2. Delete the session, then Undo from the toast.
  3. "N sessions under a minute · Clean up" removes the two short fixtures with one undo.
- **Marks:** edit a note inline (Enter saves, Esc cancels). Open seeks the Player to the mark.
- **Export:**
  1. Change the layout, screen, camera corner (drag it) and speed. The preview
     and the estimate update.
  2. Export. Progress runs in the footer, then "Saved · Show in folder", and
     the row appears on `#/exports`.
- **Widget:**
  1. Start, then Mark with an inline note, Pause / Resume, the pill and back, and Stop.
  2. The alert scene's "Restart screens" works.
  3. Options change only the next recording.

## Live checks (Sal)

These have only run in the harness and the tests. They need the real app
(`bun run dev` or a release build) and real hardware, and Sal runs them.

- [ ] **Tray:**
  - [ ] The icon shows idle, recording and paused.
  - [ ] The menu's Start/Stop, Pause/Resume, Mark, Open ChirpTime, Show recorder and Quit all work.
  - [ ] Closing the widget while recording hides it to the tray.
- [ ] **Hotkeys:**
  - [ ] Ctrl Alt R, M and P work from another app.
  - [ ] A hotkey another app already holds shows its error in Settings, and the app still starts.
  - [ ] A double press never starts and then stops.
- [ ] **Content protection:** a session's footage doesn't show the widget or the
  app window. Turn it off in Settings and they appear.
- [ ] **Transparent widget:** the rounded corners have no square backdrop, the
  pill resizes cleanly, and dragging works.
- [ ] **Exit hold:** Quit from the tray while recording. The app waits, stops
  the session, writes `ended_epoch_ms`, then exits. The session plays back
  complete and no ffmpeg is left in Task Manager.
- [ ] **Display sleep:**
  1. Start a session and let the displays power off when idle (or force it).
  2. Come back after more than 20 minutes.
  3. Screens resume within a few seconds of input. The gap reads "Display was
     locked or asleep". You get one toast per outage, not one per retry.
  4. No screen records another monitor's picture.
- [ ] **Real export:**
  1. Export about 10 minutes of a real session in Devlog with Follow mouse,
     then again in Share.
  2. The file plays, Share is ≤25 MB, and the audio ducks while you talk.
  3. Delete it from `#/exports` and Undo.
  4. Quitting during an export leaves no `.part` file after the next start.
- [ ] **Delete on real footage:** delete a block from a real session, then check
  that disk space is freed about 12 s later and playback skips the hole.

## Environment

- FFmpeg comes from the picked folder, then PATH, then `C:\ffmpeg\bin`, then
  `C:\Program Files\ffmpeg\bin`, then the managed install under `%APPDATA%`.
- Recordings live where `settings.json` says, by default `%USERPROFILE%\Videos\ChirpTime`.
- Ports: Vite 7460, file server 9120. Don't touch either while Sal's dev app is running.
- No credentials or secrets.
