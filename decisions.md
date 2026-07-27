# ChirpTime — Decisions

> Why things are the way they are. A recorded decision is settled unless the
> user reopens it. Newest first.
>
> Entries below dated 2026-03-xx were reconstructed from the code on 2026-07-27
> when this file was created; the reasoning is inferred from the implementation.

## 2026-07-27 — The webcam and mic share ONE ffmpeg process, via ONE dshow input
**Why:** lip-sync. As two independent processes each device's own open latency
(a BRIO takes ~1.3s to hand over its first MJPEG frame, a mic ~0.15s) is baked
into its track's start time, and nothing can recover it afterwards — measured
**+1239ms** and **+586ms** of audio-ahead-of-picture on sessions
`2026-07-27_14-03-20` / `_14-03-54`. One DirectShow graph stamps both streams
against one clock, so sync holds by construction. Measured after, over two real
runs: both tracks anchored to the **same** instant (+1879/+1879, +1843/+1843),
end-to-end divergence **+22ms** and **−11ms**, and per-segment A/V difference of
−6.3ms / +17.0ms on the segments not clipped by run start or stop.
**Rejected:** two `-f dshow` inputs in one process — that is two DirectShow
graphs with two independent start instants, which reproduces the offset exactly;
estimating and subtracting per-device latency (unstable, device-specific, and
wrong the first time a driver warms up differently); muxing them into one file
(transcription needs an isolated mic track and the player renders them
independently — they stay two tracks and two files).
**Consequence:** `TrackSource::WebcamMic`, and `TrackSpec.outputs` is a list so
one supervised process can feed two manifest tracks. The combined input fails
outright when another app holds the camera, so after `PAIR_SPLIT_AFTER` runs in
which either half produces nothing, `split_pair` hands the mic to its own
supervisor and the webcam continues solo — recording beats lip-sync.

## 2026-07-27 — Continuous recording is a second mode in the widget, not a replacement
**Why:** the interval chirp/clip capture and the all-day session recorder answer
different questions ("what was I doing at 3pm" vs "replay the last four hours"),
and the spec is explicit that `run_capture_cycle` keeps working unchanged. They
also share nothing in the backend — separate state, separate loops, both can run
at once.
**Rejected:** replacing the widget's primary button with a mode-aware one (the
two have different stop semantics and different failure surfaces), and a second
window (the widget is the app; ui.md forbids modals and drawers).
**Consequence:** a **Clips | Session** segmented control at the top of the 320px
widget swaps the pane below it. Both statuses are polled from the root regardless
of which pane is showing, so the inactive tab still carries a live red dot — a
user can never hide a running recording by switching tabs.

## 2026-07-27 — `ts` means epoch-ms for a session link and `HH-MM-SS` for a clip link
**Why:** the viewer already deep-links clips as `?mode=viewer&date=…&ts=HH-MM-SS`.
A session has no per-clip timestamps to name — its playhead is an absolute
instant on the master clock, which is epoch-ms everywhere in the engine and the
manifest. Inventing a second parameter name for the same idea would have made the
scheme harder to read, not easier.
**Rejected:** `?session=…&at=…` (two names for one concept), and an offset in
seconds from session start (breaks the moment a rebuild re-anchors the start).
**Consequence:** `date` and `session` are the discriminator. `ViewerApp` reads
`ts` as a number only in the `session` branch and passes it to
`SessionPlayer.initialEpochMs`, which goes through `engine.seekTo` — so a stale
link is clamped into the session rather than parking the playhead in nothing.

## 2026-07-27 — "Recording" in the session browser requires a live recorder, not a null end time
**Why:** `SessionSummary.ended_epoch_ms === null` was being read as "still
recording". Two of the six real sessions on disk were force-killed and never got
an end time written, so the browser showed them as recording — permanently, and
for two sessions at once, which is impossible.
**Rejected:** having the backend backfill `ended_epoch_ms` during
`list_sessions` (it would be a guess written into the manifest as fact), and
dropping the badge entirely (losing the one state worth seeing at a glance).
**Consequence:** the browser takes `liveSessionId` from `get_session_status`.
Only that id gets the red **Recording** badge; any other session with no end time
gets an amber **Interrupted** badge explaining that the footage is intact but the
last segment of each track may be unplayable.

