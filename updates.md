# ChirpTime — Updates

> Terse log of every task: what was asked → what was done. Newest first.
> Older entries: `updates-archive.md` (created once this passes ~300 lines).

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
