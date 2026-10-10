# ChirpTime — CliffNotes

> Living map of the project. Read this before any coding session.
> Last updated: 2026-10-10 (redesign sprint). Visual language → `ui.md` · why → `decisions.md` · verification → `verify.md`

## What this is

ChirpTime is a Windows-only flight recorder for a working day. One continuous
**session** records every screen, the webcam, the mic and system audio as 60 s
segments, all day, unsupervised. Afterwards you browse days on a time map, play
a session back in sync, **delete blocks of time** (instant, 10 s undo, then the
disk is freed) and **export** a range as one composed MP4.

Two windows:
- **Recorder** (`main`, 320×540, transparent, always on top; collapses to a
  264×52 pill), plus a tray icon and global hotkeys.
- **The app** (`viewer`, 1280×800, min 1024×700): Library, Player, Export,
  Marks, Exports, Settings, and first run.

The old interval "Clips" mode (screenshot + 5 s clip every N minutes) and its
calendar / gallery / media viewer are gone from the UI. Its Rust commands are
still registered with no caller (removal is on the Later list).

Tauri v2, React 19 + Tailwind v4 ("Paper", see `ui.md`), and a Rust backend
that shells out to FFmpeg for all capture, trimming, thumbnails and export.

## Quick reference

- **Dev app:** `bun run dev` (`tauri dev`, Vite on **http://localhost:7460**, strict).
  It rebuilds and restarts on every Rust save. When Sal is running it, don't
  edit Rust under it, and **never rebase or rewrite the tree while it runs**:
  the 2026-10-10 rebase killed it.
- **Browser harness (no Tauri):** `bun run vite:dev` and `bun scripts/fixture-server.ts`, then
  http://localhost:7460/?mode=viewer&mock=1#/library. See `verify.md`.
- **Entry:** `src/ui/main.tsx` loads `app/App.tsx` for `?mode=viewer` and
  `CompactApp.tsx` otherwise. With `?mock=1` in DEV, it installs `dev/mockIpc.ts` first.
- **Rust entry:** `src-tauri/src/main.rs` → `chirptime_lib::run()` in `lib.rs`.
- **Type-check:** `bunx tsc --noEmit` · `cargo check --manifest-path src-tauri/Cargo.toml`.
- **Tests:** `cargo test --lib`, then `--test edit_cuts`, `thumbs`, `settings_persistence`
  and `export`, plus `bun scripts/check-library.ts` and `bun scripts/check-log.ts`.
  **Never plain `cargo test`**: it includes `tests/live_session.rs`, which records.
- **Settings:** `%APPDATA%\com.chirptime.app\settings.json` (v2): output dir,
  session options, onboarded, `ffmpeg_dir`, hotkeys, content protection,
  discard-short-sessions. A missing or corrupt file falls back to defaults, field by field.
- **Recordings:** `%USERPROFILE%\Videos\ChirpTime` by default (`state::default_output_dir`).
- **Media:** the in-process Warp server on **127.0.0.1:9120** serves `/files/<rel>`
  (with Range support) and `/thumb/...`. The Tauri asset protocol is not used.
- **Hotkeys:** Ctrl Alt R starts or stops, Ctrl Alt M marks, Ctrl Alt P pauses (configurable).
- **Windows only:** DXGI `ddagrab`, dshow, WASAPI loopback, Win32 input and cursor polling.

## Stack

| Layer | Choice | Notes |
| --- | --- | --- |
| Runtime / pkg mgr | bun | `bun.lock` is authoritative |
| Desktop shell | Tauri v2 | plugins: shell, fs, notification, global-shortcut (`hotkeys.rs`) |
| Frontend | React 19 + TS strict | Vite 6 on 7460, an in-house hash router, no state library |
| Styling | Tailwind v4 `@theme` tokens | `src/ui/app.css`; fonts bundled via `@fontsource-variable` (offline) |
| Backend | Rust (tokio, warp, serde, chrono) | crate `chirptime`, lib `chirptime_lib` |
| Capture / edit / export | FFmpeg subprocesses | found, never bundled; fetched on first run if missing |

## Directory structure

```
chirptime/
├── index.html · vite.config.ts (7460 strict) · package.json
├── CLIFFNOTES.md · ui.md · decisions.md · updates.md · verify.md · plans/
├── scripts/
│   ├── make-fixture.ts        Synthetic sessions → fixtures/sessions/ (gitignored), 7 sessions
│   ├── fixture-server.ts      /files + /thumb on 9120 for the harness; refuses if 9120 is taken
│   ├── check-log.ts           Prints the day log of a fixture session and asserts its rules
│   └── check-library.ts       Pure checks: follow hysteresis, time map, paused/gap facts
├── src/ui/
│   ├── main.tsx               Window switch (?mode=viewer) + DEV mock install
│   ├── app.css                Paper tokens (@theme), base, .num, .kbd, toast + rec-dot keyframes
│   ├── types.ts               Wire types (mirror the Rust serde structs)
│   ├── CompactApp.tsx         Recorder widget shell: view switch, pill resize, toasts, events
│   ├── recorder/              Widget views: IdleView, RecordingView, OptionsView, PillView,
│   │                          DeskMap, fields (shared option controls), live (state + events),
│   │                          setup (next recording), api (controls/hotkeys commands), ui (icons)
│   ├── app/                   The app window
│   │   ├── App.tsx            Shell + router + FirstRun gate
│   │   ├── AppShell.tsx       212 px sidebar (counts, recorder card, storage meter) / 60 px rail
│   │   ├── router.ts          Hash routes + legacy ?session=&ts= link migration
│   │   ├── appData.tsx        Shared session list / status / exports count for every page
│   │   ├── LibraryPage.tsx    Time map (days × hours), inspector, session delete, short-session clean-up, list view
│   │   ├── libraryModel.ts    Pure: time-map layout, session facts, thumb instants
│   │   ├── MarksPage.tsx      Every mark, by day, with inline notes
│   │   ├── NoteField.tsx      Inline note editor (Marks page + Player log)
│   │   └── icons.tsx          Line icons, 24-unit grid
│   ├── player/                #/session/<id>
│   │   ├── PlayerPage.tsx     Engine, selection, delete/confirm/undo flow, keys
│   │   ├── Header.tsx · Stage.tsx (Focus/Desk on the mat) · LogCard.tsx · Transport.tsx
│   │   │                      (transport / selection bar / ConfirmRow) · Ribbon.tsx (day ribbon + zoom strip)
│   │   ├── Toast.tsx · Popover.tsx (Tracks)
│   │   ├── log.ts             Pure: the day log from the manifest
│   │   ├── timeline.ts        Pure: collapsed time axis, outages, human gap reasons
│   │   └── follow.ts · useFollow.ts   Follow-the-mouse with 1.5 s hysteresis
│   ├── session/               Playback core: engine.ts (SyncEngine), TrackSurface.tsx, cuts.ts, format.ts
│   ├── export/                #/export + #/exports: ExportPage, Preview (mat), RangeStrip, Controls,
│   │                          compose.ts (preview geometry), jobs.ts (jobs outside React), api.ts, types.ts
│   ├── settings/              SettingsPage, FirstRun, Ffmpeg, Hotkeys
│   ├── hooks/                 useApi.ts (invoke wrappers), mediaUrl.ts (/files + /thumb URLs)
│   └── dev/                   mockIpc.ts (DEV + ?mock=1 only) + extra/{ctx,library,widget,export}.ts
└── src-tauri/
    ├── tauri.conf.json        main window 320×540 transparent; devUrl :7460
    ├── capabilities/          windows ["main","viewer"]
    ├── src/
    │   ├── lib.rs             Startup (settings, edit recovery, file server, tray, hotkeys,
    │   │                      export sweep), window close-to-tray, exit hold, command list
    │   ├── recorder.rs        Session recorder: per-track supervisors, outages, idle trim, pause, cursor sampler
    │   ├── session.rs         Manifest types, atomic write, cuts/markers merge, rebuild, thumb lookup
    │   ├── edit.rs            delete_range / undo_cut / finalize, whole-session delete, recovery, notes
    │   ├── export.rs          Export composer backend, jobs, index, leftovers sweep
    │   ├── cursor.rs          Cursor track format (CTC1), reader, get_cursor_track
    │   ├── controls.rs        Shared recorder actions + ActionLock, events, exit shutdown, open_app
    │   ├── tray.rs            Tray icon + menu, 1 s watcher, one toast per outage
    │   ├── hotkeys.rs         Global shortcuts, per-key registration errors
    │   ├── fileserver.rs      /files (Range, traversal guard) + /thumb (cache, 3 ffmpeg slots)
    │   ├── commands.rs        Settings, environment, ffmpeg setup, sessions, markers, edit, open_viewer
    │   ├── settings.rs · state.rs · nativeui.rs · ffmpeg_install.rs · sysaudio.rs
    │   └── capture.rs         find_ffmpeg, silent_command, output dir; legacy interval capture (no UI)
    └── tests/                 edit_cuts, thumbs, settings_persistence, export, ffmpeg_install (ignored),
                               rebuild_check (ignored), live_session (ignored, RECORDS)
```

## File map (concept → path)

| Concept | Location |
| --- | --- |
| Which window renders what | `src/ui/main.tsx` |
| A route / deep link | `src/ui/app/router.ts`; `open_app(route)` in `controls.rs` |
| Sidebar, recorder card, storage meter | `src/ui/app/AppShell.tsx` |
| Time map, inspector, whole-session delete | `src/ui/app/LibraryPage.tsx` + `libraryModel.ts` |
| Player layout / keys / delete flow | `src/ui/player/PlayerPage.tsx` |
| Day log wording | `src/ui/player/log.ts` (+ `timeline.ts` `humanReason`) |
| Ribbon, zoom strip, snapping | `src/ui/player/Ribbon.tsx`, `timeline.ts` |
| Sync playback, drift correction | `src/ui/session/engine.ts` |
| Cut maths on the frontend | `src/ui/session/cuts.ts` |
| Delete / undo / finalize on disk | `src-tauri/src/edit.rs` |
| Thumbnails | `fileserver.rs` `/thumb` + `session.rs` (`ThumbLookup`) |
| Recorder widget views | `src/ui/recorder/*`, shell in `CompactApp.tsx` |
| Start/stop/pause/mark from any surface | `src-tauri/src/controls.rs` |
| Tray menu + outage toast | `src-tauri/src/tray.rs` |
| Hotkeys | `src-tauri/src/hotkeys.rs`, `src/ui/settings/Hotkeys.tsx` |
| Screen outage / restart logic | `src-tauri/src/recorder.rs` (`Supervision`, `outage_backoff_ms`, identity match) |
| Cursor samples | `src-tauri/src/cursor.rs` (+ sampler in `recorder.rs`) |
| Export filter graph / jobs | `src-tauri/src/export.rs` |
| Export UI | `src/ui/export/*` |
| Settings / first run | `src/ui/settings/*`, `src-tauri/src/settings.rs` |
| FFmpeg lookup / install | `capture::find_ffmpeg` + `session::find_ffprobe` (keep in step), `ffmpeg_install.rs` |
| Add a command | `commands.rs` (or the owning module) → `generate_handler!` in `lib.rs` → wrapper in `useApi.ts` or the module's `api.ts` → mock in `dev/` |
| New window label | `src-tauri/capabilities/default.json` |

## Routes / URLs

App window (`viewer`), hash routes, from `src/ui/app/router.ts`:

| Route | Page |
| --- | --- |
| `#/library` (default) | `app/LibraryPage.tsx` |
| `#/session/<id>?t=<epoch_ms>` | `player/PlayerPage.tsx`, playhead at `t` |
| `#/export/<id>?from=<ms>&to=<ms>` | `export/ExportPage.tsx` |
| `#/marks` | `app/MarksPage.tsx` |
| `#/exports` | `export/ExportsPage.tsx` |
| `#/settings` | `settings/SettingsPage.tsx` |

- First run (`settings/FirstRun.tsx`) covers the whole window while `settings.onboarded` is false.
- `open_viewer` still builds `/?mode=viewer&session=<id>&ts=<ms>`, and `migrateLegacyLink`
  folds that onto `#/session/<id>?t=`. Old `?date=` / `?tab=` links land on `#/library`.
- The recorder widget is `/` (window `main`).

File server (127.0.0.1:9120):

| URL | Serves |
| --- | --- |
| `/files/sessions/<id>/session.json` | Manifest |
| `/files/sessions/<id>/<track_dir>/<YYYYMMDD-HHMMSS>.mp4` | One segment (Range) |
| `/files/exports/<name>.mp4` / `.jpg` | An export and its first-frame thumbnail |
| `/thumb/<session>/<track>/<epoch_ms>.jpg?w=<64..960>` | One frame. 404 in a gap, a cut, an audio track or an unfinished segment. Cached on disk |

The frontend rounds `/thumb` instants down to 10 s so the cache hits.

## Commands

Registered in `lib.rs`. Grouped by owner:

- **Settings / environment** (`commands.rs`): `get_settings`, `set_session_options`,
  `set_onboarded`, `get_environment`, `get_output_dir`, `set_output_dir`,
  `pick_output_dir`, `open_folder`, `list_devices`, `list_display_outputs`,
  `open_viewer`.
- **FFmpeg:** `get_ffmpeg_setup`, `install_ffmpeg`, `pick_ffmpeg_dir`, `clear_ffmpeg_dir`.
- **Sessions:** `start_session`, `stop_session`, `get_session_status`,
  `list_sessions`, `get_session`, `add_marker`, `set_marker_label` (refused while
  live: "Stop recording to edit notes"), `rebuild_session`.
- **Editing** (`edit.rs` via commands): `delete_range`, `undo_cut`,
  `finalize_cuts`, `delete_session`, `undo_delete_session`.
- **Recorder controls** (`controls.rs`): `get_recorder_control`,
  `start_recording`, `stop_recording` (with short-session discard),
  `pause_session`, `resume_session`, `restart_screens`, `mark_now`,
  `set_live_marker_label`, `open_app`, `show_recorder_window`,
  `set_content_protection`, `set_discard_short_sessions`.
- **Hotkeys:** `get_hotkeys`, `set_hotkeys`.
- **Cursor:** `get_cursor_track(sessionId, fromMs, toMs)`.
- **Export** (`export.rs`): `estimate_export`, `start_export` (emits
  `export-progress`), `cancel_export`, `list_exports`, `delete_export`,
  `reveal_export`.
- **Legacy, no caller:** `get_dates`, `get_date_entries`,
  `get/save_monitor_config`, `delete_entry`, `delete_file`,
  `get/set_capture_config`, `start/stop_capture`, `get_capture_status`,
  `run_single_capture`, `get_ffmpeg_path`.

Events: `recorder-changed` (`RecorderControl`), `recorder-mark`, `recorder-discarded`
(receipt, for Undo), `export-progress`, plus the window event `chirptime:exports-changed` (frontend only).

## On-disk layout

```
<recordings>/                          default %USERPROFILE%\Videos\ChirpTime
├── sessions/
│   ├── <YYYY-MM-DD_HH-MM-SS>/
│   │   ├── session.json               manifest (atomic); cuts merged in from cuts.jsonl
│   │   ├── cuts.jsonl                 the cut sidecar: Pending / Final / Undone lines, append-only
│   │   ├── markers.jsonl              append-only; read_markers dedupes by epoch, last note wins
│   │   ├── <track_dir>/<YYYYMMDD-HHMMSS>.mp4   60 s video segments (600 s for mic / sysaudio)
│   │   ├── mic-16k/                   16 kHz mic sidecar (for future transcription)
│   │   ├── .trash/<cut_id>/<track_dir>/   segments parked by a Pending cut (+ plan.json, finalize.json)
│   │   ├── thumbs/<track>/<w>/<epoch_ms>.jpg   /thumb cache, purged inside every cut
│   │   └── cursor/<start_epoch_ms>.bin    CTC1 cursor runs, one per start / resume
│   └── .trash/<id>/                   a whole deleted session during its undo window
└── exports/
    ├── <YYYY-MM-DD HH.mm> <title>.mp4 + .jpg   (raw-tracks exports are a folder)
    ├── index.jsonl                    one record per export, rewritten atomically
    └── .work/<job>/                   per-job concat lists + temp files, removed when the job ends
```

`list_sessions` skips `.trash`, and everything outside `sessions/`. The manifest
rebuild never indexes `thumbs/`, `cursor/` or `.trash`.

## Architecture

```
Recorder widget ─┐                      ┌─ recorder.rs ── ffmpeg × tracks ──► sessions/<id>/
Tray, hotkeys ───┼─► controls.rs (ActionLock) ─┤
App window ──────┘   invoke() ─► commands.rs ─┼─ edit.rs (EDIT_LOCK) ── ffmpeg -c copy trims
                                              └─ export.rs ── ffmpeg filter_complex ──► exports/
<video>/<img> ──HTTP──► fileserver.rs :9120 (/files, /thumb)
```

All mutable state is in one `Arc<Mutex<AppState>>`. Media never crosses IPC.
The widget, the tray and the hotkeys all call the same `controls::*`, and every change is
announced as `recorder-changed`, so all three surfaces agree. The tray watcher polls
once a second, so it also catches a start made from the app's recorder card.

## Systems

### Session recorder — `recorder.rs`
- One supervised ffmpeg per track. Screens use `ddagrab`, with NVENC where it
  exists and QSV/x264 otherwise. The webcam and mic share **one** dshow process.
  System audio is WASAPI loopback piped in by `sysaudio.rs`.
- As each segment closes it is ffprobed and indexed into the manifest.
- **Idle trimming:** closed segments with no keyboard or mouse input are
  deleted, leaving gaps with the reason `"idle — no keyboard or mouse activity"`.
- **Pause** stops every track's ffmpeg. Resume restarts them into the same
  session, and each track that had recorded gets one gap with the reason
  `"paused"` (`PAUSED_GAP_REASON`).
- **Outages:** a display going dark (Kernel-Power: displays power off when idle) is
  classified as an outage. The screen waits it out with a 1 s → 30 s backoff,
  for as long as the session runs, once it has recorded anything. Retries come
  every 2 min while the user is away, and 3 s after they come back.
  `restart_screens` retries now. The tray sends one toast per outage.
- **Screen identity:** each screen keeps its session-start `ScreenIdentity`
  (device name + rect). Relaunches, resumes and restarts locate the monitor
  from that identity, never from a moving one.
- **Cursor sampler:** `GetCursorPos` at 10 Hz while recording (never while paused) → `cursor.rs`.

### Deleting time — `edit.rs`
1. **`delete_range`:** plans the cut, renames the segments wholly inside it into
   `.trash/<cut_id>/`, writes the manifest without them, appends `Pending` to
   `cuts.jsonl`, and purges the thumbs in the range. Instant.
2. **`undo_cut`:** moves the segments back, within `UNDO_WINDOW_MS` (10 s).
3. **`finalize_cuts`:** runs `FINALIZE_AFTER_MS` (12 s) later, or at startup
   recovery. It trims the edge segments with ffmpeg stream copy (never a
   re-encode), verifies the pieces with ffprobe, deletes the parked files and
   appends `Final`.
   - Video edges snap outward to keyframes (2 s GOP), and the cut keeps that
     sliver masked forever.
- Whole-session delete moves `sessions/<id>` to `sessions/.trash/<id>`, with the same undo.
- **Ordering rule:** the manifest is written before anything is deleted.
  `recover_sessions` finishes whatever a crash interrupted.

### Playback — `session/engine.ts`
- One `SyncEngine` per session owns a master clock in epoch ms. Each
  `TrackController` holds two media elements: one visible, one pre-rolling the
  next segment.
- Drift correction: over 150 ms → hard seek, 30–150 ms → ±3 % rate nudge.
- Cuts and gaps are skipped. `segmentAt` binary-searches each segment's own
  start and duration.
- The Player renders the clock imperatively from `subscribeFrame`, never
  through React state.
- In a gap or a cut, the stage holds the last frame before it, dimmed, with "No footage · <reason>".

### Export — `export.rs`
- One ffmpeg run per export (two for Share). Each track is **one** concat-demuxer
  input that lists only the segments the export needs, placed at their
  wall-clock offsets.
- The filter graph:
  1. `select`s the kept pieces: the range minus cuts, minus idle stretches when
     "Skip idle stretches" is on.
  2. Composes the layout (Devlog, Short 9:16, Side by side, Desk) with overlays
     enabled per piece. Follow mouse switches screens as hard cuts with
     1.5 s hysteresis.
  3. Mixes mic and system audio, with sidechain ducking.
  4. Applies the speed.
- Share targets ≤25 MB with a two-pass encode at 1080, 720 or 480. It is refused
  below 480p. Raw tracks stream-copies each track into a folder.
- Safety:
  - Outputs are reserved with `create_new` (`.mp4.part`).
  - Start is refused below the free-space floor.
  - `record_path` only accepts `exports/<one plain name>`.
  - `kill_all_jobs()` runs on exit, and `sweep_leftovers()` runs at setup.

## Key types

| Type | Where | Purpose |
| --- | --- | --- |
| `SessionManifest` / `SessionTrack` / `SessionSegment` / `TrackGap` | `session.rs` / `types.ts` | The session contract; `cuts` merged in from the sidecar |
| `Cut` / `CutReceipt` / `SessionTrashReceipt` | `edit.rs` / `types.ts` | `pending` → `final` (or `undone`); receipts carry `undo_until_epoch_ms` |
| `SessionMarker` | `session.rs` / `types.ts` | `epoch_ms` + note (`label`) |
| `SessionSummary` | `recorder.rs` / `types.ts` | One Library entry. Must match the Rust struct exactly |
| `SessionStatus` / `TrackSummary` | `commands.rs` / `types.ts` | Live health, incl. `last_error_human`, `waiting_since_epoch_ms`, `next_attempt_epoch_ms` |
| `RecorderControl` | `controls.rs` | Idle / recording / paused + flags, the `recorder-changed` payload |
| `CursorSample` | `cursor.rs` / `types.ts` | `{ t, x, y }` in virtual-desktop pixels (same space as `track.monitor`) |
| `SessionOptions` | `recorder.rs` / `types.ts` | What the next recording takes. Read once at start |
| `Settings` | `settings.rs` / `types.ts` | v2 persisted settings |
| Export request / estimate / progress / index | `export.rs` / `src/ui/export/types.ts` | Export-only mirrors |

## Gotchas & hard rules

- **Don't touch Sal's running dev app.** `tauri dev` restarts on every Rust save. Never
  rebase, reset or switch branches under it. Never kill processes by name pattern.
- **Outages are not failures.** Once a screen has recorded, a dark display retries
  forever. Only a screen that never recorded spends the 20-restart budget, so an
  encoder that can never open still fails instead of "waiting" forever.
- **Screen identity is immutable.** Match against the session-start identity. An output that
  exactly matches another screen's identity is never adopted, and the home index wins ties.
  Updating the identity as you go is how one monitor got recorded twice.
- **One edit lock.** Every manifest read-modify-write in `edit.rs` (cut, undo,
  finalize, note, session delete) runs under the process-wide `EDIT_LOCK`.
- **A session is editable only once its stop has finished.** A self-stopped session runs its stop first.
- **Finalize waits on exports.** `finalize_due_cuts` waits while `export::is_exporting(session)`,
  re-checking every 3 s and once more under the edit lock, so a trim never pulls a
  segment out from under an export. Export refuses while a cut is pending
  ("Wait a few seconds for the delete to finish").
- **One action lock.** Start, stop, pause, resume and Restart screens run one at a time under
  `controls::ActionLock`. A hotkey or tray toggle that arrives while another
  action is busy is **dropped, not queued**. `start` runs `Recorder::start`
  in `spawn_blocking`, and does nothing when a session is already open.
- **Exit hold.** `RunEvent::ExitRequested` is prevented while `controls::busy()`.
  `shutdown()` stops the session (bounded wait) before the app exits. Closing
  the recorder while busy hides it to the tray. Tray Quit goes through the
  same path. No export outlives the app.
- **Deleted footage must not survive as JPEGs.** Every cut purges `thumbs/` in its range.
- **`set_marker_label` is refused on a live session.** The recorder owns that manifest; use `set_live_marker_label`.
- **Segments are ragged and overlap.** Never compute a position from
  `index * segment_seconds`. Seam holes under 250 ms are label jitter, not loss.
  Webcam and mic lengths differ by about 200 ms by design.
- **The webcam and mic are ONE ffmpeg process** (one dshow input) and two tracks. Never split them.
- **A track can be backed by more than one output** (`TrackSpec.outputs`). Never assume one process per track.
- **Idle and paused gaps are intentional.** Health checks must not count them as failures.
- **`ended_epoch_ms === null` is not "recording".** Cross-check `get_session_status`; otherwise the session shows as Interrupted.
- **Every subprocess goes through `silent_command()`,** or a console window flashes into the capture.
- **`find_ffmpeg` and `find_ffprobe` look in the same five places, in the same
  order** (picked dir → PATH → `C:\ffmpeg\bin` → `C:\Program Files\ffmpeg\bin`
  → the app-managed install). `ffmpeg_install` never downloads on its own, and
  its pin is three constants that move together.
- **Port 9120 is hardcoded** in `lib.rs`, `hooks/mediaUrl.ts` and `scripts/fixture-server.ts`. Vite is 7460 in
  `vite.config.ts` and `tauri.conf.json` `devUrl`.
- **The widget window is transparent.** The rounded Paper body is drawn by
  CSS. `CompactApp.tsx` calls `getCurrentWindow()` at import, so the app
  window must never import it.
- **Content protection is on by default.** Both windows are excluded from capture, applied as each window is created.
- **Version lives in three files** (`tauri.conf.json`, `Cargo.toml`, `package.json`).
- **Releases are unsigned.** `chirptime.exe` is the binary name.
- **The repo is public.** No footage, and no personal paths in commits. Fixtures are synthetic.

## Status

- **Done (2026-10-10 sprint):**
  - Paper redesign: app shell, Library, Player with delete and undo, Marks,
    Export composer, Exports, Settings, first run.
  - The recorder widget, tray, hotkeys, pause, content protection, the cursor
    track and thumbnails.
  - Outage handling, and the review fixes on all of it.
- **Not verified live** (only in the harness and the tests): see "Live checks (Sal)" in `verify.md`.
- **Later:**
  - Away detection (presence lines, "delete all away stretches").
  - Transcription and OCR.
  - A storage view with retention rules.
  - A dark mode that follows Windows.
  - Removing the legacy interval-capture commands.
  - `stop_session` still bypasses `controls` (no UI caller).
  - A job object for export ffmpeg.
  - A stale comment at `tray.rs:5-6`.
