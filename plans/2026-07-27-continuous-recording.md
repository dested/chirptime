# Continuous Session Recording — Implementation Spec

**Status: done** · Created 2026-07-27 · Shipped 2026-07-27 · Owner: multi-agent build

Shipped and measured. Verified on real data: no progressive drift across 191
probed files; worst playback drift **61.6ms** over 185s with 6 boundary crossings
and zero hard seeks; cross-track spread 22ms typical / 53.6ms peak; system audio
+5ms over 75s including 50s of true silence; all 4 monitors capture at 13% of one
core. **Still unverified: a multi-hour soak run** (longest session to date is
~7 minutes) and human judgement of picture quality and audio lip-sync.

Goal: replace XSplit for all-day "life recording". Record every monitor + webcam +
mic + system audio continuously for hours, as separate tracks that **line up
perfectly** on playback, without crashing, leaking memory, or losing everything
when something goes wrong.

This is a NEW mode. The existing chirp/interval clip capture (`run_capture_cycle`)
stays exactly as it is and must keep working.

---

## 0. Verified facts about THIS machine (measured 2026-07-27, do not re-litigate)

FFmpeg 7.1.1-full (gyan.dev), GPU: RTX 4090 + Intel UHD 770 (iGPU).

| Fact | Evidence |
| --- | --- |
| `ddagrab` → `h264_nvenc` works **zero-copy**, no filters, native res | 3840×2160@30 sustained, ~2–10 Mbps |
| Two concurrent 4K ddagrab+NVENC = **29% encoder util, 14% GPU** | `nvidia-smi` during stress test |
| **D3D11→CUDA derive is NOT supported** in this build | `hwmap=derive_device=cuda` → `-40 Function not implemented` |
| CPU downscale path (`hwdownload,scale`) runs at **0.8x realtime** | unusable — never use it |
| **`h264_nvenc` fails above 4096px wide** | 5120×2160 → "Error while opening encoder" |
| `hevc_nvenc` and `av1_nvenc` both encode 5120×2160 fine | verified, 6s clips produced |
| `ddagrab` sees only **3 outputs** on the default adapter | `output_idx=3` → "Generic error in an external library" |
| Segment boundaries land on **keyframes**, not clock ticks | got 40/46/50/56s instead of 40/45/50/55 |
| Logitech BRIO needs `vcodec=mjpeg` for ≥1280×720 | `yuyv422` maxes out at low res |
| **No audio loopback device exists** | no VB-Cable, no Stereo Mix; "XSplit Audio" only works while XSplit runs |

**Consequence: there is no GPU scaler available, so we capture at native
resolution and never scale during capture.** That is also why this will be far
lighter than XSplit — the entire pipeline stays on the GPU.

### Monitor layout (physical pixels)

| ddagrab | Physical | Notes |
| --- | --- | --- |
| `output_idx=0` | 5120×2160 | ultrawide, primary — **must use `av1_nvenc`** |
| `output_idx=1` | 3840×2160 | `h264_nvenc` |
| `output_idx=2` | 3840×2160 | `h264_nvenc` |
| (4th monitor) | unknown | on the Intel iGPU — best-effort, see §7 |

⚠️ `System.Windows.Forms.Screen` (used by the existing `detect_monitors`) returns
**DPI-scaled logical** coords — it reports the 5120×2160 panel as 2926×1234.
Do not use it for this feature. Use Tauri's `app.available_monitors()`, which
returns `PhysicalPosition`/`PhysicalSize`.

---

## 1. Encoder selection rule

```
width > 4096          -> av1_nvenc      (h264_nvenc hard-fails; AV1 decodes natively in Chromium/WebView2)
width <= 4096         -> h264_nvenc     (widest playback compatibility)
no NVENC available    -> libx264 -preset ultrafast, and WARN the user loudly
```

Prefer **AV1 over HEVC** for the >4096 case. HEVC-in-MP4 playback in WebView2
requires an OS codec extension the user may not have; AV1 is decoded by
Chromium's bundled dav1d. This must be verified in the actual app window (§7).

### Verified-good screen capture command

