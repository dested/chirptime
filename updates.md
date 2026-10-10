# ChirpTime — Updates

> Terse log of every task: what was asked → what was done. Newest first.
> Older entries: `updates-archive.md` (created once this passes ~300 lines).

## 2026-10-10 — Redesign sprint: Paper app, delete time, export, recorder rebuilt
Sal asked for a full UI/UX rethink, the ability to delete blocks of time, and
export. He took every recommendation, with overrides: the player looks like the
rest of the app, delete frees disk now with an undo, and the UI is light only.
Commits `35509d0..f4460a4`. Ledger: `plans/2026-10-10-ux-sprint.md`.

- **Backend:**
  - Idle trimming committed.
  - `edit.rs`: cuts sidecar, instant delete with a 10 s undo, then stream-copy
    finalize, whole-session delete, crash recovery.
  - `/thumb` with a cache purged inside cuts.
  - Screens wait out display outages, and restarts keep the session-start identity.
  - Pause leaves `"paused"` gaps.
  - New: the `cursor.rs` CTC1 track; `controls.rs` (one action lock, exit hold,
    short-session discard with Undo); the tray, hotkeys and content protection.
  - `export.rs`: one filter_complex over per-track concat lists; Share two-pass ≤25 MB;
    raw tracks; index + `.work`; leftovers swept at startup, jobs killed on exit.
  - Finalize waits while an export reads the session.
- **Frontend:**
  - Paper tokens and bundled fonts; a hash router.
  - The app shell, Library time map, Player (stage, log, ribbon, zoom strip,
    select / confirm / undo, follow-mouse), Marks with inline notes, Export
    composer, Exports, Settings and first run.
  - The recorder widget, rebuilt (idle, recording, alert, options, pill).
  - Clips, the old viewer and the dead CLI deleted. Vite moved to 7460.
- **Harness:** `make-fixture.ts` (7 sessions), `fixture-server.ts`, `?mock=1`
  IPC with per-owner extras, `check-log.ts`, `check-library.ts`.
- **Reviews:** two read-only reviewers, on the recorder and on export. Every finding was fixed.
- **Gate (98d1c6f, clean archive):** tsc, vite build, lib 148 ×6, edit_cuts 12,
  thumbs 6, settings 1, export 17 all exit 0. The review fixes then added more tests (final gate at f4460a4: lib 152, edit_cuts 12, thumbs 6, settings 1, export 24, all exit 0).
- **Still open:** the live checks Sal has to do himself (`verify.md`).

## 2026-08-01 — Inactivity trimming for the session recorder
User wanted the continuous recorder to stop keeping footage of idle time —
"delete that part in real time" once there's been no keyboard/mouse input for a
while. Confirmed scope with the user: **session recorder only** (not interval
clips), **keyboard/mouse only** (`GetLastInputInfo`, no "audio playing"
exception), **drop everything idle** (video and audio alike).
Implementation, all in `recorder.rs`: a background `idle_poll_loop` samples the
system-wide last-input time (`poll_last_input_ms`, Win32 `GetLastInputInfo` +
`GetTickCount`) into `Inner.last_input_ms` every 2s. When a closed segment is
about to be indexed (`index_output_segment`), if its whole span had no input and
the idle stretch already exceeded the threshold (`should_drop_idle`), the file is
`remove_file`d and never added to the manifest; ffmpeg keeps running untouched.
Consecutive dropped segments collapse into one `IdleTail`, flushed by the next
kept segment as a single "idle — no keyboard or mouse activity" gap; that kept
segment chains across the hole via a new `chain_override` arg to `index_segment`
so the drift path doesn't also invent a "dropped time" gap. Delete failure falls
back to keeping the segment (never orphans a file). Config: new
`SessionOptions.drop_idle_after_seconds` (default 60, 0 = off), Windows-only
(forced 0 elsewhere — no poller means no false mass-deletion). Frontend: field
added to `types.ts` + `DEFAULT_OPTIONS`, and an "Inactivity" dropdown (Keep
everything / 1 / 2 / 5 min) in `SessionSettings`. Tests: `should_drop_idle`
cases + payload/default coverage. `cargo test --lib` (72) + `tsc` green.

