# ChirpTime — Updates

> Terse log of every task: what was asked → what was done. Newest first.
> Older entries: `updates-archive.md` (created once this passes ~300 lines).

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