```
ffmpeg -hide_banner -loglevel warning
  -f lavfi -i "ddagrab=output_idx=N:framerate=30:draw_mouse=1"
  -c:v h264_nvenc -preset p4 -tune ll -rc vbr -cq 30 -b:v 0 -maxrate 40M -bufsize 80M
  -fps_mode cfr
  -g 60                          # ⚠️ MANDATORY — see below. NOT -force_key_frames.
  -f segment -segment_time 60 -segment_atclocktime 1 -reset_timestamps 1
  -strftime 1 -segment_format mp4
  "<dir>/%Y%m%d-%H%M%S.mp4"
```

### Verified-good webcam command

```
ffmpeg -f dshow -rtbufsize 512M -vcodec mjpeg -video_size 1920x1080 -framerate 30 -i "video=<CAM>"
       -f dshow -rtbufsize 512M -i "audio=<MIC>"
       -c:v h264_nvenc -preset p4 -cq 28 -c:a aac -b:a 128k  ...same segment args...
```

Note: video and audio must be **separate `-f dshow` inputs**. The combined
`video=X:audio=Y` form threw `I/O error` in testing. Always probe with
`-list_options true` before assuming a resolution is supported.

---

## 2. On-disk layout

```
output/sessions/<YYYY-MM-DD_HH-MM-SS>/
  session.json              manifest — rewritten atomically (tmp + rename) after each segment
  screen-0/20260727-115640.mp4 ...
  screen-1/...
  screen-2/...
  webcam/...
  mic/...                   AAC/Opus segments
  sysaudio/...              AAC/Opus segments
  markers.jsonl             append-only
```

**The manifest must be reconstructible by scanning the directory alone** — the
filenames carry wall-clock time. If the app is killed, a rebuild pass recovers
the session. The manifest is an index, never the only copy of the truth.

## 3. Manifest schema (contract — backend writes, frontend reads)

```jsonc
{
  "version": 1,
  "id": "2026-07-27_11-56-40",
  "started_epoch_ms": 1785000000000,
  "ended_epoch_ms": null,               // null while still recording
  "segment_seconds": 60,
  "desktop": { "x": -2194, "y": -1234, "w": 7314, "h": 3394 },   // PHYSICAL px bounding box
  "tracks": [{
    "id": "screen-0",
    "kind": "screen",                    // screen | webcam | mic | system-audio
    "label": "DISPLAY1 (5120x2160)",
    "monitor": { "output_idx": 0, "adapter": 0, "x": -2194, "y": -1234, "w": 5120, "h": 2160 },
    "codec": "av1",
    "width": 5120, "height": 2160, "fps": 30,
    "start_epoch_ms": 1785000000123,     // MEASURED, per track — not assumed
    "dir": "screen-0",
    "segments": [
      { "file": "20260727-115640.mp4", "start_epoch_ms": 1785000000123,
        "duration_ms": 60000, "bytes": 1234567 }
    ],
    "gaps": [{ "from_epoch_ms": 0, "to_epoch_ms": 0, "reason": "device lost" }]
  }],
  "markers": [{ "epoch_ms": 0, "label": "kid walked in" }]
}
```

`start_epoch_ms` per segment is **measured**, via `ffprobe` on close (duration)
plus the `-strftime` filename (wall clock). Never assume track N segment K
starts at the same instant as track M segment K.

## 4. Sync strategy — the core requirement

Three independent layers. All three are required.

