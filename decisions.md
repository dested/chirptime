# ChirpTime — Decisions

> Why things are the way they are. A recorded decision is settled unless the
> user reopens it. Newest first.
>
> Entries below dated 2026-03-xx were reconstructed from the code on 2026-07-27
> when this file was created; the reasoning is inferred from the implementation.

## 2026-07-27 — ChirpTime is MIT, and that is what keeps FFmpeg out of the installer
**Why:** the repo had no LICENSE at all, which made "can we ship FFmpeg with
this" unanswerable. MIT settles it, and the two questions turn out to be one:
ChirpTime **invokes** FFmpeg as a separate subprocess and neither links it nor
distributes its binaries, so FFmpeg's GPL does not reach ChirpTime's own
distribution. Bundling `ffmpeg.exe` into the MSI *would* make the installer a
GPL distribution; fetching it at the user's request does not.
**Chosen:** MIT `LICENSE`, and FFmpeg downloaded on first run — see the next
entry. The README says plainly that the build ChirpTime offers to fetch is GPL
and that the license covers that download, not ChirpTime.
**Rejected:** GPL for ChirpTime itself (would have made bundling free, but the
user chose permissive); shipping with no license (leaves the project
all-rights-reserved by default, which is worse for something published as open
source).
**Consequence:** the licensing argument now *depends* on FFmpeg staying a
subprocess. Anything that links an FFmpeg library — `ffmpeg-next`, a static
build — reopens this decision, as does putting the binary inside the installer.

## 2026-07-27 — FFmpeg is downloaded on first run, verified against a pinned hash
**Why:** the app shells out to FFmpeg for every capture, so someone who installs
the MSI and presses record gets an unexplained failure. Telling them to go and
install a second program is a real cliff at exactly the moment they are deciding
whether to bother. Bundling it is the other extreme and is what the MIT decision
above rules out.
**Chosen:** ask, then fetch. `src-tauri/src/ffmpeg_install.rs` downloads a pinned
gyan.dev **essentials** build over HTTPS, verifies a **compiled-in SHA-256**
before anything is extracted or executed, unpacks only `ffmpeg.exe` and
`ffprobe.exe` into `%APPDATA%\com.chirptime.app\ffmpeg\bin` (per-user, no
elevation), runs each binary once as a smoke test, and only then renames the
staging directory into place. The user is told the size *before* pressing the
button; nothing downloads on its own.
**Why essentials, not full:** 105 MB instead of 170 MB, and verified against the
actual archive to carry everything this app asks for — `libx264`, all three
NVENC encoders, `h264_qsv` via libvpl, `aac`, the `drawtext` filter that burns in
the timestamps, `overlay`, `scale`, and the `gdigrab` / `ddagrab` / `dshow`
inputs. `ffplay.exe` (another ~100 MB) is deliberately not extracted.
**Rejected:** bundling (licensing, above, plus 105 MB on every download for the
majority who already have FFmpeg); a silent background download (spending
someone's bandwidth unasked); trusting HTTPS alone without a hash (we download an
executable and then *run* it — TLS says who sent it, not what it is); extracting
the whole archive (doubles the install for a player the app never invokes);
`tauri-plugin-updater`-style signing infrastructure (a pinned hash needs no key
management and is auditable by reading one constant).
**Consequence:** the pin is three constants that must move together
(`FFMPEG_URL`, `FFMPEG_SHA256`, `FFMPEG_ZIP_BYTES`), and updating it requires
re-deriving the hash. `find_ffmpeg` gained a fifth candidate and
`session::find_ffprobe` had to be extended in lockstep — the recorder resolves
both, and an ffmpeg without a matching ffprobe records footage it cannot index.
A hand-picked folder (`settings.ffmpeg_dir`) sits *ahead* of everything so a
blocked download is not a dead end.

## 2026-07-27 — Releases are Windows-only, tag-triggered, and unsigned
**Why:** `tauri build` was producing a bare exe because `bundle.active` was never
set, so there was nothing to hand anyone. A tagged pipeline is the smallest thing
that turns a commit into something a stranger can install.
**Chosen:** `.github/workflows/release.yml` on `v*` tags, `windows-latest` only,
building MSI + NSIS + a portable zip of the bare exe, published with the built-in
`GITHUB_TOKEN` via `gh release create`. It is a **separate file** from the
Drydock-owned `.github/workflows/drydock.yml`, which is overwritten by that
system. **Version stays 1.0.0** — proposed as 0.1.0 on the grounds that the app
is early; the user kept 1.0.0, and the README says plainly that it is early
instead.
**No code signing.** No certificate exists and none is being bought, so the
installers are unsigned and SmartScreen shows "Windows protected your PC" on
first run. Documented in the README and in the release notes rather than worked
around: an unexplained blue box is what makes people delete the download.
**Rejected:** a mac/linux matrix — the capture path is gdigrab/ddagrab/dshow,
WASAPI COM, DXGI and NVENC/QSV behind 26 `#[cfg(target_os = "windows")]` blocks,
so those jobs could only ever fail (see *Windows-only, deliberately*);
`tauri-apps/tauri-action` (it creates the release but not the portable zip, and
`gh` needs no third-party trust); per-machine NSIS install (an unsigned installer
that also demands UAC is a worse first run than a per-user one).
**Consequence:** the version lives in three files and the workflow names its
uploads from the git tag while Tauri names bundles from `tauri.conf.json` — a
mismatch ships a file whose name lies about its contents.