## 2026-07-28 — Diagnosed "system audio recorded nothing" (session 09-29-11)
User reported sysaudio empty for a 1h session while webcam/mic were fine.
Forensics: the track is healthy (8 segments, full wall-clock duration, 0
restarts) but contains **only short app-notification blips** (0.31s/0.48s/a
recurring 1.204s ping, plus one 6.8s clip at −4.3dB); everything else is the
writer's deliberate silence padding. A new `#[ignore]`d live diagnostic
(`sysaudio::tests::live_loopback_probe`, run with
`cargo test --lib live_loopback_probe -- --ignored --nocapture`) proved WASAPI
loopback on the current default endpoint (AirPods Pro A2DP) captures real audio
correctly. Conclusion: **not a capture bug** — the loopback taps only the
default *console* render endpoint, and this machine plays audio to several
endpoints (default comms = Dell monitor speakers, 8 active render devices,
per-app routing overrides for Chrome/VLC/Steam games). Whatever the user
listened to played on a non-default endpoint, so the recorder never saw it.
Follow-ups proposed, not built: show the captured device name + a live level
meter in SessionPane track health; optionally capture the comms endpoint as a
second loopback track.

## 2026-07-27 — MIT license + first-run FFmpeg install
Four settled calls from the user, on top of the release pipeline below.
**MIT `LICENSE`** added (Sal Aiello, 2026) and referenced from the README. The
reasoning that unlocks it: ChirpTime *invokes* FFmpeg as a subprocess and
neither links it nor ships its binaries, so FFmpeg's GPL never reaches
ChirpTime's own distribution — which is exactly why the binary is fetched rather
than bundled. **Version stays 1.0.0** (the 0.1.0 downgrade was reverted in all
three files). **No code signing** — documented honestly in the README instead:
what the SmartScreen box says and how to get past it.
**FFmpeg is downloaded on first run**, never bundled. New
`src-tauri/src/ffmpeg_install.rs`: pinned HTTPS URL + compiled-in SHA-256,
hashed *while* streaming to disk, verified before anything is extracted or
executed, extracts only `ffmpeg.exe` + `ffprobe.exe` (skipping ffplay's ~100 MB),
runs each once as a smoke test, then renames staging into place — so a failed
install can never leave something that looks like a working FFmpeg. Per-user
under `%APPDATA%\com.chirptime.app\ffmpeg\bin`, no elevation. Progress events
(`ffmpeg-install-progress`) drive a real progress bar.
Pin: gyan.dev **8.1.2 essentials**, 104.6 MB, verified to carry every encoder,
filter and input the app uses. Nothing downloads without a button press, and the
size is stated first.
`find_ffmpeg` / `find_ffprobe` extended in lockstep to five candidates:
hand-picked folder → PATH → `C:\ffmpeg\bin` → `C:\Program Files\ffmpeg\bin` →
managed install. New `settings.ffmpeg_dir` + a folder picker ("I Already Have
It") so a network that blocks the download is not a dead end. Shared
`FfmpegSetupCard` renders in both the first-run panel and a persistent banner —
one flow, not two.
Verified: real install run twice end to end (fresh, then over an existing
install to exercise the replace path) — 194.2 MB on disk, both binaries report
8.1.2, no debris left; hash-mismatch path proven to fail and delete; `cargo
check`, `cargo test --lib` (67 pass), `bunx tsc --noEmit`; full `bun run build`.
Not verified: the install UI has not been driven from a running widget (this
machine has FFmpeg on PATH, so the missing branch never renders here).
Touched: LICENSE (new), src-tauri/src/ffmpeg_install.rs (new),
src-tauri/tests/ffmpeg_install.rs (new), src-tauri/src/{capture,session,settings,
state,commands,lib}.rs, src-tauri/Cargo.toml (ureq/native-tls/sha2/zip),
src/ui/{CompactApp.tsx,types.ts,hooks/useApi.ts}, README.md,
.github/workflows/release.yml, tauri.conf.json + package.json (version back to 1.0.0).

## 2026-07-27 — tagged Windows release pipeline
"Set up a tagged Windows release pipeline." Enabled the Tauri bundler
(`bundle.active`, targets `msi` + `nsis`, publisher/description/category,
`icons/icon.ico`, NSIS `installMode: currentUser` so an unsigned installer needs
no UAC). Proposed dropping the version 1.0.0 → 0.1.0; **the user kept 1.0.0**,
so all three files (`tauri.conf.json`, `src-tauri/Cargo.toml`, `package.json`)
stay at 1.0.0 and the README carries the "this is early" note instead.
New `.github/workflows/release.yml`, triggered on `v*` tags, `windows-latest`
only (Windows-only by decision — no cross-platform matrix, those jobs could only
fail). Bun + Rust + `Swatinem/rust-cache` → `bun run build` → globs the bundles,
zips the bare exe as a portable, and `gh release create` with `GITHUB_TOKEN`
(no third-party release action, no new secret). Deliberately a separate file
from the Drydock-owned `.github/workflows/drydock.yml`.
FFmpeg gap: a non-dismissible red notice above the mode switch whenever no
ffmpeg is found, and the same content in the first-run panel. Probed once at
startup, never on a poll. **Not bundled** — the app calls `libx264`
unconditionally, so any usable build is GPL. Superseded the same day by the
download-on-first-run entry above.
Real README (was `# screener`): what it is, download, FFmpeg requirement and the
three probe paths, 5–8 GB/hour + nothing auto-deleted, build-from-source,
"don't run bare `cargo test`".
Verified: local `bun run build` produced `ChirpTime_0.1.0_x64_en-US.msi` (5.01 MB)
and `ChirpTime_0.1.0_x64-setup.exe` (3.34 MB); the workflow's collect step
dry-run against that output produced all three artifacts; release-notes
here-string executed; workflow YAML parses; `cargo check`, `cargo test --lib`
(64 pass), `bunx tsc --noEmit`.
Not verified: the banner's rendered appearance (this machine has FFmpeg, so the
`false` branch never renders here), and the workflow itself has never run — no
GitHub remote exists yet. Installers were built, not installed. Unsigned:
SmartScreen will warn.
Touched: src-tauri/tauri.conf.json, src-tauri/Cargo.toml, package.json,
src/ui/CompactApp.tsx, README.md, .gitignore,
.github/workflows/release.yml (new).

## 2026-07-27 — config persistence + first-run onboarding
"The default output dir just moved — make sure the user knows that!!" Added
`settings.rs`: `%APPDATA%\com.chirptime.app\settings.json` holds the capture
config, the session options (opaque JSON — `SessionOptions` is deserialize-only
and recorder.rs was off-limits) and an `onboarded` flag. Loaded in `run()` before
the state and the file server exist; loading degrades field by field so a corrupt
or older file can never stop the app or silently reset the output dir. Every
mutating command persists.
New inline first-run panel in the widget (no modal — 320px): resolved absolute
path, free space + hours of recording it buys, ~5–8 GB/hour, nothing is ever
auto-deleted, FFmpeg/NVENC presence on this machine, and a native
`IFileOpenDialog` folder picker owned by the widget HWND (unowned it opens behind
the always-on-top window). Legacy `output/` beside the exe/repo is detected and
reported with its day/session counts plus a "Use This Folder" button — nothing is
moved, copied or deleted. Permanent "where are my recordings" affordances: a
titlebar folder button (both tabs) and the same card in session settings, which
replaced the free-text path field.
Verified: `cargo check`, `cargo test --lib` (64 pass, +12 new), new
`cargo test --test settings_persistence` (save → reload → corrupt → half-written),
`bunx tsc --noEmit`, and the widget rendered at a real 320px in a browser via a
temporary mocked-IPC harness (since deleted) — four states screenshotted.
Not exercised: the actual Win32 picker and Explorer launch need a real app run.
Touched: src-tauri/src/{settings,nativeui}.rs (new), commands.rs, lib.rs, state.rs,
Cargo.toml (windows Win32_UI_Shell features), src/ui/{CompactApp.tsx,types.ts,hooks/useApi.ts},
src-tauri/tests/settings_persistence.rs (new).

## 2026-07-27 — audio tracks get 10-minute segments; boundary choppiness accepted
User accepted the segment-boundary hiccup rather than eliminating it ("it doesnt
have to be nano second perfect"). Cheap mitigation only: `mic` and `sysaudio` now
segment at `AUDIO_SEGMENT_SECONDS` (600s) instead of the session length, cutting
boundaries ~10x. Segment length moved to per-*output* (`OutputSpec`), so the
shared webcam+mic process cuts video at 60s and audio at 600s in one command;
watchdog is per-output too. Video/screens unchanged, codec unchanged.
Verified live: all tracks produce segments, webcam/mic start-label delta **+0ms**
(lip-sync intact), mic 3 segments -> 1, long audio segments finalize playable.
Caveat recorded in decisions.md: a crash now loses up to 10min of audio, not 60s.
Touched: src-tauri/src/recorder.rs, src-tauri/tests/live_session.rs

## 2026-07-27 — fixed webcam/mic lip-sync in the session recorder
"My mouth doesn't move with my audio." Webcam and mic were two independent ffmpeg
processes, so each device's open latency was baked into its track start —
measured +1239ms / +586ms of audio ahead of picture. Now captured by one process
from one dshow input (`video=X:audio=Y`), still written as two separate tracks
and files via `-map`. Measured after, two real runs: **+22ms** and **−11ms**,
both tracks anchored to the identical instant. (The ~200ms media-length gap is
keyframe-vs-clock-tick segment quantization, not desync — verified per segment
with ffprobe.) `TrackSpec` grew an `outputs`
list, segment indexing became per-output, and the watchdog is now per-output so a
dead camera inside the pair is still noticed. If either half produces nothing for
two runs, `split_pair` gives the mic its own supervisor (verified live with a
nonexistent camera: mic recorded 3 segments while the camera failed alone).
Also investigated a suspected ~110ms dropout at every segment seam (mic, sysaudio,
and h264_qsv screen-3): **no media is lost on any track.** Seam-gap size is
uncorrelated with loss — screen-3 has the largest gaps and exactly 0.0ms loss;
two consecutive mic segments hold 469+468 AAC packets = 19989ms per 20000ms of
wall clock. The gaps are label jitter from the drift filter's biased input (file
appearance lags content by the encoder/muxer buffer). All under the player's
250ms seam tolerance. No code change; gotcha added with the correct test method.
Raised `PAIR_SPLIT_AFTER` 2→4 so a briefly-held camera doesn't cost lip-sync.
Touched: src-tauri/src/recorder.rs, src-tauri/tests/live_session.rs
Added tests: `lipsync`, `pair_split` (both `#[ignore]`d — they record), plus 6 unit tests.

## 2026-07-27 — continuous recording wired into the widget and the viewer
The recorder and the player both existed; nothing invoked them. Added typed
wrappers for every session command (`start/stop_session`, `get_session_status`,
`add_marker`, `rebuild_session`, `list_display_outputs`, `set_output_dir`) and the
types they return, mirroring the Rust structs field for field.

Widget: a **Clips | Session** switch (the interval capture is untouched and still
runs independently — the switch shows a live dot for whichever mode is recording).
The session pane has start/stop, elapsed ticking locally off `started_epoch_ms`,
per-track health rows (status dot, kind badge, segments, bytes, restarts, gaps,
error), segment/bytes/disk-free stats, an inline marker field, and an inline
settings panel (monitors, webcam, mic, system audio, fps, segment length, output
folder). Device problems are surfaced verbatim from the backend: a webcam held by
another app and the Bluetooth-mic refusal both render as their own notice.

Viewer: `?mode=viewer&session=<id>&ts=<epoch ms>` now deep-links to an instant;
a manifest that was rebuilt from disk offers **Save Index** to write it back.
Fixed: a force-killed session has no `ended_epoch_ms`, so the browser labelled two
real sessions **Recording** forever — it now cross-checks the live recorder and
shows **Interrupted** instead.

Rust: `SessionStatus` gained `disk_free_bytes` / `min_free_bytes`;
`list_display_outputs` now returns the recorder's own `screen_index` and a real
`capturable` flag (it previously reported every output as capturable);
`open_viewer` takes `session` and `tab`.

Verified without recording anything: `bunx tsc --noEmit`, `cargo check`,
`cargo test --lib` (46 pass, 2 new — they pin the options payload and defaults
against the widget), `vite build`, and a server-render pass over all 6 real
sessions in `output/sessions/` — every indexed segment resolves to a file on
disk, the player tree renders for each, deep-link seeks clamp, and the widget's
idle / recording / disk-full states render with the real error strings.
**Not verified: anything requiring a live recording or a visible app window.**
Touched: src/ui/CompactApp.tsx, ViewerApp.tsx, types.ts, hooks/useApi.ts,
session/{SessionPlayer,SessionBrowser}.tsx, src-tauri/src/{commands,recorder}.rs,
CLIFFNOTES.md, ui.md, decisions.md

## 2026-07-27 — session player verified against real recordings
Ran the player against the 6 real sessions in `output/sessions/`. Three bugs the
synthetic fixture could not expose, all fixed:
- `segmentAt()` blanked to "No footage" inside the 0–50ms holes real segments
  leave at most seams (1.04s of spurious blanking across 30 tracks); now tolerates
  seams ≤250ms and prefers the later segment where two overlap (41 real seams do).
- `SessionSummary` invented `screen_count` / `recording`, which `list_sessions`
  does not send — the browser rendered `undefined + NaN`. Now mirrors the Rust
  struct including `has_manifest`.
- A manifest referencing an unreadable segment (force-kill leaves one with no moov
  atom) showed a silent black tile; now "Segment unreadable", other tracks unaffected.
Measured on real footage: seek exact at all probes, spread median 9–27ms /
p99 27ms / max 99ms across boundary crossings, zero spurious blanks.
Touched: src/ui/session/engine.ts, TrackSurface.tsx, SessionBrowser.tsx, types.ts,
hooks/useApi.ts, cliffnotes.md, decisions.md

## 2026-07-27 — session player opens muted
The player defaulted its audio source to the mic track, so opening a session
(including by deep link) played audio out of the speakers unprompted. Now starts
on **Muted**. Dev fixtures regenerated silent as well — the fixture generator's
audio segments are `anullsrc`, not a tone.
Touched: src/ui/session/SessionPlayer.tsx, decisions.md, ui.md

## 2026-07-27 — multi-track synchronised session player (frontend)
Built the session browser and the multi-track player against the manifest schema
in `plans/2026-07-27-continuous-recording.md` §3, with the backend not yet
existing. Sync engine per §4 layer 3: master epoch-ms clock, 250ms resync,
>150ms hard seek / 30–150ms ±3% rate nudge. Two `<video>` per track with an 8s
pre-roll for gapless boundary crossing; segment-aware seeking; grid / focus /
overlay layouts; session-wide scrubber with per-track segments, gaps and markers.
Verified against generated ffmpeg fixtures (3 screens incl. 5120×2160 AV1 +
webcam + mic, 26 segments, deliberate 237/684/1420ms track offsets, a 47.5s gap,
burned-in absolute timecodes): 1x inter-track spread median 3.7ms / max 53ms
across four boundary crossings, boundaries cross as swaps with one dropped frame,
seeking exact at 10 probe points. 4x saturates the decoders in grid layout.
Touched: src/ui/session/* (new), types.ts, hooks/useApi.ts, ViewerApp.tsx,
main.tsx, cliffnotes.md, ui.md, decisions.md

## 2026-07-27 — set up the cliffnotes kit
Rewrote the stale map from the code (Tailwind v4 migration, monitor
auto-detection, audio, the calendar→gallery→viewer flow) and added the rest of
the kit: visual language, retroactive decision log, verification recipes.
`features/` not yet reconciled.
Touched: CLIFFNOTES.md, ui.md, decisions.md, verify.md, updates.md
