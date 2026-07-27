# ChirpTime — Verify

> How to prove the app works. Scale to the change: [cheap] always,
> flow recipes when touched, [heavy] only after asking.

## Commands

| What | Command | Cost |
| --- | --- | --- |
| Frontend type-check | `bunx tsc --noEmit` | [cheap] |
| Rust check | `cargo check --manifest-path src-tauri/Cargo.toml` | [medium] — first run compiles the whole dep tree |
| Rust unit tests | `cargo test --lib --manifest-path src-tauri/Cargo.toml` | [medium] — 46 pure tests, no hardware, no capture |
| Frontend-only build | `bun run vite:build` | [cheap] |
| Dev app | `bun run dev` → widget window + Vite on http://localhost:5173 | [medium] — spawns a Rust build and a real window |
| Release bundle | `bun run build` | [heavy — ask first] — full optimized Rust build + installer |
| FFmpeg present? | `ffmpeg -version` | [cheap] |
| Media server up? | `curl -I http://localhost:9120/files/` (needs the app running) | [cheap] |

No frontend test runner is configured. The Rust side has unit tests (`--lib`,
pure: device selection, encoder choice, ffmpeg arg shapes, drift maths, the
options contract with the widget) and integration tests under `src-tauri/tests/`.

**`src-tauri/tests/live_session.rs` starts a real recording** — screen, webcam,
microphone. Its tests are `#[ignore]`d, so a bare `cargo test` will not run them;
they only start when passed `-- --ignored`. Never do that while the user is at
the machine: opening the mic can flip a Bluetooth headset into hands-free mode,
and the capture contains whatever is on screen. `cargo test --lib` is the safe
subset.

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

### Multi-track session playback stays in sync [medium]
Touchpoints: `src/ui/session/*`, `types.ts`, `hooks/useApi.ts`

Needs a recorded session under `output/sessions/<id>/` with a `session.json`.
The frontend runs standalone: `bun run vite:dev`, then
`http://localhost:5173/?mode=viewer&session=<id>` (media still comes from :9120,
so the app — or a stand-in file server rooted at the output dir — must be up).

1. `?mode=viewer&tab=sessions` lists the session with duration, tracks, segments, size.
2. Open it → **Grid** shows every video track; press Space.
3. Turn on **Sync**. Every track's drift stays green (≤30ms) and the transport's
   **Spread** readout stays under ~50ms. Red drift or a climbing `cold` count
   means the pre-roll is not landing.
4. Let it run through a 60s boundary. `swap` increments, `cold` does not, and no
   track visibly stutters.
5. Scrub the timeline anywhere → every track lands on the same instant; tracks
   with a gap there show **No footage** and rejoin on time when the gap ends.
6. **Focus** puts one monitor full-bleed with the webcam as PiP; **Overlay**
   arranges the monitors in their real desktop positions. Both stay in sync.

1x and 2x are clean in every layout. 4x in Grid saturates three 4K/5K decoders —
that is a hardware limit, not a regression; check 4x in Focus instead.

To verify sync *objectively* rather than by eye, the session must carry a
burned-in timecode. Recordings do not; generate a fixture with ffmpeg
(`drawtext=text='%{pts\:hms\:<segment start offset>}'` per segment) and confirm
every tile shows the same number.

### Session UI against the sessions already on disk [cheap — no recording]
Touchpoints: `src/ui/session/*`, `CompactApp.tsx`, `types.ts`, `hooks/useApi.ts`

Everything below runs headless and starts nothing. Useful when changing the
manifest contract, the browser, the player, or the widget's session pane.

1. `bunx tsc --noEmit` and `bun run vite:build` — the widget and viewer both
   compile and bundle.
2. Server-render the session UI against the real manifests. In a scratch `.tsx`
   at the repo root, `JSON.parse` each `output/sessions/*/session.json` and
   `renderToString(<SessionPlayer manifest={m} onBack={()=>{}} />)`. Assert: no
   throw; every `segment.file` the manifest indexes exists on disk at the path
   `segmentUrl()` builds; the output contains neither `undefined` nor `NaN`; and
   `new SyncEngine(m).seekTo(<absurd value>)` clamps to the session bounds.
   Delete the scratch file afterwards.
3. The widget's session pane renders the same way, but `CompactApp.tsx` calls
   `getCurrentWindow()` at module scope — stub `@tauri-apps/api/window` and
   `@tauri-apps/api/core` with a `Bun.plugin` `build.module(...)` before
   importing it, then render the exported `SessionPane` with a fabricated
   `SessionStatus`. Effects do not run under SSR, so anything the pane needs at
   first paint must be derived during render, not set in a `useEffect`.

Real data worth knowing about: 6 sessions, 5 tracks each; three of them have one
unfinalized segment per track on disk that the manifest correctly omits; two have
`ended_epoch_ms === null` because they were force-killed, which must render as
**Interrupted**, not **Recording**.

### Continuous session recording end to end [heavy — ask first, starts real capture]
Touchpoints: `CompactApp.tsx` session pane, `commands.rs`, `recorder.rs`, `sysaudio.rs`

This grabs every monitor, the webcam and the microphone, and it beeps nothing —
but it does hold the camera and can switch a Bluetooth headset into hands-free
mode. Only run it with the user's knowledge, and stop it when done.

1. Widget → **Session** tab → **Settings**: monitors list the four displays with
   the iGPU one marked capturable only if `h264_qsv` exists; the mic picker's
   Bluetooth entries are labelled as such.
2. **Start Session** → the chip turns red, the elapsed clock ticks, and within
   `segment_seconds` the Tracks list shows one sky pulsing dot per track with
   rising segment counts and bytes. `output/sessions/<id>/session.json` appears.
3. A device that is unavailable (unplug the webcam, or hold it in another app)
   must appear as a red **is not recording** notice naming the device, while the
   other tracks keep counting up.
4. Type into the marker field and press **Flag** → a line confirms the wall-clock
   time and `markers.jsonl` gains a row.
5. **Stop Session** → the pane goes Idle, the footer offers **Open Last Session**,
   and that link opens the viewer straight into the player for that id.

### Delete a capture [medium — destructive, deletes real files]
Touchpoints: `commands.rs::delete_entry`, `ViewerApp.tsx`, `CompactApp.tsx`

1. In the media viewer press `Delete` (or the trash button) → `confirm()` names the time.
2. Accept → the entry leaves the filmstrip, the neighbour is selected, and all four
   files for that timestamp are gone from `output/<date>/`.

Use a throwaway capture you just took, never an archive day.