## 2026-07-27 — Where recordings go is stated on first run, and never guessed at again
**Why:** the default output dir moved off the original author's hardcoded
`G:/code/screener/output` to `%USERPROFILE%\Videos\ChirpTime`. This app writes
hours of somebody's life at 5–8 GB/hour and **deletes nothing, ever** (the user
explicitly chose no retention policy), so the destination cannot be something you
discover afterwards. The first run states the resolved absolute path, the free
space, what it costs per hour, and that nothing is auto-deleted — and lets you
change it right there.
**Chosen:** an inline first-run panel that replaces the panes until dismissed
(ui.md forbids modals/toasts/drawers at 320px wide), plus a permanent
output-folder affordance in two places — the titlebar folder button and the
settings card — because "where are my recordings" is asked on day 180 too.
**Rejected:** a modal (banned by ui.md); a one-line banner (too easy to dismiss
unread for a decision this expensive); auto-migrating the old `output/` (moving
someone's footage without asking is not ours to do — it is reported with its
path, and adopting it is one button).
**Consequence:** the panel is gated on a persisted flag, so onboarding is one of
the reasons settings persistence had to exist first.

## 2026-07-27 — Settings persist to %APPDATA%, load tolerantly, and never block startup
**Why:** nothing survived a restart, which made the settings UI decorative and
made a first-run flag impossible. The file lives at
`%APPDATA%\com.chirptime.app\settings.json` — the same path Tauri's
`app_config_dir()` resolves to — but is computed **without** an `AppHandle`,
because settings are read in `run()` before the app and the file-server thread
exist and both need the output dir from the first request.
**Chosen:** field-by-field tolerant loading. Bad JSON → defaults; one bad key →
that key's default with every other key kept. Writes go through a temp file +
rename. A failed *write* is logged, not surfaced as a rejected UI action.
**Rejected:** `tauri-plugin-store` / a plugin dependency (needs an AppHandle,
and this must run earlier); all-or-nothing parsing (an unrelated bad key would
silently reset the output dir, i.e. send recordings somewhere unasked for).
**Consequence:** `recorder::SessionOptions` is deserialize-only and the recording
pipeline is off-limits, so session options are persisted as an opaque
`serde_json::Value` that is validated by parsing and merged over the frontend's
`DEFAULT_OPTIONS`. Adding `#[serde(default)]` to `CaptureConfig` is what lets an
older file keep the keys it does have.

## 2026-07-27 — Segment-boundary choppiness is ACCEPTED, and mitigated only by longer audio segments
**Why:** the user, told the tradeoff explicitly (~40s of audio lost per 6-hour
day, a brief hiccup around each boundary), chose to live with it: *"it doesnt
have to be nano second perfect, the entire time, it can be choppy here and
there."* This is a deliberate product call, not an oversight — **do not "fix" it
later without reopening the decision.**
**Measurement note for whoever revisits this:** investigation found no media loss
on any track (screen-3/QSV has the largest seam gaps and exactly 0.0ms of loss;
two consecutive mic segments held 469+468 AAC packets = 19989ms per 20000ms of
wall clock). The seam gaps are label jitter from the drift filter's biased input.
Judge any future claim of loss by media-vs-clock-span or packet count, never by
seam gaps — see cliffnotes Gotchas.
**Chosen mitigation:** `AUDIO_SEGMENT_SECONDS = 600` for `mic` and `sysaudio`
only, since the loss is per *boundary* and audio is cheap (~350MB per 6h at
128kbps). Cuts boundaries from ~360 to ~36 per six-hour day.
**Rejected:** changing the audio codec (risk, no time); chasing encoder-drain /
segment-muxer flush behaviour (explicitly dropped by the user); touching
screen-3/QSV video loss (accepted as-is).
**Consequence and the real cost:** a force-killed process leaves its final
segment without a moov atom, so a crash now loses up to 10 minutes of audio
instead of 60 seconds. This trades a small distributed loss for a rarer but much
larger one — the right call only while crashes stay rare. Segment length is
per-*output* (`OutputSpec.segment_seconds`), so the shared webcam+mic process
cuts video at the session length and audio at 600s in the same command.

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