## 2026-07-27 — Amber is the widget's advisory tone
**Why:** the session recorder has a class of outcome that is neither fine nor
broken: the mic was skipped because the only one is a Bluetooth headset, a
monitor is not capturable, a track is restarting. ui.md's light palette had only
sky (live/selected) and red (recording/destruction), so these were going to
render either as alarming failures or as invisible grey.
**Rejected:** red for everything not perfect (trains the user to ignore red, and
red already means "recording" two inches higher up), and muted slate (the
Bluetooth explanation is the single most important sentence the recorder emits —
it explains why the user's headphones did *not* degrade).
**Consequence:** amber-50/amber-800 notices for advisories, red-50/red-700 for
failures, and amber was already the marker/`drift-warn` colour on dark surfaces,
so nothing new entered the palette. Documented in ui.md.

## 2026-07-27 — Session options live in the widget, not in `CaptureConfig`
**Why:** `SessionOptions` is read once, when `start_session` spawns the tracks;
`CaptureConfig` is the interval capture's persistent-ish state. Merging them
would have implied that changing fps mid-session does something (it does not) and
coupled two features that share no backend state.
**Rejected:** persisting options to disk as part of this change (nothing in this
app persists anything yet — that is one decision, to be made once, for config
generally).
**Consequence:** the settings panel holds a `SessionOptions` in React state,
seeded from a mirror of `SessionOptions::default()`, and posts the whole object
to `start_session`. Two Rust tests pin that mirror: one deserializes the exact
payload the widget sends, one asserts the defaults match. The panel is disabled
while recording, because the backend has already read it.

## 2026-07-27 — A seam under 250ms is a keyframe artifact, not a gap
**Why:** real recordings do not produce contiguous segments. Measured across the
first 6 real sessions (108 seams): 60 seams left a 0–50ms hole, 2 more up to
250ms, and **41 seams actually overlapped**, by as much as 4.2s on `sysaudio`.
Strict `[start, start+duration)` containment made `segmentAt()` return "nothing
here" inside those sub-frame holes, so every track blanked to **No footage** for
a frame at most segment boundaries — 1.04s of spurious blanking across 30 tracks.
The synthetic fixture was perfectly contiguous and never showed it.
**Rejected:** trusting `contig` and rewriting segment starts on load (edits the
backend's measured truth), rendering the previous frame frozen (hides that the
clock moved on).
**Consequence:** `segmentAt()` rolls onto the next segment when a hole is
≤ `SEAM_TOLERANCE_MS` (250ms), and the binary search naturally prefers the later
segment where two overlap. **No footage** now means only a gap the manifest
declares. Verified: 715,393 instants sampled at 10ms across all 6 sessions, zero
unexplained blanks.

## 2026-07-27 — `SessionSummary` mirrors the Rust struct, and derives nothing extra
**Why:** the frontend type was written before `list_sessions` existed and invented
two fields the backend never sends — `screen_count` and `recording`. Against real
data the browser rendered "undefined + NaN" for the track count. A type that
describes what we wish the wire format were is worse than no type.
**Rejected:** having the backend add the fields (screen count is a display
detail; "recording" is just `ended_epoch_ms === null`).
**Consequence:** `SessionSummary` matches `recorder::SessionSummary` field for
field, including `has_manifest`. Anything else is computed at the render site.
When that struct changes, this type changes with it.

## 2026-07-27 — The session player opens muted, every time
**Why:** it first defaulted to the mic track, on the theory that a life recorder's
audio is the point. In practice a session can be opened by a deep link or
restored on load, so the machine started playing its own microphone recording out
loud with no user action — which is exactly what happened during development, out
of the user's speakers. A recorder that plays your own room back at you
unprompted is a privacy problem before it is an annoyance.
**Rejected:** remembering the last chosen source (same surprise, one session
later), autoplaying at low volume (still audible, now also hard to find).
**Consequence:** `audioId` starts `null`, the transport's audio-source control
shows **Muted** selected, and the engine mutes every element until the user picks
a track. The standby element of each pair is muted unconditionally, so a swap can
never briefly unmute a second stream.

## 2026-07-27 — Two media elements per track, pre-rolled, instead of one
**Why:** a session is hours of 60s files. With a single `<video>` per track,
every boundary is `src = …; load()` — a network round trip plus decoder init,
which stalls visibly and desyncs that track from the others. Holding a second
element that has already buffered the next segment turns a boundary into an
opacity swap plus `play()`. Measured: every boundary crossed as a swap, 0 cold
loads, worst frame gap 64ms (one dropped frame at 30fps).
**Rejected:** MediaSource Extensions (would need the backend to emit fragmented
MP4 and an init segment per track — the recorder writes plain segment MP4s, and
AV1-in-fMP4 support is a second unknown); one element with `preload` tricks (no
way to buffer a *different* URL ahead of time).
**Consequence:** N tracks means 2N media elements. The inactive one is
`opacity: 0`, never `display: none` — a hidden element stops decoding.

## 2026-07-27 — The session player lives in `src/ui/session/`, not inline
**Why:** the rule "the two app files are the app" exists so nobody adds to the
dead `src/ui/components/`. The multi-track player is ~1400 lines with a
non-visual core (the sync engine) that must be readable on its own; folding it
into `ViewerApp.tsx` would triple that file and bury the one algorithm that
decides whether the feature works.
**Rejected:** inlining into `ViewerApp.tsx` (unreadable), reviving
`src/ui/components/` (that directory is dead and stays dead).
**Consequence:** `src/ui/session/` is the single exception. `ViewerApp.tsx` still
owns routing and mounts `SessionBrowser` / `SessionPlayer` as two more views.

## 2026-07-27 — Playback speed is capped by decode, not by the sync engine
**Why:** at 4x in grid layout the machine must decode 5120×2160 AV1 plus two
3840×2160 H.264 streams at 120fps. It cannot; frames drop, tracks fall behind,
and the inter-track spread goes from 4ms to several hundred. A stall guard that
skips correcting a starved element was tried and rejected — it made playback
smoother but let tracks drift a full 2.7s apart, which is exactly the thing this
feature exists to prevent.
**Rejected:** silently capping the speed control (hides a real capability —
4x is fine in Focus layout, median 13ms), decode-capability probing (no
reliable API).
**Consequence:** all four speeds stay offered. Staying aligned outranks staying
smooth, so the resync loop keeps hard-seeking a lagging track. 1x and 2x are
clean in every layout; 4x is a Focus-layout speed.

## 2026-07-27 — Tailwind v4 over the inline `<style>` block in index.html
**Why:** the previous build carried ~800 lines of hand-written CSS inside
`index.html`, which meant no scoping, no reuse between the widget and the viewer,
and every class name being invented twice. Tailwind's `@tailwindcss/vite` plugin
needs no config file, so the migration cost was one `@import` in `app.css`.
**Rejected:** CSS modules (another build concept for a two-file frontend),
styled-components (runtime cost in a video-heavy view), keeping the inline block
(unmaintainable at its current size).
**Consequence:** all styling is utility classes inline in `CompactApp.tsx` /
`ViewerApp.tsx`; only keyframes, resets, and scrollbars stay in `app.css`.

## 2026-03-07 — Auto-detect monitors via PowerShell instead of trusting monitor-config.json
**Why:** the hand-maintained `monitor-config.json` went stale whenever a display
was added, moved, or rescaled, silently breaking per-monitor zoom.
`System.Windows.Forms.Screen::AllScreens` gives the real layout, and rescaling by
`3840 / virtualDesktopWidth` maps it exactly onto FFmpeg's scaled output.
**Rejected:** a Rust winapi enumeration crate (extra dependency for one call),
asking the user to draw regions in the UI (the old flow — nobody kept it current).
**Consequence:** `monitor-config.json` is now only a fallback, and
`get_monitor_config` pays a PowerShell spawn per call.

## 2026-03-07 — Viewer is a separate window opened on demand, not a route in the widget
**Why:** the widget is 320×540 and always-on-top; browsing an archive of video
needs 1280×800 and must not drag the always-on-top pill around with it. Tauri can
build a second webview against the same bundle for free.
**Rejected:** resizing the main window between modes (loses the always-on-top pill
identity), a single window with an in-app router (unusable at 320px).
**Consequence:** `?mode=viewer` in `main.tsx` chooses the root component, and
deep links (`&date=&ts=`) are passed as query params by `open_viewer`. Every new
window label must also be added to `capabilities/default.json`.

## 2026-03-05 — Serve media over a local Warp server on :9120, not the Tauri asset protocol
**Why:** video scrubbing needs HTTP `Range` responses. A tiny Warp server in its
own thread gives byte-range support, correct MIME types, and normal `<video src>`
semantics, with a canonicalize check to block path traversal outside the output dir.
**Rejected:** Tauri's asset protocol / `convertFileSrc` (awkward Range support),
base64 over IPC (unusable for multi-MB clips).
**Consequence:** port 9120 is hardcoded in both `lib.rs` and `hooks/mediaUrl.ts`,
and the frontend is useless if that server fails to bind.

## 2026-03-05 — Shell out to FFmpeg rather than link a capture library
**Why:** one binary already handles screen (gdigrab), webcam+audio (dshow),
encoding, burned-in timestamps, and PiP compositing. Three parallel `Command`
spawns keep the screenshot and both videos in sync, and each capture is
independently debuggable by copying the args into a terminal.
**Rejected:** `ffmpeg-next` bindings (heavy build, no gdigrab convenience),
`screenshots`/`scrap` crates (no video, no webcam).
**Consequence:** FFmpeg must be on PATH (or in two hardcoded Windows locations),
and every spawn must use `silent_command()` so `CREATE_NO_WINDOW` suppresses the
console flash — which would otherwise appear in the capture itself.

## 2026-03-05 — Windows-only, deliberately
**Why:** gdigrab, dshow, the Win32 `Beep` chirp, `CREATE_NO_WINDOW`, and
PowerShell monitor detection are all Windows APIs, and this is a single-user tool
on a Windows machine. Abstracting them would triple the capture code for zero
current users.
**Rejected:** an x11grab/avfoundation abstraction layer (speculative), Electron +
web `getDisplayMedia` (no background capture without a visible prompt).

## 2026-03-05 — Tauri app supersedes the Bun CLI; the CLI stays in-tree
**Why:** the original `src/index.ts` + `src/viewer.ts` CLI proved the capture
pipeline and is kept as a working reference, but the Tauri app owns the product.
Deleting it wasn't worth the churn while the Rust port was still settling.
**Rejected:** deleting it immediately (reference value), maintaining both (double
work).
**Consequence:** `src/index.ts`, `src/capture/`, `src/utils/`, `src/viewer.ts`,
and `src/viewer-ui/` are legacy with no `package.json` script pointing at them,
and `src/ui/App.tsx` + `src/ui/components/` are dead leftovers of the first Tauri
viewer. Do not extend any of them.