1. **Capture** — all processes spawned as close together as possible; each
   records its own spawn instant. CFR (`-fps_mode cfr`) so frame index maps
   linearly to time. Forced keyframes every 2s + `-segment_atclocktime 1` keeps
   boundaries close to clock ticks (within ~2s, not exact — that's fine).
2. **Index** — after each segment closes, `ffprobe` it for exact duration; store
   absolute `start_epoch_ms`. Drift is re-anchored every segment, so it **cannot
   accumulate** over a 6-hour session. This is what makes long sessions stay
   aligned.
3. **Playback** — a master virtual clock in epoch-ms. Each `<video>` is set to
   `currentTime = (masterEpochMs - segment.start_epoch_ms) / 1000`. A resync loop
   runs every ~250ms:
   - `|drift| > 150ms` → hard seek
   - `30ms < |drift| <= 150ms` → nudge `playbackRate` by ±3% until corrected
   - else leave alone

   **Layer 3 is not optional.** N independent `<video>` elements drift apart even
   with perfectly aligned files, because each has its own clock. Perfect files
   alone will NOT satisfy "they must line up perfectly."

## 5. Robustness rules (non-negotiable)

- **Never leave ffmpeg's stdout/stderr as an unread `Stdio::piped()`.** The OS
  pipe buffer (4–64KB) fills, ffmpeg blocks on write, and recording silently
  freezes ~20 minutes in. This is the single most likely bug in this feature.
  Use `Stdio::null()` for stdout, and a **dedicated reader thread** that drains
  stderr into a bounded ring buffer (keep the last ~200 lines, discard the rest).
- Use `-progress pipe:1` + a reader thread as the health heartbeat.
- **Graceful stop: write `q` to ffmpeg's stdin**, then wait up to 5s, then kill.
  Killing outright loses the in-flight segment's moov atom.
- **Per-track watchdog**: if no new segment appears for 2× `segment_seconds`, or
  the process exits unexpectedly, restart *that track only*, record a `gap` in
  the manifest, and keep the session alive. A webcam being unplugged must not
  end a 4-hour screen recording.
- Bounded memory everywhere. No unbounded `String`/`Vec` accumulation of ffmpeg
  output. A 6-hour session must show flat memory.
- **Disk full**: on write failure, stop the session *cleanly* — finalize the
  manifest, mark tracks ended, surface the error to the UI. **Never delete or
  overwrite any existing capture.** There is no retention policy by user choice.
- Every subprocess goes through `silent_command()` (CREATE_NO_WINDOW) or a
  console flashes on screen and lands in the recording.

## 6. System audio (WASAPI loopback)

No loopback device exists on this machine, so this is implemented natively.

```rust
// src-tauri/src/sysaudio.rs — self-contained
pub struct SysAudioCapture { /* ... */ }

impl SysAudioCapture {
    /// Start WASAPI loopback on the default render endpoint. Writes interleaved
    /// f32le stereo @48kHz continuously to `sink`, padding SILENCE during gaps,
    /// anchored at `t0_epoch_ms`. Returns immediately; runs on its own thread.
    pub fn start(sink: Box<dyn std::io::Write + Send>, t0_epoch_ms: i64) -> Result<Self, String>;
    pub fn stop(self);
    pub fn is_healthy(&self) -> bool;
}
```

The recorder spawns `ffmpeg -f f32le -ar 48000 -ac 2 -i pipe:0 -c:a aac -b:a 160k
<segment args>` and hands the `ChildStdin` in as `sink`.

⚠️ **WASAPI loopback delivers no packets at all when nothing is playing.** You
MUST synthesize silence to fill those gaps, or the audio track shortens relative
to wall clock and desyncs progressively. Track elapsed wall time and pad.

Also handle: default output device changing mid-session (re-initialize), and
format negotiation (resample to 48kHz stereo f32 if the endpoint differs).

## 7. Known-unknowns to resolve empirically

- **4th monitor / Intel iGPU.** `-init_hw_device d3d11va=dda:1 -filter_hw_device dda`
  returned 5120×2160 — i.e. it still landed on the ultrawide, so adapter indexing
  did not do what was expected. Needs real investigation. Best-effort: if it
  can't be solved cleanly, ship 3 monitors and expose the 4th as unavailable with
  a clear reason. Do NOT fall back to gdigrab silently — it costs real CPU.
- ~~**AV1-in-MP4 playback inside WebView2.**~~ **RESOLVED 2026-07-27 — AV1 confirmed,
  HEVC confirmed DEAD.** Measured in the Edge/Chromium engine that backs WebView2:
  - `canPlayType('video/mp4; codecs="av01.0.08M.08"')` → `probably`
  - `canPlayType('video/mp4; codecs="hvc1.1.6.L120.B0"')` → **`NO`** (no OS HEVC
    extension installed on this machine)
  - Real decode of the 5120×2160 `av1_nvenc` sample: `videoWidth=5120`,
    `readyState=4`, playback advanced normally, no error.

  So the §1 rule stands and is now load-bearing: **>4096px wide MUST be
  `av1_nvenc`.** Do NOT "fall back to HEVC" — HEVC does not play here and would
  make the ultrawide silently unviewable. If AV1 encode is ever unavailable, the
  correct fallback is to fail loudly, not to emit an unplayable file.
- **Webcam device contention.** The BRIO returned "device already in use by
  other application" during testing (XSplit was likely holding it). Surface this
  as a clear, actionable UI error rather than a silent failed track.
- **NVENC concurrent session limit.** 3 screens + webcam = 4 sessions. Consumer
  drivers ≥550 allow 8. Verify, and degrade gracefully with a clear message.

## 8. Out of scope for this build

Transcription. But keep it cheap to add later: write a **mono 16kHz sidecar of
the mic track** alongside the AAC, so whisper can be pointed at it with no
re-encode. Nothing else transcription-related.

---

## 4th monitor — findings

**Status: SOLVED.** The 4th monitor is on the Intel iGPU, it *is* reachable, and
it should be captured **and encoded** entirely on the iGPU via `h264_qsv`.
Measured 2026-07-27. Everything below was verified on this machine, not reasoned
about.

### 9.1 The real adapter → output map

Measured with `IDXGIFactory1::EnumAdapters1` + `IDXGIAdapter::EnumOutputs` — the
exact enumeration ffmpeg's d3d11va hwcontext uses, so these indices are the ones
`-init_hw_device d3d11va=x:N` and `ddagrab=output_idx=M` operate on.

| DXGI adapter | GPU | `output_idx` | Device | Physical rect |
| --- | --- | --- | --- | --- |
| **0** | NVIDIA RTX 4090 | 0 | `\.\DISPLAY1` | 5120×2160 at (0, 0) |
| **0** | NVIDIA RTX 4090 | 1 | `\.\DISPLAY2` | 3840×2160 at (5120, −15) |
| **0** | NVIDIA RTX 4090 | 2 | `\.\DISPLAY3` | 3840×2160 at (−3840, −15) |
| **1** | Intel UHD 770 | 0 | `\.\DISPLAY5` | **3840×2160 at (729, −2160)** |
| 2 | Microsoft Basic Render Driver | — | (no outputs) | software adapter, ignore |

The iGPU theory is **confirmed**. `output_idx=3` failed simply because adapter 0
has exactly 3 outputs — `output_idx` is *per adapter*, not global. The identical
"Generic error in an external library" reproduces on adapter 1 with
`output_idx=1`, so that error just means "output index out of range".

Desktop bounding box (physical): **x −3840, y −2160, w 12800, h 4320.**

> ⚠️ **`IDXGIOutput::GetDesc` lies unless the process is DPI-aware.** A plain
> console probe reported the ultrawide as 2926×1234 and DISPLAY5 as 2194×1234 —
> the same DPI-virtualised numbers `System.Windows.Forms.Screen` gives. Calling
> `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` first changed them to the
> true 5120×2160 / 3840×2160 above. **The recorder process must set per-monitor
> DPI awareness before reading any monitor rect**, or the manifest's `desktop`
> box and every `monitor` rect will be silently wrong by the scale factor (175%
> here). The `desktop` example in §3 above (`w: 7314, h: 3394`) is in logical
> coords and should be updated.

### 9.2 Root cause: `-f lavfi -i` silently ignores `-filter_hw_device`

The earlier experiment wasn't wrong about the adapter index — the hw device was
never applied at all. The **lavfi demuxer builds its own filtergraph and does not
receive `filter_hw_device`**, so `ddagrab` fell back to creating its own D3D11
device on the default adapter (0) and captured the ultrawide. Using
`-filter_complex` instead — which *does* get the device — fixes it.

Measured, reporting the negotiated stream size:

| # | Command shape | Result |
| --- | --- | --- |
| A | `-f lavfi -i ddagrab=output_idx=0`, no hw device | 5120×2160 |
| B | `d3d11va=dda:1` + **`-f lavfi -i`** ← the earlier attempt | **5120×2160** (device ignored) |
| C | `d3d11va=dda:1` + **`-filter_complex`** | **3840×2160** ✅ the 4th monitor |
| D | `d3d11va=dda:0` + `-filter_complex`, `output_idx=0` | 5120×2160 |
| E | `d3d11va=dda:1` + `-filter_complex`, `output_idx=1` | "Generic error in an external library" (only 1 output) |

`ddagrab` has no adapter option of its own (`ffmpeg -h filter=ddagrab` lists only
`output_idx`), so the hw device is the *only* way to choose the adapter.

### 9.3 Verified-good 4th-monitor command

`h264_qsv` on the iGPU. Capture and encode both stay on the Intel adapter — zero
cross-adapter copies, and the 4090 is not touched at all.

```
ffmpeg -hide_banner -loglevel warning
  -init_hw_device d3d11va=dda:1 -init_hw_device qsv=qs@dda -filter_hw_device dda
  -filter_complex "ddagrab=output_idx=0:framerate=30:draw_mouse=1,hwmap=derive_device=qsv,format=qsv"
  -c:v h264_qsv -preset medium -b:v 12M -maxrate 20M
  -fps_mode cfr -g 60
  -f segment -segment_time 60 -segment_atclocktime 1 -reset_timestamps 1
  -strftime 1 -segment_format mp4
  "<dir>/%Y%m%d-%H%M%S.mp4"
```

D3D11 → QSV `hwmap=derive_device` **works** (oneVPL 2.15, unlike the D3D11 → CUDA
derive which is unsupported in this build). Output is ordinary H.264, so unlike
the ultrawide's AV1 it carries no WebView2 playback risk.

### 9.4 NVENC cross-adapter: does not work — use QSV

| Attempt | Result |
| --- | --- |
| Intel capture → `h264_nvenc` direct | `OpenEncodeSessionEx failed: no encode device` |
| Intel capture → `hwdownload,format=bgra` → `h264_nvenc` | same failure |
| …plus an explicit `-init_hw_device cuda=cu:0` | `-16 Device or resource busy`, 0 frames |

ffmpeg hands the encoder the filtergraph's hw device — the Intel d3d11va one —
and NVENC has no encode engine there. There is no per-encoder device override in
the CLI. **This is fine: QSV is the better answer anyway.** The gdigrab fallback
was not needed and is not recommended.

### 9.5 Measured cost — all 4 monitors at once, 45s, with production segment args

3× on the 4090 (1× `av1_nvenc` 5120×2160 + 2× `h264_nvenc` 3840×2160) plus the
Intel `h264_qsv` track. `-benchmark`, 32 logical cores:

| Track | Encoder | CPU | share of one core |
| --- | --- | --- | --- |
| screen-0 5120×2160 | `av1_nvenc` | 1.25 s / 45.1 s | 3% |
| screen-1 3840×2160 | `h264_nvenc` | 0.98 s / 45.1 s | 2% |
| screen-2 3840×2160 | `h264_nvenc` | 1.02 s / 45.1 s | 2% |
| **screen-3 3840×2160 (iGPU)** | **`h264_qsv`** | **2.61 s / 45.2 s** | **6%** |
| **total** | | **5.86 s / 45 s** | **13% of one core = 0.4% of 32** |

`nvidia-smi` during the run: **GPU 13–15%, encoder 41%** (adding the 4th monitor
did not raise NVENC load at all — it's on the other GPU). Every track held a
clean 30fps: each 20s segment contains exactly 600 frames, and all 16 segments
decode with the right codec and resolution. Isolated, the QSV track alone runs at
`speed=0.985x`, 8% of one core.

### 9.6 Two bugs this uncovered in the sections above

**(a) `-force_key_frames expr:gte(t,n_forced*2)` is silently ignored** by both
`h264_nvenc` and `h264_qsv` in this ffmpeg build. Measured keyframe PTS over 12s:

| Args | Keyframes |
| --- | --- |
| `-tune ll -force_key_frames expr:gte(t,n_forced*2)` | `0.0` — **one, in 12 seconds** |
| `-force_key_frames …` (qsv) | `0.0, 8.57` — just the default GOP |
| **`-g 60`** (both encoders) | `0, 2, 4, 6, 8, 10` ✅ exactly every 2s |

Because `-tune ll` sets an infinite GOP, the two `h264_nvenc` tracks produced a
**single 45-second file with one keyframe and never segmented at all**. This
would have broken segmentation on the main monitors in production, not just the
4th. **Replace `-force_key_frames expr:…` with `-g 60` in §1.** With `-g 60` all
four tracks segmented at exactly 20.000s on clock boundaries.

**(b) Launching the ddagrab processes simultaneously fails.** §4 says to spawn
them "as close together as possible" — doing so makes Desktop Duplication return
`-16 Device or resource busy` for all but the first. Reproduced deliberately:

| Stagger between launches | Result (2 attempts each) |
| --- | --- |
| 0 s | **1 of 4 started**, both attempts |
| 0.15 s | 4 of 4, both attempts |
| 0.25 s | 4 of 4, both attempts |
| 0.5 s | 4 of 4, both attempts |

**Stagger the spawns by ~250–500ms and retry on `-16`.** This costs nothing for
sync: §4 layer 2 re-anchors every segment from the measured `start_epoch_ms`, so
a sub-second spawn offset is recorded, not accumulated.
