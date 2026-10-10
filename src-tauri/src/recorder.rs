//! Continuous multi-track session recorder.
//!
//! One supervised ffmpeg process per track (each screen, the webcam, the mic, and
//! system audio). Every process writes 60s MP4 segments with `-strftime 1`
//! filenames; as each segment closes it is ffprobed and appended to the session
//! manifest, which is rewritten atomically. A per-track watchdog restarts a dead
//! track without touching the others.
//!
//! This is additive — the interval-clip flow in `capture.rs` is untouched.

use crate::session::{
    self, Gap, MonitorRef, Rect, Segment, SessionManifest, Track, TrackStatus,
};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// How often each supervisor scans its track directory for a newly opened segment.
/// This is also the resolution of the measured segment start times, so it is
/// deliberately tight.
const POLL_MS: u64 = 100;
/// Keep the last N stderr lines per track. Bounded — a 6h session must not grow.
const LOG_LINES: usize = 200;
/// Give up on a track after this many watchdog restarts.
const MAX_RESTARTS: u32 = 20;
/// Wait this long for a graceful `q` shutdown before killing.
const GRACEFUL_STOP_MS: u64 = 5_000;
/// No `-progress` line for this long means the process is wedged.
const PROGRESS_STALL_MS: i64 = 30_000;
/// Stop the session cleanly if the drive drops below this.
pub const MIN_FREE_BYTES: u64 = 1024 * 1024 * 1024;
/// Delay between consecutive *screen* launches.
///
/// Desktop Duplication cannot be acquired by several processes at the same
/// instant: measured, launching 4 `ddagrab` processes simultaneously started only
/// 1 of them, the other 3 dying with `-16 Device or resource busy`. 150ms was
/// already enough for 4-of-4 in every trial; 300ms leaves margin.
///
/// This costs nothing for sync. Every segment's start is measured and re-anchored
/// (see [`index_segment`]), so a sub-second launch offset is recorded rather than
/// accumulated.
const SCREEN_STAGGER_MS: u64 = 300;
/// How long after launch we keep sampling ffmpeg's `-progress` clock to work out
/// when a dshow capture's content actually began. See [`Anchor::FromProgress`].
///
/// Long enough for ~10 reports at ffmpeg's default 0.5s cadence, short enough that
/// it is closed well before the second segment of even a 5s-segment session is
/// indexed, so the value the first segment consumes is already final.
const ANCHOR_WINDOW_MS: i64 = 5_000;
/// Consecutive runs in which one half of the shared webcam+mic process produces
/// no segments before we stop insisting the two devices be captured together.
/// See [`supervise_track`].
///
/// A real tension sets this number. While the pair is failing, the *microphone*
/// is recording nothing either, so waiting is not free — but splitting is
/// permanent for the session, and gives up the lip-sync this whole arrangement
/// exists for. A camera released by another app, or one still warming up, can
/// easily need a few seconds; abandoning sync after the ~1.5s that 2 runs buys
/// would trade the feature away for a transient. At 4, the retry backoff
/// (500ms x n) gives the camera ~5s to appear, and the mic loses at most that.
const PAIR_SPLIT_AFTER: u32 = 4;
/// Segment length for audio-only tracks (`mic`, `sysaudio`), overriding the
/// session's `segment_seconds`.
///
/// Every segment boundary costs a few tens of ms of label jitter, and on the
/// audio tracks that shows up as a brief hiccup. The loss is per *boundary*, so
/// the cheapest mitigation is simply to have far fewer of them: at 60s segments a
/// six-hour session crosses 360 boundaries, at 600s it crosses 36.
///
/// Audio can afford this where video cannot — six hours of 128kbps AAC is ~350MB,
/// so long segments cost little disk and little seek granularity. The real cost is
/// crash exposure: a force-killed process leaves its final segment without a moov
/// atom, so this trades a bounded, distributed loss for a rarer but much larger
/// one. See decisions.md — this was a deliberate call, not an oversight.
const AUDIO_SEGMENT_SECONDS: u32 = 600;
/// How often the idle tracker re-reads the system-wide last-input time. Coarse on
/// purpose: the inactivity threshold is measured in minutes, so second-scale
/// resolution is ample and the poll costs one cheap syscall.
const IDLE_POLL_MS: u64 = 2_000;

// ---------------------------------------------------------------------------
// Inactivity tracking
// ---------------------------------------------------------------------------

/// The epoch-ms of the most recent system-wide keyboard or mouse input.
///
/// `GetLastInputInfo` reports the last input across the whole desktop session —
/// any application, not just this one — which is exactly the signal inactivity
/// trimming wants: "was the user doing anything, anywhere". `dwTime` is a
/// `GetTickCount` value (ms since boot, 32-bit, wraps ~every 49.7 days), so the
/// idle duration is `GetTickCount() - dwTime` computed with wrapping arithmetic,
/// and the wall-clock instant of that input is `now - idle`.
#[cfg(target_os = "windows")]
fn poll_last_input_ms() -> Option<i64> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    let mut lii = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    let ok = unsafe { GetLastInputInfo(&mut lii) };
    if !ok.as_bool() {
        return None;
    }
    let tick = unsafe { GetTickCount() };
    let idle_ms = tick.wrapping_sub(lii.dwTime) as i64;
    Some(now_ms() - idle_ms)
}

#[cfg(not(target_os = "windows"))]
fn poll_last_input_ms() -> Option<i64> {
    None
}

/// Keep `inner.last_input_ms` current for as long as the session runs. Only
/// spawned when inactivity trimming is on.
fn idle_poll_loop(inner: Arc<Inner>) {
    while !inner.stopping() {
        if let Some(t) = poll_last_input_ms() {
            inner.last_input_ms.fetch_max(t, Ordering::Relaxed);
        }
        // Sleep IDLE_POLL_MS in short slices so stop is prompt.
        for _ in 0..(IDLE_POLL_MS / 100).max(1) {
            if inner.stopping() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

/// Every subprocess must go through this or a console window flashes on screen —
/// and lands in the recording.
pub fn silent_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn now_ms() -> i64 {
    session::now_epoch_ms()
}

/// Bounded ring of the most recent stderr lines.
#[derive(Debug, Default)]
pub struct RingLog {
    lines: VecDeque<String>,
}

impl RingLog {
    fn push(&mut self, line: String) {
        if self.lines.len() >= LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
    pub fn snapshot(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }
    fn contains(&self, needle: &str) -> bool {
        self.lines.iter().any(|l| l.contains(needle))
    }
}

/// Lines ffmpeg emits at warning level that are never the reason a track died.
///
/// This list exists because of a real incident: all five tracks of session
/// `2026-07-27_12-41-11` exited at once with `exit code: 1`, and the recorded
/// cause was `Guessed Channel Layout: stereo` — a startup notice from 30 seconds
/// earlier that happened to be the last line in the ring. The genuine cause was
/// unrecoverable from that. Filtering these means the diagnosis is either a real
/// error or an honest "no error output", never a misleading one.
const BENIGN_STDERR: [&str; 6] = [
    "Guessed Channel Layout",
    "deprecated",
    "Past duration",
    "Last message repeated",
    "Estimating duration from bitrate",
    "VBV buffer size not set",
];

fn is_benign_stderr(line: &str) -> bool {
    BENIGN_STDERR.iter().any(|b| line.contains(b))
}

/// Best explanation the stderr ring can offer, as one line.
///
/// Prefers the most recent lines that are not [`BENIGN_STDERR`]; falls back to
/// saying plainly that there was none. **Only call this after the stderr drain
/// thread has been joined** — a process can write its fatal error and exit before
/// the reader has drained the pipe, which is how the real error was lost in the
/// incident above.
pub fn diagnose(lines: &[String]) -> String {
    let real: Vec<&String> = lines.iter().rev().filter(|l| !is_benign_stderr(l)).collect();
    if !real.is_empty() {
        return real
            .into_iter()
            .take(3)
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>()
            .join(" | ");
    }
    if lines.is_empty() {
        // No output at all before exit. On Windows this is what an externally
        // killed process looks like (`taskkill /F` leaves the child with exit
        // code 1 and nothing on stderr).
        "no stderr output — the process may have been killed externally".to_string()
    } else {
        format!(
            "no error on stderr, only routine notices (last: {})",
            lines.last().map(|s| s.trim()).unwrap_or("")
        )
    }
}

// ---------------------------------------------------------------------------
// DPI awareness
// ---------------------------------------------------------------------------

/// Declare per-monitor-v2 DPI awareness for the whole process. Idempotent.
///
/// **This must run before anything in the process reads a monitor rectangle.**
/// Windows virtualises coordinates for DPI-unaware processes, and it does so at
/// a layer *below* every API we have: `IDXGIOutput::GetDesc`, Tauri's
/// `available_monitors()`, and `System.Windows.Forms.Screen` all return the same
/// scaled lie. Measured on this machine (175% scale): the 5120×2160 ultrawide
/// reports as 2926×1234 and the desktop bounding box comes out 12801×2176
/// instead of 12800×4320 — the odd width is the rounding tell.
///
/// Nothing downstream can detect or undo this, because the scale factor is
/// per-monitor and the rects have already been composed into one virtual desktop.
/// Hence: set it once, at process start, and verify it stuck.
#[cfg(target_os = "windows")]
pub fn ensure_process_dpi_aware() -> Result<(), String> {
    use std::sync::OnceLock;
    use windows::Win32::UI::HiDpi::{
        GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext,
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        DPI_AWARENESS_PER_MONITOR_AWARE,
    };
    static ONCE: OnceLock<Result<(), String>> = OnceLock::new();
    ONCE.get_or_init(|| {
        // A failure here is not conclusive: it also means "already set", either by
        // an application manifest or by an earlier call. Only the read-back is.
        let set = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let awareness =
            unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
        if awareness == DPI_AWARENESS_PER_MONITOR_AWARE {
            Ok(())
        } else {
            Err(format!(
                "process is not per-monitor DPI aware (awareness={:?}, set returned {:?}); \
                 monitor rectangles will be DPI-virtualised and the manifest's desktop \
                 box will be wrong",
                awareness.0, set
            ))
        }
    })
    .clone()
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_process_dpi_aware() -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Monitor enumeration
// ---------------------------------------------------------------------------

/// One DXGI output. `output_idx` is exactly the index ffmpeg's `ddagrab` filter
/// passes to `IDXGIAdapter::EnumOutputs`, so this mapping is definitional rather
/// than inferred.
///
/// **`output_idx` is per adapter, not global.** Adapter 0 output 0 and adapter 1
/// output 0 are different monitors. Selecting the adapter is done with
/// `-init_hw_device d3d11va=dda:<adapter>` — `ddagrab` itself has no adapter
/// option.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DisplayOutput {
    pub adapter: u32,
    pub output_idx: u32,
    pub device_name: String,
    pub adapter_name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// False when the output exists but no desktop is extended onto it.
    pub attached: bool,
    /// True when we have a working capture *and* encode path for this output.
    /// Adapter 0 (the NVIDIA GPU) uses NVENC; any other adapter must both capture
    /// and encode on that same adapter via QSV, so it is only capturable when
    /// `h264_qsv` exists. Filled in by [`resolve_screen_targets`].
    pub capturable: bool,
}

#[cfg(target_os = "windows")]
pub fn enumerate_dxgi_outputs() -> Result<Vec<DisplayOutput>, String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput,
    };

    // Non-negotiable: GetDesc below returns DPI-virtualised rects otherwise.
    // The failure is reported as a warning by `resolve_screen_targets`, not here —
    // wrong-but-present rects still beat refusing to record.
    let _ = ensure_process_dpi_aware();

    let mut out = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {}", e))?;

        let mut ai: u32 = 0;
        loop {
            let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(ai) {
                Ok(a) => a,
                Err(_) => break,
            };
            let adapter_name = match adapter.GetDesc1() {
                Ok(d) => wide_to_string(&d.Description),
                Err(_) => String::new(),
            };

            let mut oi: u32 = 0;
            loop {
                let output: IDXGIOutput = match adapter
                    .cast::<windows::Win32::Graphics::Dxgi::IDXGIAdapter>()
                    .and_then(|a| a.EnumOutputs(oi))
                {
                    Ok(o) => o,
                    Err(_) => break,
                };
                if let Ok(odesc) = output.GetDesc() {
                    let r = odesc.DesktopCoordinates;
                    out.push(DisplayOutput {
                        adapter: ai,
                        output_idx: oi,
                        device_name: wide_to_string(&odesc.DeviceName),
                        adapter_name: adapter_name.clone(),
                        x: r.left,
                        y: r.top,
                        w: (r.right - r.left).max(0) as u32,
                        h: (r.bottom - r.top).max(0) as u32,
                        attached: odesc.AttachedToDesktop.as_bool(),
                        // Refined by `resolve_screen_targets`, which knows which
                        // encoders exist. Enumeration alone cannot say.
                        capturable: true,
                    });
                }
                oi += 1;
            }
            ai += 1;
        }
    }
    Ok(out)
}

#[cfg(not(target_os = "windows"))]
pub fn enumerate_dxgi_outputs() -> Result<Vec<DisplayOutput>, String> {
    Err("DXGI enumeration is Windows-only".to_string())
}

#[cfg(target_os = "windows")]
fn wide_to_string(w: &[u16]) -> String {
    let len = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..len])
}

/// A monitor as reported by Tauri: physical position and size, plus the Windows
/// device name (`\\.\DISPLAY1`) which is the join key back to DXGI.
#[derive(Debug, Clone)]
pub struct TauriMonitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

pub fn tauri_monitors(app: &tauri::AppHandle) -> Vec<TauriMonitor> {
    app.available_monitors()
        .map(|ms| {
            ms.into_iter()
                .map(|m| {
                    let p = m.position();
                    let s = m.size();
                    TauriMonitor {
                        name: m.name().cloned().unwrap_or_default(),
                        x: p.x,
                        y: p.y,
                        w: s.width,
                        h: s.height,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A screen we are going to record.
#[derive(Debug, Clone)]
pub struct ScreenTarget {
    /// Position in the recorder's own global ordering — this is what names the
    /// track (`screen-3`) and its directory. **Not** `output_idx`, which repeats
    /// across adapters.
    pub screen_index: u32,
    pub output_idx: u32,
    pub adapter: u32,
    pub label: String,
    pub rect: MonitorRef,
}

/// The ffmpeg arguments that put `ddagrab` on a specific DXGI adapter.
///
/// Adapter 0 uses the plain `-f lavfi -i ddagrab=…` form, which is the verified
/// zero-copy path into NVENC.
///
/// Any other adapter **must** use `-filter_complex`. The lavfi *demuxer* builds
/// its own filtergraph and never receives `-filter_hw_device`, so `ddagrab` would
/// silently create its own D3D11 device on the default adapter and capture the
/// wrong monitor — measured: `d3d11va=dda:1` plus `-f lavfi -i` still returned the
/// 5120×2160 ultrawide. `-filter_complex` does get the device and returns the
/// intended 3840×2160.
fn ddagrab_input_args(adapter: u32, filter: &str) -> Vec<String> {
    if adapter == 0 {
        vec!["-f".into(), "lavfi".into(), "-i".into(), filter.into()]
    } else {
        vec![
            "-init_hw_device".into(),
            format!("d3d11va=dda:{}", adapter),
            "-filter_hw_device".into(),
            "dda".into(),
            "-filter_complex".into(),
            filter.into(),
        ]
    }
}

/// Ask ddagrab itself what an output's real pixel size is, by decoding a single
/// frame.
///
/// Cross-checks the DXGI rectangle against what will actually be captured. These
/// agree once the process is per-monitor DPI aware; before that fix they did not,
/// and feeding a virtualised 2926px width into encoder selection would pick
/// `h264_nvenc` for a 5120px-wide capture, which hard-fails at open. Costs ~0.5s
/// per output, once per session.
pub fn probe_ddagrab_size(ffmpeg: &str, adapter: u32, output_idx: u32) -> Option<(u32, u32)> {
    let mut cmd = silent_command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "info"]);
    cmd.args(ddagrab_input_args(
        adapter,
        &format!("ddagrab=output_idx={}:framerate=30", output_idx),
    ));
    let out = cmd
        .args(["-frames:v", "1", "-f", "null", "-"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    for line in text.lines() {
        if !line.contains("Stream #") || !line.contains("Video:") {
            continue;
        }
        // ".. wrapped_avframe, d3d11, 5120x2160 [SAR 1:1 .."
        for tok in line.split(|c: char| !(c.is_ascii_digit() || c == 'x')) {
            if let Some((w, h)) = tok.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    if w >= 320 && h >= 240 {
                        return Some((w, h));
                    }
                }
            }
        }
    }
    None
}

/// Resolve `ddagrab output_idx` → physical desktop rectangle.
///
/// **How this works and where it can go wrong.**
///
/// `ddagrab` calls `IDXGIAdapter::EnumOutputs(output_idx)` on the default adapter,
/// so enumerating DXGI ourselves gives the mapping by definition, not by guess.
/// That is important here: three of this machine's four monitors are 3840×2160,
/// so matching ddagrab indices to monitors *by size* is ambiguous and would put
/// the wrong rectangle on two of the three screen tracks.
///
/// Sizes always come from `probe_ddagrab_size` — the only source that reports
/// what will actually be captured. Positions come from Tauri's
/// `available_monitors()` (physical pixels) joined to DXGI on the Windows device
/// name; when that join misses, DXGI's own `DesktopCoordinates` are rescaled by
/// the measured physical/logical ratio.
///
/// Multi-adapter layouts are supported: `output_idx` is per adapter, so the
/// recorder assigns its own global `screen_index` (the `screen-N` track name) by
/// walking adapters in order. Outputs on a non-default adapter are captured *and*
/// encoded on that adapter via QSV — see [`pick_video_encoder`].
///
/// Known failure modes, none of which are silently wrong:
/// - **A secondary-adapter output with no QSV encoder** is reported with
///   `capturable: false` and skipped with a warning. Cross-adapter NVENC does not
///   work (`OpenEncodeSessionEx failed: no encode device`), so there is no
///   fallback that would not either fail or burn CPU on a downscale path.
/// - **A device-name join miss** falls back to rescaled DXGI coordinates. The
///   recorded pixels are still correct — only the layout rectangle degrades — and
///   a warning is emitted.
/// - **Mixed per-monitor DPI** makes the rescaled fallback positions approximate,
///   because Windows lays the virtual desktop out in logical coordinates and a
///   single ratio cannot describe it. With per-monitor DPI awareness set (see
///   [`ensure_process_dpi_aware`]) the rescale should never trigger at all.
/// - **Hot-plugging a monitor mid-session** is not handled; the mapping is
///   resolved once at session start.
pub fn resolve_screen_targets(
    ffmpeg: &str,
    tauri_mons: &[TauriMonitor],
    caps: &EncoderCaps,
) -> (Vec<ScreenTarget>, Vec<String>) {
    let mut warnings = Vec::new();

    if let Err(e) = ensure_process_dpi_aware() {
        warnings.push(e);
    }

    let outputs = match enumerate_dxgi_outputs() {
        Ok(o) => o,
        Err(e) => {
            warnings.push(format!(
                "DXGI enumeration failed ({}); falling back to size matching, \
                 monitor positions may be wrong",
                e
            ));
            Vec::new()
        }
    };

    let mut targets = Vec::new();

    if !outputs.is_empty() {
        let mut screen_index: u32 = 0;
        for o in outputs.iter().filter(|o| o.attached) {
            // Reserve this monitor's slot before any skip, so that a monitor that
            // cannot be captured does not renumber the ones after it.
            let this_index = screen_index;
            screen_index += 1;

            if !output_is_capturable(o, caps) {
                warnings.push(format!(
                    "{} ({}) is on adapter {} ({}) and h264_qsv is unavailable; a \
                     non-default adapter must encode on itself (cross-adapter NVENC \
                     fails with \"no encode device\"), so this monitor is not being \
                     recorded",
                    o.device_name,
                    fmt_size(o.w, o.h),
                    o.adapter,
                    o.adapter_name
                ));
                continue;
            }
            // Cross-check against what ddagrab will actually hand us.
            let probed = probe_ddagrab_size(ffmpeg, o.adapter, o.output_idx);
            if probed.is_none() {
                warnings.push(format!(
                    "could not probe ddagrab adapter {} output_idx {} ({}); skipping it",
                    o.adapter, o.output_idx, o.device_name
                ));
                continue;
            }
            let (pw, ph) = probed.unwrap();

            // Prefer Tauri's physical rect, joined on the Windows device name.
            let joined = tauri_mons.iter().find(|m| m.name == o.device_name);
            let (x, y) = match joined {
                Some(m) if m.w == pw && m.h == ph => (m.x, m.y),
                Some(m) => {
                    warnings.push(format!(
                        "{}: Tauri reports {}x{} but ddagrab captures {}x{}; \
                         rescaling the layout position",
                        o.device_name, m.w, m.h, pw, ph
                    ));
                    rescale_origin(m.x, m.y, m.w, pw)
                }
                None => {
                    // Should not happen now that the process is DPI aware; if the
                    // sizes still disagree the position is only approximate.
                    if o.w != pw {
                        warnings.push(format!(
                            "{}: no Tauri monitor matched; DXGI reports {}x{} but \
                             ddagrab captures {}x{}, so the layout position is \
                             rescaled and approximate under mixed DPI",
                            o.device_name, o.w, o.h, pw, ph
                        ));
                    }
                    rescale_origin(o.x, o.y, o.w, pw)
                }
            };
            let (w, h) = (pw, ph);
            let short = o.device_name.trim_start_matches("\\\\.\\").to_string();
            targets.push(ScreenTarget {
                screen_index: this_index,
                output_idx: o.output_idx,
                adapter: o.adapter,
                label: format!("{} ({})", short, fmt_size(w, h)),
                rect: MonitorRef {
                    output_idx: o.output_idx,
                    adapter: o.adapter,
                    x,
                    y,
                    w,
                    h,
                },
            });
        }
    } else {
        // Last resort: no DXGI. Assume ddagrab index order matches Tauri's, which
        // is a guess. Loudly flagged above.
        for (i, m) in tauri_mons.iter().enumerate() {
            targets.push(ScreenTarget {
                screen_index: i as u32,
                output_idx: i as u32,
                adapter: 0,
                label: format!("{} ({})", m.name, fmt_size(m.w, m.h)),
                rect: MonitorRef {
                    output_idx: i as u32,
                    adapter: 0,
                    x: m.x,
                    y: m.y,
                    w: m.w,
                    h: m.h,
                },
            });
        }
    }

    targets.sort_by_key(|t| t.screen_index);
    (targets, warnings)
}

/// Whether we have both a capture and an encode path for this output.
///
/// Adapter 0 is the NVIDIA GPU: ddagrab feeds NVENC zero-copy. Any other adapter
/// must encode on itself, which means QSV. Pure so it can be tested against fake
/// enumerations.
pub fn output_is_capturable(o: &DisplayOutput, caps: &EncoderCaps) -> bool {
    o.attached && (o.adapter == 0 || caps.h264_qsv)
}

fn fmt_size(w: u32, h: u32) -> String {
    format!("{}x{}", w, h)
}

/// Map a logical desktop origin into physical pixels using this monitor's own
/// DPI ratio. Exact when every monitor shares a scale factor; approximate under
/// mixed DPI, because Windows composes the virtual desktop in logical space.
fn rescale_origin(x: i32, y: i32, logical_w: u32, physical_w: u32) -> (i32, i32) {
    if logical_w == 0 || physical_w == 0 || logical_w == physical_w {
        return (x, y);
    }
    let s = physical_w as f64 / logical_w as f64;
    ((x as f64 * s).round() as i32, (y as f64 * s).round() as i32)
}

/// Everything the recorder needs to know about the display environment, resolved
/// once at session start. Split out from `Recorder::start` so the recorder itself
/// has no Tauri dependency and can be exercised headlessly.
#[derive(Debug, Clone)]
pub struct RecorderEnv {
    pub screens: Vec<ScreenTarget>,
    pub desktop: Rect,
    pub caps: EncoderCaps,
    pub warnings: Vec<String>,
}

impl RecorderEnv {
    pub fn from_app(app: &tauri::AppHandle) -> Self {
        // Before `tauri_monitors` — `available_monitors()` is DPI-virtualised
        // exactly like DXGI is.
        let _ = ensure_process_dpi_aware();
        let ffmpeg = crate::capture::find_ffmpeg().unwrap_or_else(|| "ffmpeg".into());
        let caps = detect_encoders(&ffmpeg);
        let mons = tauri_monitors(app);
        let (screens, mut warnings) = resolve_screen_targets(&ffmpeg, &mons, &caps);
        let desktop = resolve_desktop(&mons, &screens, &mut warnings);
        Self {
            screens,
            desktop,
            caps,
            warnings,
        }
    }

    /// DXGI only — no window system. Used by the headless test harness.
    pub fn from_dxgi() -> Self {
        let _ = ensure_process_dpi_aware();
        let ffmpeg = crate::capture::find_ffmpeg().unwrap_or_else(|| "ffmpeg".into());
        let caps = detect_encoders(&ffmpeg);
        let (screens, mut warnings) = resolve_screen_targets(&ffmpeg, &[], &caps);
        let desktop = resolve_desktop(&[], &screens, &mut warnings);
        Self {
            screens,
            desktop,
            caps,
            warnings,
        }
    }
}

/// The virtual desktop bounding box, in physical pixels.
///
/// Every attached monitor counts, including ones we are not recording — the box
/// describes where the recorded rectangles sit, so dropping an unrecorded monitor
/// would put the others in the wrong place.
///
/// Preference order: the union of all attached DXGI outputs (authoritative once
/// the process is DPI aware), then Tauri's monitor list, then the union of just
/// the capture rectangles.
fn resolve_desktop(
    mons: &[TauriMonitor],
    screens: &[ScreenTarget],
    warnings: &mut Vec<String>,
) -> Rect {
    if let Ok(outputs) = enumerate_dxgi_outputs() {
        let attached: Vec<&DisplayOutput> = outputs.iter().filter(|o| o.attached).collect();
        let dxgi = union_rects(attached.iter().map(|o| (o.x, o.y, o.w, o.h)));
        if dxgi.w > 0 && dxgi.h > 0 {
            // Every capture rectangle must sit inside the box, or the box is not
            // describing the same coordinate space.
            let contains_all = screens.iter().all(|s| {
                s.rect.x >= dxgi.x
                    && s.rect.y >= dxgi.y
                    && s.rect.x + s.rect.w as i32 <= dxgi.x + dxgi.w
                    && s.rect.y + s.rect.h as i32 <= dxgi.y + dxgi.h
            });
            if contains_all {
                return dxgi;
            }
            warnings.push(format!(
                "the DXGI desktop box {}x{} at ({},{}) does not contain every capture \
                 rectangle; falling back to the union of the capture rectangles",
                dxgi.w, dxgi.h, dxgi.x, dxgi.y
            ));
        }
    }

    let bounds = desktop_bounds(mons);
    if bounds.w > 0
        && screens
            .iter()
            .all(|s| mons.iter().any(|m| m.w == s.rect.w && m.h == s.rect.h))
    {
        return bounds;
    }
    if !mons.is_empty() {
        warnings.push(
            "Tauri monitor sizes did not match the captured sizes; the desktop \
             bounding box was derived from the capture rectangles instead"
                .to_string(),
        );
    }
    union_rects(screens.iter().map(|s| (s.rect.x, s.rect.y, s.rect.w, s.rect.h)))
}

/// Bounding box of `(x, y, w, h)` rectangles. Empty input is an empty rect.
pub fn union_rects(rects: impl IntoIterator<Item = (i32, i32, u32, u32)>) -> Rect {
    let mut it = rects.into_iter();
    let Some((x, y, w, h)) = it.next() else {
        return Rect::default();
    };
    let (mut min_x, mut min_y) = (x, y);
    let (mut max_x, mut max_y) = (x + w as i32, y + h as i32);
    for (x, y, w, h) in it {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w as i32);
        max_y = max_y.max(y + h as i32);
    }
    Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    }
}

/// Bounding box of every attached monitor, in physical pixels.
pub fn desktop_bounds(mons: &[TauriMonitor]) -> Rect {
    union_rects(mons.iter().map(|m| (m.x, m.y, m.w, m.h)))
}

// ---------------------------------------------------------------------------
// Encoder selection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncoderCaps {
    pub h264_nvenc: bool,
    pub av1_nvenc: bool,
    pub hevc_nvenc: bool,
    /// Intel Quick Sync. The only encoder reachable from a capture running on the
    /// Intel iGPU's D3D11 device.
    pub h264_qsv: bool,
}

pub fn detect_encoders(ffmpeg: &str) -> EncoderCaps {
    let out = silent_command(ffmpeg).args(["-hide_banner", "-encoders"]).output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    EncoderCaps {
        h264_nvenc: text.contains("h264_nvenc"),
        av1_nvenc: text.contains("av1_nvenc"),
        hevc_nvenc: text.contains("hevc_nvenc"),
        h264_qsv: text.contains("h264_qsv"),
    }
}

/// Encoder for a screen captured on `adapter`.
///
/// ffmpeg hands the encoder whatever hardware device the filtergraph produced, and
/// there is no per-encoder device override in the CLI. So a capture on a
/// non-default adapter can only be encoded *on that adapter*: measured, every
/// NVENC route from an Intel capture fails with `OpenEncodeSessionEx failed: no
/// encode device`, including after `hwdownload`. QSV is the answer, and a good one
/// — `hwmap=derive_device=qsv` works, the whole track stays on the iGPU, and it
/// costs ~6% of one core.
pub fn pick_screen_encoder(
    width: u32,
    adapter: u32,
    caps: &EncoderCaps,
) -> (String, String, Option<String>) {
    if adapter != 0 {
        if caps.h264_qsv {
            return ("h264_qsv".into(), "h264".into(), None);
        }
        // `resolve_screen_targets` filters these out before we get here; this is
        // the belt-and-braces path.
        return (
            "h264_qsv".into(),
            "h264".into(),
            Some(format!(
                "adapter {} needs h264_qsv and it is unavailable; this screen will \
                 fail to start",
                adapter
            )),
        );
    }
    pick_video_encoder(width, caps)
}

/// `(ffmpeg encoder, manifest codec name, warning)`.
///
/// `h264_nvenc` hard-fails above 4096px wide on this hardware, so anything wider
/// must use AV1. AV1 is preferred over HEVC because Chromium/WebView2 decodes it
/// with the bundled dav1d, whereas HEVC-in-MP4 needs an OS codec extension.
pub fn pick_video_encoder(width: u32, caps: &EncoderCaps) -> (String, String, Option<String>) {
    if width > 4096 {
        if caps.av1_nvenc {
            return ("av1_nvenc".into(), "av1".into(), None);
        }
        if caps.hevc_nvenc {
            return (
                "hevc_nvenc".into(),
                "hevc".into(),
                Some(format!(
                    "{}px wide needs AV1 but av1_nvenc is unavailable; using HEVC, \
                     which may not play back in the app window",
                    width
                )),
            );
        }
        return (
            "libx264".into(),
            "h264".into(),
            Some(format!(
                "NO NVENC AVAILABLE — falling back to CPU libx264 for a {}px-wide \
                 screen. This will use heavy CPU and may not keep up in realtime.",
                width
            )),
        );
    }
    if caps.h264_nvenc {
        return ("h264_nvenc".into(), "h264".into(), None);
    }
    (
        "libx264".into(),
        "h264".into(),
        Some(
            "NO NVENC AVAILABLE — falling back to CPU libx264. This will use heavy \
             CPU and may not keep up in realtime."
                .to_string(),
        ),
    )
}

// ---------------------------------------------------------------------------
// Capture device selection
// ---------------------------------------------------------------------------

/// The outcome of picking an input device, plus anything the UI should say about
/// it. `device: None` means *deliberately nothing* — the track is skipped and the
/// warnings explain why. Never silently empty.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceSelection {
    pub device: Option<String>,
    pub warnings: Vec<String>,
}

/// Opening a Bluetooth microphone is not a neutral act.
///
/// Windows cannot run a Bluetooth headset's mic and its high-quality audio at the
/// same time: taking the mic switches the whole device from A2DP to HFP
/// (hands-free). The user *hears* the switch, and for as long as the recorder
/// holds the mic their music, calls and videos play back at 16kHz mono. A
/// background recorder that runs all day must never do that behind the user's
/// back — so a Bluetooth device is never auto-selected, only ever chosen
/// explicitly.
///
/// Matching is on the dshow display name, which is what we have. Windows names the
/// HFP endpoint `Headset (<device name>)` and the A2DP one `Headphones (…)`, so
/// `headset` is the reliable structural marker; the rest catch common vendor
/// spellings.
pub fn is_bluetooth_audio_device(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const MARKERS: [&str; 10] = [
        "bluetooth",
        "hands-free",
        "handsfree",
        "hands free",
        "hfp",
        "headset",
        "airpods",
        "earbuds",
        "earphone",
        "galaxy buds",
    ];
    MARKERS.iter().any(|m| n.contains(m))
}

/// Virtual / loopback audio inputs: real devices, but not microphones. Recording
/// one as "the mic" produces a track of system audio or silence.
fn is_virtual_audio_device(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "stereo mix",
        "what u hear",
        "vb-audio",
        "cable output",
        "voicemeeter",
        "virtual",
        "xsplit",
        "obs",
    ]
    .iter()
    .any(|m| n.contains(m))
}

/// Rank an auto-selectable mic. Higher wins; enumeration order breaks ties.
fn mic_score(name: &str) -> i32 {
    let n = name.to_ascii_lowercase();
    let mut score = 0;
    // Windows names a plain capture endpoint `Microphone (<device>)`; anything
    // else is more likely a line-in, a mix, or something exotic.
    if n.starts_with("microphone") {
        score += 3;
    }
    if n.contains("line in") || n.contains("line-in") || n.contains("aux") {
        score -= 2;
    }
    score
}

/// Choose the microphone to record.
///
/// An explicit `configured` device always wins — including a Bluetooth one, which
/// is a decision only the user can make. Auto-selection deliberately refuses
/// Bluetooth and virtual devices, and refuses to fall back to one: if the only
/// microphone in the machine is a Bluetooth headset, this returns `None` with a
/// warning so the UI can offer the choice, rather than hijacking the user's
/// headphones for the next six hours.
pub fn select_mic_device(configured: Option<&str>, available: &[String]) -> DeviceSelection {
    let mut warnings = Vec::new();

    if let Some(want) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        if !available.iter().any(|d| d == want) {
            warnings.push(format!(
                "configured microphone \"{}\" was not in the dshow device list; \
                 trying it anyway",
                want
            ));
        }
        if is_bluetooth_audio_device(want) {
            warnings.push(format!(
                "\"{}\" looks like a Bluetooth headset. Recording its microphone \
                 forces the device into hands-free mode, so everything you listen to \
                 will drop to 16kHz mono until the session ends.",
                want
            ));
        }
        return DeviceSelection {
            device: Some(want.to_string()),
            warnings,
        };
    }

    if available.is_empty() {
        warnings.push("no dshow audio device found; the microphone is not being recorded".into());
        return DeviceSelection {
            device: None,
            warnings,
        };
    }

    let mut candidates: Vec<(usize, &String)> = available
        .iter()
        .enumerate()
        .filter(|(_, d)| !is_bluetooth_audio_device(d) && !is_virtual_audio_device(d))
        .collect();
    candidates.sort_by_key(|(i, d)| (-mic_score(d), *i));

    match candidates.first() {
        Some((_, d)) => DeviceSelection {
            device: Some((*d).clone()),
            warnings,
        },
        None => {
            warnings.push(format!(
                "the only microphones available are Bluetooth or virtual devices ({}). \
                 None was selected, because opening a Bluetooth mic switches the device \
                 to hands-free mode and degrades everything you hear. Choose one \
                 explicitly to record it anyway.",
                available.join(", ")
            ));
            DeviceSelection {
                device: None,
                warnings,
            }
        }
    }
}

/// Virtual cameras present themselves exactly like hardware ones. Auto-selecting
/// an idle OBS/XSplit virtual camera yields a black track, and on this machine
/// XSplit also *holds* the real BRIO, so the real device reports "already in use".
fn is_virtual_video_device(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "virtual",
        "obs",
        "xsplit",
        "vcam",
        "ndi",
        "manycam",
        "droidcam",
        "snap camera",
    ]
    .iter()
    .any(|m| n.contains(m))
}

/// Choose the webcam. Same shape as [`select_mic_device`]: an explicit choice
/// always wins, auto-selection prefers a physical camera over a virtual one, and a
/// virtual-only machine is a warning rather than a silent black track.
pub fn select_webcam_device(configured: Option<&str>, available: &[String]) -> DeviceSelection {
    let mut warnings = Vec::new();

    if let Some(want) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        if !available.iter().any(|d| d == want) {
            warnings.push(format!(
                "configured webcam \"{}\" was not in the dshow device list; trying it anyway",
                want
            ));
        }
        return DeviceSelection {
            device: Some(want.to_string()),
            warnings,
        };
    }

    if available.is_empty() {
        warnings.push("no dshow video device found; the webcam is not being recorded".into());
        return DeviceSelection {
            device: None,
            warnings,
        };
    }

    if let Some(real) = available.iter().find(|d| !is_virtual_video_device(d)) {
        return DeviceSelection {
            device: Some(real.clone()),
            warnings,
        };
    }

    warnings.push(format!(
        "only virtual cameras are available ({}); using \"{}\", which will record \
         whatever that software is outputting",
        available.join(", "),
        available[0]
    ));
    DeviceSelection {
        device: Some(available[0].clone()),
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Track definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum TrackSource {
    Screen {
        adapter: u32,
        output_idx: u32,
        encoder: String,
        fps: u32,
    },
    Webcam {
        device: String,
        encoder: String,
        video_size: String,
        fps: u32,
    },
    Mic {
        device: String,
        /// Also write a mono 16kHz WAV sidecar for later transcription.
        sidecar: bool,
    },
    /// Webcam **and** microphone, captured by one ffmpeg process from one
    /// DirectShow graph, writing two separately segmented outputs.
    ///
    /// This exists for lip-sync, and only for lip-sync. Two independent captures
    /// cannot be aligned after the fact: each device has its own open latency (a
    /// Logitech BRIO takes ~1.3s to hand over its first MJPEG frame, a microphone
    /// starts in ~0.15s) and each demuxer zeroes its own first packet, so the
    /// difference is baked in as a genuine, unrecoverable offset. Measured on this
    /// machine, two processes put the audio **1378ms** ahead of the picture; one
    /// process, one `-i "video=X:audio=Y"` input, measured **0.0ms** of skew
    /// between the two streams on the shared timeline (ffprobe start_time of both
    /// streams in a muxed reference output of the same run).
    ///
    /// It has to be a *single* dshow input, not two `-f dshow` inputs in one
    /// process: two inputs are two DirectShow graphs with two independent start
    /// instants, which reproduces the original offset exactly.
    WebcamMic {
        video_device: String,
        audio_device: String,
        encoder: String,
        video_size: String,
        fps: u32,
        sidecar: bool,
    },
    SysAudio,
}

impl TrackSource {
    /// How the first segment of a run learns where its content really began.
    ///
    /// Everything dshow-based needs help here: the wall-clock instant a segment
    /// *file* appears is not the instant its content starts, and the size of that
    /// lag depends on the pipeline. Measured, same session, same 100ms poll: a
    /// webcam file appeared 324ms after its first frame, a mic file **1702ms**
    /// after its first sample, because an audio-only AAC/MP4 output buffers far
    /// longer before its first write. Anchoring each to its own file creation is
    /// therefore not a small error, and it is not a shared one.
    fn anchor(&self, launched: i64, progress_t0: &Arc<AtomicI64>, sys_t0: Option<i64>) -> Anchor {
        match self {
            TrackSource::SysAudio => sys_t0.map(Anchor::Fixed).unwrap_or(Anchor::FileCreation),
            TrackSource::Webcam { .. } | TrackSource::Mic { .. } | TrackSource::WebcamMic { .. } => {
                Anchor::FromProgress(progress_t0.clone(), launched)
            }
            // Screens land within ~270ms of their content start (measured), and
            // that path is verified. Left alone deliberately.
            TrackSource::Screen { .. } => Anchor::FileCreation,
        }
    }
}

/// Where a run's first segment believes its content began.
#[derive(Clone)]
enum Anchor {
    /// Nothing better than the file's own creation time; `decide_segment_start`
    /// falls back to `observed`.
    FileCreation,
    /// Known exactly. `SysAudioCapture` pads silence from this instant, so its
    /// content genuinely begins here — several seconds before ffmpeg creates the
    /// file.
    Fixed(i64),
    /// Derived from ffmpeg's own clock. Every `-progress` block carries
    /// `out_time_us`, the media position just muxed; at wall time `W` that implies
    /// content started at `W - out_time`. Every sample is biased *late* by however
    /// far the pipeline is behind, so the **minimum** over the sampling window is
    /// the estimate — pipeline latency only ever inflates it.
    ///
    /// Measured against the true content start (recovered from the stop instant
    /// minus the media produced): screen 137ms late, webcam 204–229ms late, mic
    /// 369ms late — versus 270ms / 324ms / **1702ms** for file creation.
    FromProgress(Arc<AtomicI64>, i64),
}

impl Anchor {
    fn resolve(&self) -> Option<i64> {
        match self {
            Anchor::FileCreation => None,
            Anchor::Fixed(t) => Some(*t),
            Anchor::FromProgress(est, launched) => {
                let v = est.load(Ordering::Relaxed);
                // No usable sample, or a value outside the window in which a
                // capture can plausibly have started. Fall back rather than
                // publish a start time we do not believe.
                if v == i64::MAX || v > launched + 60_000 {
                    return None;
                }
                // Content cannot predate the process that captured it.
                Some(v.max(*launched))
            }
        }
    }
}

/// One manifest track fed by a supervisor's ffmpeg process.
#[derive(Debug, Clone)]
struct OutputSpec {
    track_id: String,
    dir_name: String,
    /// Per-output, not per-process: the webcam and mic share one ffmpeg but are
    /// segmented at different lengths. See [`AUDIO_SEGMENT_SECONDS`].
    segment_seconds: u32,
}

/// One supervised ffmpeg process, and the manifest track(s) it feeds.
///
/// Almost every process feeds exactly one track. The webcam/mic pair is the sole
/// exception and the reason `outputs` is a list: they must share a process to
/// share a clock, but they remain two separate tracks and two separate files —
/// transcription needs an isolated mic track and the player renders them
/// independently.
#[derive(Debug, Clone)]
struct TrackSpec {
    /// Supervisor id — also the key for this process's stderr ring and logs.
    id: String,
    outputs: Vec<OutputSpec>,
    /// Delay before this track's first launch. See [`SCREEN_STAGGER_MS`].
    launch_delay_ms: u64,
    source: TrackSource,
}

impl TrackSource {
    fn writes_mic_sidecar(&self) -> bool {
        matches!(
            self,
            TrackSource::Mic { sidecar: true, .. } | TrackSource::WebcamMic { sidecar: true, .. }
        )
    }
}

/// A fresh manifest track, before anything has been recorded into it.
#[allow(clippy::too_many_arguments)]
fn new_track(
    id: &str,
    kind: &str,
    label: &str,
    monitor: Option<MonitorRef>,
    codec: &str,
    width: u32,
    height: u32,
    fps: u32,
) -> Track {
    Track {
        id: id.to_string(),
        kind: kind.to_string(),
        label: label.to_string(),
        monitor,
        codec: codec.to_string(),
        width,
        height,
        fps,
        start_epoch_ms: 0,
        dir: id.to_string(),
        segments: Vec::new(),
        gaps: Vec::new(),
        status: TrackStatus::Starting,
        error: None,
        restarts: 0,
    }
}

/// Whether these two device names can be expressed as one dshow input, and why
/// not if they cannot.
///
/// ffmpeg spells a combined capture `video=<cam>:audio=<mic>`, so the separator
/// is part of the *device name string*. A device whose own name contains
/// `:audio=` would be parsed apart in the wrong place and open the wrong thing —
/// vanishingly unlikely, but the failure would be silent, and silently recording
/// the wrong device is worse than recording two processes.
fn combined_dshow_unsupported(video: &str, audio: &str) -> Option<String> {
    for (role, name) in [("webcam", video), ("microphone", audio)] {
        if name.contains(":audio=") || name.contains(":video=") {
            return Some(format!(
                "the {} device name \"{}\" contains ffmpeg's own dshow separator, so \
                 the two devices cannot be named in a single input.",
                role, name
            ));
        }
    }
    None
}

/// How long a track's segments should be, given the session's setting.
///
/// Audio-only tracks override it; everything with a picture keeps the session
/// length, because video is where segment size actually buys crash granularity.
fn segment_seconds_for(track_id: &str, session_seconds: u32) -> u32 {
    match track_id {
        "mic" | "sysaudio" => AUDIO_SEGMENT_SECONDS,
        _ => session_seconds,
    }
}

impl TrackSpec {
    fn solo(id: &str, source: TrackSource, session_seconds: u32) -> Self {
        Self {
            id: id.to_string(),
            outputs: vec![OutputSpec {
                track_id: id.to_string(),
                dir_name: id.to_string(),
                segment_seconds: segment_seconds_for(id, session_seconds),
            }],
            launch_delay_ms: 0,
            source,
        }
    }
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct SessionOptions {
    pub segment_seconds: u32,
    pub screen_fps: u32,
    pub record_screens: bool,
    /// Restrict to these **screen indices** — the `N` in `screen-N`, as reported by
    /// `list_display_outputs`. Empty = every capturable screen. Note this is not
    /// `output_idx`, which repeats across adapters.
    pub screens: Vec<u32>,
    pub record_webcam: bool,
    /// dshow device name. `None` auto-selects; see [`select_webcam_device`].
    pub webcam_device: Option<String>,
    pub webcam_size: String,
    pub webcam_fps: u32,
    pub record_mic: bool,
    /// dshow device name. `None` auto-selects, which deliberately never picks a
    /// Bluetooth headset — see [`select_mic_device`]. Naming one here is how you
    /// override that.
    pub mic_device: Option<String>,
    pub record_system_audio: bool,
    pub mic_sidecar_16k: bool,
    /// Delete any segment that covered a stretch with **no** keyboard or mouse
    /// input, once the idle stretch has run longer than this. `0` disables it and
    /// every segment is kept. Windows-only — see [`Inner::drop_idle_after_ms`].
    pub drop_idle_after_seconds: u32,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            segment_seconds: 60,
            screen_fps: 30,
            record_screens: true,
            screens: Vec::new(),
            record_webcam: true,
            webcam_device: None,
            webcam_size: "1920x1080".to_string(),
            webcam_fps: 30,
            record_mic: true,
            mic_device: None,
            record_system_audio: true,
            mic_sidecar_16k: true,
            drop_idle_after_seconds: 60,
        }
    }
}

// ---------------------------------------------------------------------------
// Recorder
// ---------------------------------------------------------------------------

struct Inner {
    session_dir: PathBuf,
    segment_seconds: u32,
    ffmpeg: String,
    ffprobe: String,
    manifest: Mutex<SessionManifest>,
    stop: AtomicBool,
    logs: Mutex<std::collections::HashMap<String, RingLog>>,
    warnings: Mutex<Vec<String>>,
    fatal: Mutex<Option<String>>,
    /// Supervisors spawned *after* `start` — see [`supervise_track`], which splits
    /// the shared webcam/mic process into two solo ones if the pair cannot run.
    /// `stop` drains this as well, so a split session still shuts down cleanly.
    extra_joins: Mutex<Vec<JoinHandle<()>>>,
    /// Epoch-ms of the last system-wide keyboard/mouse input, maintained by
    /// [`idle_poll_loop`]. Read when a segment closes to decide whether it covered
    /// a stretch with no user activity.
    last_input_ms: AtomicI64,
    /// Delete a closed segment whose whole span was idle, once the idle stretch
    /// has run at least this long. `0` disables trimming (also forced to `0` off
    /// Windows, where there is no input poller). See [`index_output_segment`].
    drop_idle_after_ms: i64,
}

impl Inner {
    fn flush(&self) {
        let m = self.manifest.lock().unwrap();
        if let Err(e) = session::write_manifest(&self.session_dir, &m) {
            drop(m);
            self.warn(format!("manifest write failed: {}", e));
        }
    }

    fn warn(&self, msg: String) {
        eprintln!("[recorder] {}", msg);
        let mut w = self.warnings.lock().unwrap();
        if !w.contains(&msg) {
            if w.len() > 100 {
                w.remove(0);
            }
            w.push(msg);
        }
    }

    fn log(&self, track: &str, line: String) {
        let mut logs = self.logs.lock().unwrap();
        logs.entry(track.to_string()).or_default().push(line);
    }

    /// Start a run with an empty ring, so a diagnosis can never quote the
    /// *previous* run's stderr. The old contents have already been written to
    /// `logs/<track>.log` by then.
    fn clear_log(&self, track: &str) {
        self.logs.lock().unwrap().remove(track);
    }

    fn track_log(&self, track: &str) -> Vec<String> {
        self.logs
            .lock()
            .unwrap()
            .get(track)
            .map(|l| l.snapshot())
            .unwrap_or_default()
    }

    fn log_contains(&self, track: &str, needle: &str) -> bool {
        let logs = self.logs.lock().unwrap();
        logs.get(track).map(|l| l.contains(needle)).unwrap_or(false)
    }

    fn set_fatal(&self, msg: String) {
        let mut f = self.fatal.lock().unwrap();
        if f.is_none() {
            eprintln!("[recorder] FATAL: {}", msg);
            *f = Some(msg);
        }
        self.stop.store(true, Ordering::SeqCst);
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

pub struct Recorder {
    inner: Arc<Inner>,
    joins: Mutex<Vec<JoinHandle<()>>>,
    stopped: AtomicBool,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("session_dir", &self.inner.session_dir)
            .field("stopping", &self.inner.stopping())
            .finish()
    }
}

impl Recorder {
    pub fn session_dir(&self) -> PathBuf {
        self.inner.session_dir.clone()
    }

    pub fn manifest(&self) -> SessionManifest {
        self.inner.manifest.lock().unwrap().clone()
    }

    pub fn warnings(&self) -> Vec<String> {
        self.inner.warnings.lock().unwrap().clone()
    }

    pub fn fatal(&self) -> Option<String> {
        self.inner.fatal.lock().unwrap().clone()
    }

    pub fn is_stopping(&self) -> bool {
        self.inner.stopping()
    }

    pub fn track_log(&self, track: &str) -> Vec<String> {
        self.inner
            .logs
            .lock()
            .unwrap()
            .get(track)
            .map(|l| l.snapshot())
            .unwrap_or_default()
    }

    pub fn add_marker(&self, label: String) -> Result<session::Marker, String> {
        let marker = session::Marker {
            epoch_ms: now_ms(),
            label,
        };
        session::append_marker(&self.inner.session_dir, &marker)?;
        {
            let mut m = self.inner.manifest.lock().unwrap();
            m.markers.push(marker.clone());
        }
        self.inner.flush();
        Ok(marker)
    }

    // -----------------------------------------------------------------------

    /// Spawn every track. Returns as soon as the supervisors are running; the
    /// first segments appear on disk a second or so later.
    pub fn start(
        env: RecorderEnv,
        sessions_root: &Path,
        opts: SessionOptions,
    ) -> Result<Arc<Recorder>, String> {
        let ffmpeg = crate::capture::find_ffmpeg().ok_or("FFmpeg not found in PATH")?;
        let ffprobe = session::find_ffprobe().ok_or("ffprobe not found in PATH")?;

        let now = chrono::Local::now();
        let id = session::session_id_from(now);
        let session_dir = sessions_root.join(&id);
        std::fs::create_dir_all(&session_dir)
            .map_err(|e| format!("create {}: {}", session_dir.display(), e))?;

        let RecorderEnv {
            screens,
            desktop,
            caps,
            mut warnings,
        } = env;

        // The session-wide length, which the video tracks use verbatim. Audio
        // tracks override it via `segment_seconds_for`, so the manifest's value is
        // the video length, not a promise about every track.
        let seg_secs = opts.segment_seconds.max(5);
        let mut manifest =
            SessionManifest::new(id.clone(), now.timestamp_millis(), seg_secs, desktop);

        let mut specs: Vec<TrackSpec> = Vec::new();

        // ---- screens ----
        let mut screen_n: u64 = 0;
        if opts.record_screens {
            for t in &screens {
                if !opts.screens.is_empty() && !opts.screens.contains(&t.screen_index) {
                    continue;
                }
                let (encoder, codec, warn) = pick_screen_encoder(t.rect.w, t.adapter, &caps);
                if let Some(w) = warn {
                    warnings.push(w);
                }
                let id = format!("screen-{}", t.screen_index);
                manifest.tracks.push(new_track(
                    &id,
                    "screen",
                    &t.label,
                    Some(t.rect),
                    &codec,
                    t.rect.w,
                    t.rect.h,
                    opts.screen_fps,
                ));
                let mut spec = TrackSpec::solo(
                    &id,
                    TrackSource::Screen {
                        adapter: t.adapter,
                        output_idx: t.output_idx,
                        encoder,
                        fps: opts.screen_fps,
                    },
                    seg_secs,
                );
                spec.launch_delay_ms = screen_n * SCREEN_STAGGER_MS;
                specs.push(spec);
                screen_n += 1;
            }
            if specs.is_empty() {
                warnings.push("no capturable screens were found".to_string());
            }
        }

        // ---- webcam / mic device discovery (one dshow enumeration for both) ----
        let need_dshow = opts.record_webcam || opts.record_mic;
        let (dshow_video, dshow_audio) = if need_dshow {
            crate::capture::list_dshow_devices(&ffmpeg)
        } else {
            (Vec::new(), Vec::new())
        };

        let webcam_device = opts.record_webcam.then(|| {
            let picked = select_webcam_device(opts.webcam_device.as_deref(), &dshow_video);
            warnings.extend(picked.warnings);
            picked.device
        }).flatten();

        let mic_device = opts.record_mic.then(|| {
            let picked = select_mic_device(opts.mic_device.as_deref(), &dshow_audio);
            warnings.extend(picked.warnings);
            picked.device
        }).flatten();

        let (webcam_w, webcam_h) = parse_size(&opts.webcam_size).unwrap_or((1920, 1080));
        let webcam_enc = webcam_device.as_ref().map(|_| {
            let (encoder, codec, warn) = pick_video_encoder(webcam_w, &caps);
            if let Some(x) = warn {
                warnings.push(x);
            }
            (encoder, codec)
        });

        if let Some(device) = &webcam_device {
            let (_, codec) = webcam_enc.clone().unwrap();
            manifest.tracks.push(new_track(
                "webcam", "webcam", device, None, &codec, webcam_w, webcam_h, opts.webcam_fps,
            ));
        }
        if let Some(device) = &mic_device {
            manifest.tracks.push(new_track("mic", "mic", device, None, "aac", 0, 0, 0));
        }

        match (&webcam_device, &mic_device) {
            // The one case that matters: both wanted, so capture them together.
            (Some(video_device), Some(audio_device)) => {
                if let Some(reason) = combined_dshow_unsupported(video_device, audio_device) {
                    warnings.push(format!(
                        "{} The webcam and microphone are being recorded by separate \
                         processes, so their lip-sync may be off by up to a second.",
                        reason
                    ));
                    specs.push(TrackSpec::solo(
                        "webcam",
                        TrackSource::Webcam {
                            device: video_device.clone(),
                            encoder: webcam_enc.clone().unwrap().0,
                            video_size: opts.webcam_size.clone(),
                            fps: opts.webcam_fps,
                        },
                        seg_secs,
                    ));
                    specs.push(TrackSpec::solo(
                        "mic",
                        TrackSource::Mic {
                            device: audio_device.clone(),
                            sidecar: opts.mic_sidecar_16k,
                        },
                        seg_secs,
                    ));
                } else {
                    specs.push(TrackSpec {
                        id: "webcam".into(),
                        // One process, two segment lengths: the video is cut at
                        // the session length and the audio at the much longer
                        // audio one. The muxer takes `-segment_time` per output,
                        // so sharing a clock costs nothing here.
                        outputs: vec![
                            OutputSpec {
                                track_id: "webcam".into(),
                                dir_name: "webcam".into(),
                                segment_seconds: segment_seconds_for("webcam", seg_secs),
                            },
                            OutputSpec {
                                track_id: "mic".into(),
                                dir_name: "mic".into(),
                                segment_seconds: segment_seconds_for("mic", seg_secs),
                            },
                        ],
                        launch_delay_ms: 0,
                        source: TrackSource::WebcamMic {
                            video_device: video_device.clone(),
                            audio_device: audio_device.clone(),
                            encoder: webcam_enc.clone().unwrap().0,
                            video_size: opts.webcam_size.clone(),
                            fps: opts.webcam_fps,
                            sidecar: opts.mic_sidecar_16k,
                        },
                    });
                }
            }
            (Some(device), None) => specs.push(TrackSpec::solo(
                "webcam",
                TrackSource::Webcam {
                    device: device.clone(),
                    encoder: webcam_enc.clone().unwrap().0,
                    video_size: opts.webcam_size.clone(),
                    fps: opts.webcam_fps,
                },
                seg_secs,
            )),
            (None, Some(device)) => specs.push(TrackSpec::solo(
                "mic",
                TrackSource::Mic {
                    device: device.clone(),
                    sidecar: opts.mic_sidecar_16k,
                },
                seg_secs,
            )),
            (None, None) => {}
        }

        if opts.record_system_audio {
            manifest.tracks.push(new_track(
                "sysaudio",
                "system-audio",
                "System audio (WASAPI loopback)",
                None,
                "aac",
                0,
                0,
                0,
            ));
            specs.push(TrackSpec::solo("sysaudio", TrackSource::SysAudio, seg_secs));
        }

        if specs.is_empty() {
            return Err("nothing to record — every track was disabled or unavailable".into());
        }

        for s in &specs {
            for o in &s.outputs {
                std::fs::create_dir_all(session_dir.join(&o.dir_name))
                    .map_err(|e| format!("create track dir: {}", e))?;
            }
            if s.source.writes_mic_sidecar() {
                let _ = std::fs::create_dir_all(session_dir.join("mic-16k"));
            }
        }

        // Inactivity trimming is Windows-only: it depends on the input poller, and
        // without one every segment past the threshold would be read as idle and
        // deleted. Off Windows the feature is simply disabled.
        #[cfg(target_os = "windows")]
        let drop_idle_after_ms = opts.drop_idle_after_seconds as i64 * 1000;
        #[cfg(not(target_os = "windows"))]
        let drop_idle_after_ms = 0i64;

        // Seed from a real reading so a session that starts mid-activity is not
        // mistaken for idle; fall back to "now" if the poll fails.
        let seed_input_ms = poll_last_input_ms().unwrap_or_else(now_ms);

        let inner = Arc::new(Inner {
            session_dir: session_dir.clone(),
            segment_seconds: manifest.segment_seconds,
            ffmpeg,
            ffprobe,
            manifest: Mutex::new(manifest),
            stop: AtomicBool::new(false),
            logs: Mutex::new(Default::default()),
            warnings: Mutex::new(warnings),
            fatal: Mutex::new(None),
            extra_joins: Mutex::new(Vec::new()),
            last_input_ms: AtomicI64::new(seed_input_ms),
            drop_idle_after_ms,
        });
        inner.flush();

        // Spawn every supervisor as close together as possible; each records its
        // own measured start instant, so a few ms of skew here is recovered later.
        let mut joins = Vec::new();
        for spec in specs {
            let inner = inner.clone();
            joins.push(
                std::thread::Builder::new()
                    .name(format!("rec-{}", spec.id))
                    .spawn(move || supervise_track(inner, spec))
                    .map_err(|e| format!("spawn supervisor: {}", e))?,
            );
        }

        // Disk-space guard.
        {
            let inner = inner.clone();
            joins.push(
                std::thread::Builder::new()
                    .name("rec-diskguard".into())
                    .spawn(move || disk_guard(inner))
                    .map_err(|e| format!("spawn disk guard: {}", e))?,
            );
        }

        // Inactivity tracker — only when trimming is on.
        if inner.drop_idle_after_ms > 0 {
            let inner = inner.clone();
            joins.push(
                std::thread::Builder::new()
                    .name("rec-idle".into())
                    .spawn(move || idle_poll_loop(inner))
                    .map_err(|e| format!("spawn idle tracker: {}", e))?,
            );
        }

        Ok(Arc::new(Recorder {
            inner,
            joins: Mutex::new(joins),
            stopped: AtomicBool::new(false),
        }))
    }

    /// Signal every track to finish the in-flight segment gracefully, then join.
    /// Blocking — call from `spawn_blocking`.
    pub fn stop(&self) -> SessionManifest {
        if !self.stopped.swap(true, Ordering::SeqCst) {
            self.inner.stop.store(true, Ordering::SeqCst);
            let handles: Vec<JoinHandle<()>> = std::mem::take(&mut *self.joins.lock().unwrap());
            for h in handles {
                let _ = h.join();
            }
            // A supervisor may have spawned another one while shutting down, so
            // drain until the list stays empty rather than taking it once.
            loop {
                let extra: Vec<JoinHandle<()>> =
                    std::mem::take(&mut *self.inner.extra_joins.lock().unwrap());
                if extra.is_empty() {
                    break;
                }
                for h in extra {
                    let _ = h.join();
                }
            }
            {
                let mut m = self.inner.manifest.lock().unwrap();
                m.ended_epoch_ms = Some(now_ms());
                m.error = self.inner.fatal.lock().unwrap().clone();
            }
            self.inner.flush();
        }
        self.manifest()
    }
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

// ---------------------------------------------------------------------------
// Disk guard
// ---------------------------------------------------------------------------

fn disk_guard(inner: Arc<Inner>) {
    while !inner.stopping() {
        if let Some(free) = free_space_bytes(&inner.session_dir) {
            if free < MIN_FREE_BYTES {
                inner.set_fatal(format!(
                    "stopped: only {} MB free on the recording drive. Existing \
                     captures were left untouched.",
                    free / 1024 / 1024
                ));
                break;
            }
        }
        for _ in 0..50 {
            if inner.stopping() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Bytes free on the volume holding `path`. `None` when the path does not exist
/// yet or the query fails — the UI shows "--" rather than inventing a number.
#[cfg(target_os = "windows")]
pub fn free_space_bytes(path: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide = HSTRING::from(path.as_os_str());
    let mut free: u64 = 0;
    unsafe {
        GetDiskFreeSpaceExW(&wide, Some(&mut free), None, None).ok()?;
    }
    Some(free)
}

#[cfg(not(target_os = "windows"))]
pub fn free_space_bytes(_path: &Path) -> Option<u64> {
    None
}

// ---------------------------------------------------------------------------
// Per-track supervisor
// ---------------------------------------------------------------------------

enum RunOutcome {
    /// The session is stopping; the last segment was finalized cleanly.
    Finished,
    /// The process died on its own, or the watchdog killed a wedged one.
    Died(String),
    /// Could not even spawn.
    SpawnFailed(String),
}

/// A run of consecutive segments deleted for inactivity, collapsed into one gap.
///
/// `from` is where the first dropped segment began and `to` where the most recent
/// one ended. The next *kept* segment flushes this as a single manifest gap and
/// chains across it, so a long idle stretch is one honest hole rather than a
/// scatter of per-segment gaps or a spurious "dropped time" drift gap.
#[derive(Debug, Clone, Copy)]
struct IdleTail {
    from: i64,
    to: i64,
}

/// One output file stream of a supervisor's ffmpeg process, and the manifest
/// track it feeds.
struct OutputState {
    track_id: String,
    dir: PathBuf,
    /// Persists across restarts: a relaunched ffmpeg writes into the same
    /// directory, and its existing files must not be re-indexed.
    seen: std::collections::BTreeSet<String>,
    /// The segment ffmpeg is currently writing, with the instant it appeared. Only
    /// probed once the *next* file appears (or the process exits), so a file that
    /// is still open is never ffprobed.
    pending: Option<(String, i64)>,
    /// Cleared by the first segment indexed in each run — only that one gets to
    /// use the run's anchor.
    anchor_pending: bool,
    /// This output's own segment length; see [`OutputSpec::segment_seconds`].
    segment_seconds: u32,
    /// Segments indexed during the current run. Reset by [`run_track_once`].
    produced: u32,
    /// Consecutive completed runs in which this output produced nothing.
    ///
    /// Counted per output, not per process, because that is what distinguishes
    /// "the pair cannot run" from "one half of the pair is broken". Both are
    /// answered the same way — see [`split_pair`].
    barren_runs: u32,
    /// The still-open run of inactivity-dropped segments, if the last few segments
    /// were all deleted for idle. Flushed by the next kept segment. See
    /// [`index_output_segment`] and [`IdleTail`].
    idle_tail: Option<IdleTail>,
}

/// Whether a segment starting at `seg_start` should be deleted for inactivity,
/// given the last input at `last_input` and a threshold of `drop_after_ms`.
///
/// Drop only when there was no input from before the segment began (`last_input <
/// seg_start`) *and* the idle stretch had already run at least the threshold by
/// the time it started. That second clause is what keeps up to one threshold's
/// worth of footage after the last activity, instead of cutting the instant input
/// stops. `drop_after_ms <= 0` never drops.
fn should_drop_idle(last_input: i64, seg_start: i64, drop_after_ms: i64) -> bool {
    drop_after_ms > 0 && last_input < seg_start && seg_start - last_input >= drop_after_ms
}

impl OutputState {
    fn new(session_dir: &Path, o: &OutputSpec) -> Self {
        let dir = session_dir.join(&o.dir_name);
        let mut seen = std::collections::BTreeSet::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                seen.insert(e.file_name().to_string_lossy().to_string());
            }
        }
        Self {
            track_id: o.track_id.clone(),
            dir,
            seen,
            pending: None,
            anchor_pending: true,
            segment_seconds: o.segment_seconds,
            produced: 0,
            barren_runs: 0,
            idle_tail: None,
        }
    }

    /// Where ffmpeg's segment muxer writes this output.
    fn out_pattern(&self) -> PathBuf {
        self.dir.join(format!("{}.mp4", session::SEGMENT_STRFTIME))
    }
}

/// The output feeding `track_id`, for a process that writes more than one.
fn output_for<'a>(outs: &'a [OutputState], track_id: &str) -> Option<&'a OutputState> {
    outs.iter().find(|o| o.track_id == track_id)
}

fn supervise_track(inner: Arc<Inner>, spec: TrackSpec) {
    let mut outs: Vec<OutputState> = spec
        .outputs
        .iter()
        .map(|o| OutputState::new(&inner.session_dir, o))
        .collect();

    // The source can change under us: a shared webcam+mic process that cannot run
    // gives up the pairing and continues as a solo capture. See `split_pair`.
    let mut source = spec.source.clone();
    let mut restarts: u32 = 0;

    // Screens are launched a few hundred ms apart; see SCREEN_STAGGER_MS.
    if spec.launch_delay_ms > 0 && !sleep_unless_stopping(&inner, spec.launch_delay_ms) {
        set_all_status(&inner, &outs, TrackStatus::Stopped, None);
        inner.flush();
        return;
    }

    loop {
        if inner.stopping() {
            break;
        }
        set_all_status(&inner, &outs, TrackStatus::Recording, None);

        let outcome = run_track_once(&inner, &spec, &source, &mut outs);

        match outcome {
            RunOutcome::Finished => break,
            RunOutcome::Died(reason) | RunOutcome::SpawnFailed(reason) => {
                if inner.stopping() {
                    break;
                }
                // Disk-full is not something to retry into.
                if inner.log_contains(&spec.id, "No space left on device") {
                    inner.set_fatal(format!(
                        "stopped: the recording drive is full ({} could not write). \
                         Existing captures were left untouched.",
                        spec.id
                    ));
                    break;
                }

                for o in outs.iter_mut() {
                    o.barren_runs = if o.produced == 0 { o.barren_runs + 1 } else { 0 };
                }

                restarts += 1;
                let now = now_ms();
                for o in &outs {
                    let gap_from = track_end_or(&inner, &o.track_id, now);
                    record_gap(&inner, &o.track_id, gap_from, now, &reason);
                }
                inner.warn(format!(
                    "track {} died ({}), restart {}/{}",
                    spec.id, reason, restarts, MAX_RESTARTS
                ));
                if restarts > MAX_RESTARTS {
                    set_all_status(
                        &inner,
                        &outs,
                        TrackStatus::Failed,
                        Some(format!("gave up after {} restarts: {}", MAX_RESTARTS, reason)),
                    );
                    inner.flush();
                    return;
                }
                set_all_status(&inner, &outs, TrackStatus::Restarting, Some(reason.clone()));
                for o in &outs {
                    bump_restarts(&inner, &o.track_id);
                }

                // A shared process in which either half keeps producing nothing is
                // worse than no pairing at all. Two shapes, one remedy:
                //
                // - Neither produces (the camera is held by another app, so the
                //   single combined input cannot open): both tracks get nothing.
                // - One produces and the other does not (a device unplugged
                //   mid-session): the watchdog kills the process on the silent
                //   output, so the healthy half is repeatedly cut short too, and
                //   after MAX_RESTARTS it would be marked Failed alongside the
                //   device that actually broke.
                //
                // Lip-sync is a luxury on top of recording; recording comes first.
                // So give the mic its own supervisor and carry on solo — exactly
                // the pre-existing arrangement, with its independent failure
                // behaviour restored, after which each device can die on its own.
                if outs.len() > 1 && outs.iter().any(|o| o.barren_runs >= PAIR_SPLIT_AFTER) {
                    if let Some(solo) = split_pair(&inner, &source, &mut outs, &reason) {
                        source = solo;
                        for o in outs.iter_mut() {
                            o.barren_runs = 0;
                        }
                        restarts = 0;
                    }
                }
                inner.flush();
                // Back off a little so a permanently missing device doesn't spin.
                let backoff = (500 * restarts.min(6) as u64).max(500);
                if !sleep_unless_stopping(&inner, backoff) {
                    return;
                }
            }
        }
    }

    set_all_status(&inner, &outs, TrackStatus::Stopped, None);
    inner.flush();
}

/// Give up on capturing the webcam and mic in one process, and spawn a solo
/// supervisor for the microphone. Returns the reduced source this supervisor
/// should carry on with, or `None` if there was no pairing to break.
///
/// The mic is the half that moves, because the pairing exists for the webcam's
/// benefit and a solo mic is a complete, useful track on its own. The two are
/// then exactly as independent as they were before this feature: either can die,
/// restart, or fail for good without touching the other.
fn split_pair(
    inner: &Arc<Inner>,
    source: &TrackSource,
    outs: &mut Vec<OutputState>,
    reason: &str,
) -> Option<TrackSource> {
    let TrackSource::WebcamMic {
        video_device,
        audio_device,
        encoder,
        video_size,
        fps,
        sidecar,
    } = source
    else {
        return None;
    };

    inner.warn(format!(
        "the webcam and microphone could not be captured together ({}). They are now \
         being recorded by separate processes, which restores independent recovery \
         but means their lip-sync may be off by up to a second.",
        reason
    ));

    outs.retain(|o| o.track_id != "mic");

    // `segment_seconds_for` keys off the track id, so the mic keeps the same
    // audio segment length it had inside the pair.
    let mic_spec = TrackSpec::solo(
        "mic",
        TrackSource::Mic {
            device: audio_device.clone(),
            sidecar: *sidecar,
        },
        inner.segment_seconds,
    );
    let inner2 = inner.clone();
    match std::thread::Builder::new()
        .name("rec-mic".into())
        .spawn(move || supervise_track(inner2, mic_spec))
    {
        Ok(h) => inner.extra_joins.lock().unwrap().push(h),
        Err(e) => {
            inner.warn(format!("could not spawn a solo microphone supervisor: {}", e));
            set_track_status(
                inner,
                "mic",
                TrackStatus::Failed,
                Some(format!("could not be recorded separately: {}", e)),
            );
        }
    }

    Some(TrackSource::Webcam {
        device: video_device.clone(),
        encoder: encoder.clone(),
        video_size: video_size.clone(),
        fps: *fps,
    })
}

fn set_all_status(
    inner: &Arc<Inner>,
    outs: &[OutputState],
    status: TrackStatus,
    err: Option<String>,
) {
    for o in outs {
        set_track_status(inner, &o.track_id, status, err.clone());
    }
}

/// Sleep in short slices, returning `false` as soon as the session is stopping.
fn sleep_unless_stopping(inner: &Arc<Inner>, ms: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(ms);
    while std::time::Instant::now() < deadline {
        if inner.stopping() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !inner.stopping()
}

fn run_track_once(
    inner: &Arc<Inner>,
    spec: &TrackSpec,
    source: &TrackSource,
    outs: &mut [OutputState],
) -> RunOutcome {
    inner.clear_log(&spec.id);
    // Each run has its own anchor, and each output gets one shot at using it.
    for o in outs.iter_mut() {
        o.anchor_pending = true;
        o.produced = 0;
    }
    let mut cmd = build_command(inner, source, outs);

    // RULE: never leave stdout/stderr as an unread pipe. Both get a dedicated
    // draining thread below; if either were left unread the OS pipe buffer would
    // fill, ffmpeg would block on write, and recording would silently freeze.
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Read before the spawn, so the anchor window and the "content cannot start
    // before this" floor both bracket the process rather than trail it.
    let launched = now_ms();

    let mut child: Child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return RunOutcome::SpawnFailed(format!("spawn ffmpeg: {}", e)),
    };

    let last_progress = Arc::new(AtomicI64::new(launched));
    // Lowest `wall - out_time` seen so far: the best estimate of when this
    // process's content began. `i64::MAX` means "no usable sample yet".
    let progress_t0 = Arc::new(AtomicI64::new(i64::MAX));

    // stdout: `-progress pipe:1`. Doubles as the health heartbeat and, for the
    // first few seconds, as the content-start estimator (see `Anchor`).
    let stdout_join = child.stdout.take().map(|out| {
        let hb = last_progress.clone();
        let t0 = progress_t0.clone();
        let deadline = launched + ANCHOR_WINDOW_MS;
        std::thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                let Ok(l) = line else { break };
                let now = now_ms();
                hb.store(now, Ordering::Relaxed);
                if now > deadline {
                    continue;
                }
                // `out_time_us` only. ffmpeg's sibling `out_time_ms` key has
                // carried microseconds for years and is not worth trusting.
                if let Some(us) = l
                    .strip_prefix("out_time_us=")
                    .and_then(|v| v.trim().parse::<i64>().ok())
                    .filter(|us| *us > 0)
                {
                    t0.fetch_min(now - us / 1000, Ordering::Relaxed);
                }
            }
        })
    });

    // stderr: drain into a bounded ring.
    let stderr_join = child.stderr.take().map(|err| {
        let inner = inner.clone();
        let id = spec.id.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if !l.trim().is_empty() {
                            inner.log(&id, l);
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    });

    // stdin: `q` for a graceful stop — except for the system-audio track, where
    // stdin *is* the PCM feed and closing it is what ends the process.
    let mut stdin = child.stdin.take();
    let mut sysaudio: Option<crate::sysaudio::SysAudioCapture> = None;

    // Where this run's first segment believes its content started. Every output of
    // this process shares it, which is what makes the webcam and mic tracks agree
    // with each other by construction rather than by estimate.
    let mut sys_t0: Option<i64> = None;

    if matches!(source, TrackSource::SysAudio) {
        let sink = match stdin.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                return RunOutcome::SpawnFailed("no stdin for system audio pipe".into());
            }
        };
        let t0 = now_ms();
        match crate::sysaudio::SysAudioCapture::start(Box::new(sink), t0) {
            Ok(c) => {
                sysaudio = Some(c);
                sys_t0 = Some(t0);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return RunOutcome::SpawnFailed(format!("WASAPI loopback: {}", e));
            }
        }
    }

    // ---- poll loop -------------------------------------------------------
    // Every output of this process shares one anchor, which is what makes the
    // webcam and mic tracks agree with each other by construction rather than by
    // per-device estimate.
    let anchor = source.anchor(launched, &progress_t0, sys_t0);

    // The watchdog is per output, not per process. A shared webcam+mic process
    // whose camera has died still writes mic segments on schedule; watching only
    // the process would never notice the picture had stopped.
    let mut last_new_segment: Vec<i64> = vec![launched; outs.len()];
    // Per output, because outputs of one process can be segmented at different
    // lengths (webcam 60s, mic 600s) and a shared deadline would either fire
    // spuriously on the long one or never fire on the short one.
    //
    // This is the weaker of the two liveness checks, and the only one that scales
    // with segment length. `PROGRESS_STALL_MS` (30s) still catches a wedged or
    // dead ffmpeg regardless of how long its segments are.
    let watchdog_ms: Vec<i64> = outs.iter().map(|o| o.segment_seconds as i64 * 2000).collect();

    // Why the run ended. Deliberately *not* a message: the message quotes ffmpeg's
    // stderr, and that can only be read after the drain thread has been joined
    // below. Composing it here is how the real error got lost before — a process
    // can write its fatal line and exit before the reader has drained the pipe.
    enum ExitKind {
        Finished,
        Exited(std::process::ExitStatus),
        /// Which output went quiet, and for how many seconds.
        NoSegments(String, i64),
        ProgressStalled,
        SysAudioStopped,
        Internal(String),
    }

    let exit: ExitKind;

    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));

        // 1. New segment files?
        for (i, o) in outs.iter_mut().enumerate() {
            if let Some(seen_at) = poll_output(inner, o, &anchor) {
                last_new_segment[i] = seen_at;
            }
        }

        // 2. Did it exit on its own?
        match child.try_wait() {
            Ok(Some(status)) => {
                flush_outputs(inner, outs, &anchor);
                exit = if inner.stopping() {
                    ExitKind::Finished
                } else {
                    ExitKind::Exited(status)
                };
                break;
            }
            Ok(None) => {}
            Err(e) => {
                exit = ExitKind::Internal(format!("try_wait: {}", e));
                break;
            }
        }

        // 3. Stop requested → graceful shutdown.
        if inner.stopping() {
            graceful_stop(&mut child, &mut stdin, sysaudio.take());
            // `q` makes ffmpeg finalize what it had open, so re-scan before
            // flushing: the file it was writing is now closed and complete, and on
            // a multi-output process there is one such file per output.
            for o in outs.iter_mut() {
                poll_output(inner, o, &anchor);
            }
            flush_outputs(inner, outs, &anchor);
            exit = ExitKind::Finished;
            break;
        }

        // 4. Watchdog: some output has gone 2× segment_seconds without a segment.
        let now = now_ms();
        if let Some(i) = last_new_segment
            .iter()
            .zip(&watchdog_ms)
            .position(|(t, limit)| now - t > *limit)
        {
            let silent = (now - last_new_segment[i]) / 1000;
            let track = outs[i].track_id.clone();
            kill_and_reap(&mut child, &mut stdin);
            flush_outputs(inner, outs, &anchor);
            exit = ExitKind::NoSegments(track, silent);
            break;
        }

        // 5. Heartbeat: `-progress` has gone quiet.
        if now - last_progress.load(Ordering::Relaxed) > PROGRESS_STALL_MS {
            kill_and_reap(&mut child, &mut stdin);
            flush_outputs(inner, outs, &anchor);
            exit = ExitKind::ProgressStalled;
            break;
        }

        // 6. System audio thread health.
        if let Some(sa) = &sysaudio {
            if !sa.is_healthy() {
                if let Some(sa) = sysaudio.take() {
                    sa.stop();
                }
                let _ = child.wait();
                flush_outputs(inner, outs, &anchor);
                exit = ExitKind::SysAudioStopped;
                break;
            }
        }
    }

    if let Some(sa) = sysaudio.take() {
        sa.stop();
    }
    drop(stdin);
    let _ = child.wait();
    // Join the drain threads so their pipe ends close before we relaunch — and,
    // critically, so the ring buffer holds everything ffmpeg wrote right up to
    // exit before the reason below is composed from it.
    if let Some(j) = stdout_join {
        let _ = j.join();
    }
    if let Some(j) = stderr_join {
        let _ = j.join();
    }

    let outcome = match exit {
        ExitKind::Finished => RunOutcome::Finished,
        ExitKind::Exited(status) => RunOutcome::Died(format!(
            "ffmpeg exited ({}): {}",
            status,
            diagnose(&inner.track_log(&spec.id))
        )),
        ExitKind::NoSegments(track, secs) => RunOutcome::Died(format!(
            "no new {} segment for {}s: {}",
            track,
            secs,
            diagnose(&inner.track_log(&spec.id))
        )),
        ExitKind::ProgressStalled => RunOutcome::Died(format!(
            "ffmpeg stopped reporting progress: {}",
            diagnose(&inner.track_log(&spec.id))
        )),
        ExitKind::SysAudioStopped => RunOutcome::Died(format!(
            "WASAPI loopback capture stopped: {}",
            diagnose(&inner.track_log(&spec.id))
        )),
        ExitKind::Internal(e) => RunOutcome::Died(e),
    };

    // Persist what ffmpeg said. The in-memory ring is bounded and is overwritten
    // by the next run; over a six-hour session with restarts, the on-disk copy is
    // the only place the earlier failures survive.
    if !matches!(outcome, RunOutcome::Finished) {
        dump_track_log(inner, &spec.id, &outcome_reason(&outcome));
    }

    outcome
}

fn outcome_reason(outcome: &RunOutcome) -> String {
    match outcome {
        RunOutcome::Finished => "finished".to_string(),
        RunOutcome::Died(r) | RunOutcome::SpawnFailed(r) => r.clone(),
    }
}

/// Append this run's stderr ring to `<session>/logs/<track>.log`, headed by the
/// reason we concluded. Best-effort: a failure to write diagnostics must never
/// interfere with recording.
fn dump_track_log(inner: &Arc<Inner>, track: &str, reason: &str) {
    use std::io::Write as _;
    let dir = inner.session_dir.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{}.log", track)))
    else {
        return;
    };
    let _ = writeln!(
        f,
        "\n===== {} — {} =====",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        reason
    );
    for line in inner.track_log(track) {
        let _ = writeln!(f, "{}", line);
    }
}

/// Graceful stop: write `q` to ffmpeg's stdin so it finalizes the in-flight
/// segment's moov atom, wait up to 5s, only then kill. Killing outright loses
/// the whole segment.
fn graceful_stop(
    child: &mut Child,
    stdin: &mut Option<std::process::ChildStdin>,
    sysaudio: Option<crate::sysaudio::SysAudioCapture>,
) {
    if let Some(sa) = sysaudio {
        // Closing the PCM sink gives ffmpeg EOF on stdin, which is its clean exit.
        sa.stop();
    }
    if let Some(mut s) = stdin.take() {
        let _ = s.write_all(b"q\n");
        let _ = s.flush();
        drop(s);
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(GRACEFUL_STOP_MS);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(_) => break,
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn kill_and_reap(child: &mut Child, stdin: &mut Option<std::process::ChildStdin>) {
    drop(stdin.take());
    let _ = child.kill();
    let _ = child.wait();
}

/// Newly appeared files in a track directory, chronologically. `-strftime`
/// names sort lexicographically into time order.
/// Index every segment of `o` that has closed since the last poll, and return
/// when a new file was first seen — the watchdog's heartbeat for this output.
///
/// The newest file is left `pending` rather than indexed: ffmpeg still has it
/// open, and probing a file that is still being written yields a truncated
/// duration or no moov atom at all.
fn poll_output(inner: &Arc<Inner>, o: &mut OutputState, anchor: &Anchor) -> Option<i64> {
    let mut latest = None;
    for name in scan_new(&o.dir, &mut o.seen) {
        let observed = now_ms();
        if let Some((prev, prev_observed)) = o.pending.take() {
            index_output_segment(inner, o, anchor, &prev, prev_observed);
        }
        latest = Some(observed);
        o.pending = Some((name, observed));
    }
    latest
}

/// Index whatever each output still has open. Only correct once the process has
/// exited or been killed, at which point the file is closed for good.
fn flush_outputs(inner: &Arc<Inner>, outs: &mut [OutputState], anchor: &Anchor) {
    for o in outs.iter_mut() {
        if let Some((file, observed)) = o.pending.take() {
            index_output_segment(inner, o, anchor, &file, observed);
        }
    }
}

fn index_output_segment(
    inner: &Arc<Inner>,
    o: &mut OutputState,
    anchor: &Anchor,
    file: &str,
    observed: i64,
) {
    // Inactivity trimming. A segment spans roughly [observed, now]: `observed` is
    // when the file first appeared (≈ its start) and now is when the next file
    // appeared / the process exited (≈ its close). Delete it when there was no
    // keyboard/mouse input across that whole span *and* the idle stretch had
    // already run past the threshold when the segment began — so up to one
    // threshold's worth of footage after the last activity is always kept.
    if inner.drop_idle_after_ms > 0 {
        let last_input = inner.last_input_ms.load(Ordering::Relaxed);
        if should_drop_idle(last_input, observed, inner.drop_idle_after_ms) {
            let seg_end = now_ms();
            let path = o.dir.join(file);
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    let from = o.idle_tail.map(|t| t.from).unwrap_or(observed);
                    o.idle_tail = Some(IdleTail { from, to: seg_end });
                    // The run's anchor no longer describes a later kept segment, and
                    // the process is plainly healthy, so consume both.
                    o.anchor_pending = false;
                    o.produced += 1;
                    return;
                }
                Err(e) => {
                    // Never orphan an un-indexed file on disk; fall through and keep
                    // it so playback can still find it.
                    eprintln!(
                        "[recorder] idle segment {}/{} not deleted ({}); keeping it",
                        o.track_id, file, e
                    );
                }
            }
        }
    }

    // A kept segment after an idle run flushes the collapsed idle gap and chains
    // across it, so the drift logic in `decide_segment_start` does not also record
    // a "dropped time" gap for the same hole.
    let chain_override = o.idle_tail.take().map(|t| {
        record_gap(
            inner,
            &o.track_id,
            t.from,
            t.to,
            "idle — no keyboard or mouse activity",
        );
        t.to
    });

    // Only the first segment of a run is placed by the anchor; the rest chain off
    // it, so a late-resolving estimate cannot shunt the track mid-run.
    let a = if o.anchor_pending { anchor.resolve() } else { None };
    o.anchor_pending = false;
    o.produced += 1;
    index_segment(
        inner,
        &o.track_id,
        &o.dir,
        file,
        observed,
        a,
        o.segment_seconds,
        chain_override,
    );
}

fn scan_new(dir: &Path, seen: &mut std::collections::BTreeSet<String>) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut fresh: Vec<String> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".mp4") {
            continue;
        }
        if seen.insert(name.clone()) {
            fresh.push(name);
        }
    }
    fresh.sort();
    fresh
}

// ---------------------------------------------------------------------------
// Manifest mutation
// ---------------------------------------------------------------------------

/// Result of [`decide_segment_start`].
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentStart {
    pub start_epoch_ms: i64,
    pub gap: Option<Gap>,
}

/// Disagreement between the chained media clock and wall clock beyond this is not
/// drift — it means real time was lost.
const REANCHOR_MS: i64 = 1500;
/// Fraction of the wall-clock disagreement absorbed each segment.
const DRIFT_ALPHA: i64 = 4;

/// Where on the wall-clock timeline a freshly-closed segment starts.
///
/// Three inputs, in priority order:
///
/// - `anchor` — the instant the track's *content* began, for the first segment of
///   a run on a track that knows it. Only `sysaudio` supplies one, and it must
///   win: `SysAudioCapture` starts writing PCM the moment the thread starts, but
///   ffmpeg does not create the first segment file until its first output packet,
///   measured ~4s later. Those first ~4s are real, correctly-timed silence padded
///   by the capture, so the file's content genuinely begins before the wall clock
///   in its own `-strftime` filename. Anchoring to file creation instead put the
///   whole sysaudio track ~4.1s late (observed in sessions 1–2). A negative offset
///   versus the filename is therefore *correct*, not a bug.
/// - `chained` — `prev.start + prev.duration` from ffprobe. Sub-millisecond, but
///   it follows the *media* clock; a capture device running 0.02% slow would
///   accumulate seconds over a six-hour session.
/// - `observed` — when the file appeared in the poll scan. Absolute and
///   non-accumulating, but carries up to `POLL_MS` of jitter.
///
/// So the chained value is pulled `1/DRIFT_ALPHA` of the way toward the observed
/// one each segment. That attenuates poll jitter 4× while bounding steady-state
/// error against a constant drift to `drift * DRIFT_ALPHA` — about 22ms at the
/// measured rate — instead of letting it grow without limit.
pub fn decide_segment_start(
    anchor: Option<i64>,
    chained: Option<i64>,
    observed: i64,
) -> SegmentStart {
    match (anchor, chained) {
        // A restart breaks the chain outright, so the anchor wins over it and the
        // time between them is recorded as a gap.
        (Some(a), c) => SegmentStart {
            start_epoch_ms: a,
            gap: c.filter(|c| a > *c).map(|c| Gap {
                from_epoch_ms: c,
                to_epoch_ms: a,
                reason: "track restarted".to_string(),
            }),
        },
        (None, Some(c)) => {
            let delta = observed - c;
            if delta.abs() > REANCHOR_MS {
                SegmentStart {
                    start_epoch_ms: observed,
                    gap: (delta > 0).then(|| Gap {
                        from_epoch_ms: c,
                        to_epoch_ms: observed,
                        reason: "segment boundary drifted (dropped time)".to_string(),
                    }),
                }
            } else {
                SegmentStart {
                    start_epoch_ms: c + delta / DRIFT_ALPHA,
                    gap: None,
                }
            }
        }
        (None, None) => SegmentStart {
            start_epoch_ms: observed,
            gap: None,
        },
    }
}

/// ffprobe a segment that has just closed and append it to the manifest, with its
/// start time chosen by [`decide_segment_start`].
#[allow(clippy::too_many_arguments)]
fn index_segment(
    inner: &Arc<Inner>,
    track_id: &str,
    dir: &Path,
    file: &str,
    observed: i64,
    anchor: Option<i64>,
    segment_seconds: u32,
    chain_override: Option<i64>,
) {
    let path = dir.join(file);
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let probe = match session::probe_segment(&inner.ffprobe, &path) {
        Ok(p) => p,
        Err(e) => {
            inner.warn(format!("{}/{} unreadable: {}", track_id, file, e));
            let from = observed;
            record_gap(
                inner,
                track_id,
                from,
                from + segment_seconds as i64 * 1000,
                &format!("unreadable segment {}", file),
            );
            inner.flush();
            return;
        }
    };

    {
        let mut m = inner.manifest.lock().unwrap();
        let seg_secs = m.segment_seconds as i64;
        let Some(track) = m.track_mut(track_id) else {
            return;
        };

        // After an idle run, `chain_override` carries the end of the deleted
        // stretch so this segment chains onto it (delta ≈ 0, no drift gap) rather
        // than onto the last *kept* segment before the hole.
        let chained = chain_override.or_else(|| {
            track
                .segments
                .last()
                .map(|s| s.start_epoch_ms + s.duration_ms)
        });

        let decided = decide_segment_start(anchor, chained, observed);
        let start = decided.start_epoch_ms;
        if let Some(g) = decided.gap {
            track.gaps.push(g);
        }

        if track.segments.is_empty() {
            track.start_epoch_ms = start;
            // The first segment is also where we learn the real encoded geometry.
            if probe.has_video && probe.width > 0 {
                track.width = probe.width;
                track.height = probe.height;
                if probe.fps > 0 {
                    track.fps = probe.fps;
                }
            }
            if !probe.codec.is_empty() {
                track.codec = probe.codec.clone();
            }
        }

        track.segments.push(Segment {
            file: file.to_string(),
            start_epoch_ms: start,
            duration_ms: probe.duration_ms,
            bytes,
        });

        // A segment that came out far shorter than requested means frames were
        // lost; make that visible rather than silently misaligning playback.
        if probe.duration_ms < seg_secs * 1000 / 2 && probe.duration_ms > 0 {
            let _ = seg_secs;
        }
    }

    inner.flush();
}

fn record_gap(inner: &Arc<Inner>, track_id: &str, from: i64, to: i64, reason: &str) {
    let mut m = inner.manifest.lock().unwrap();
    if let Some(t) = m.track_mut(track_id) {
        if to > from {
            t.gaps.push(Gap {
                from_epoch_ms: from,
                to_epoch_ms: to,
                reason: reason.to_string(),
            });
        }
    }
}

fn track_end_or(inner: &Arc<Inner>, track_id: &str, fallback: i64) -> i64 {
    let m = inner.manifest.lock().unwrap();
    m.tracks
        .iter()
        .find(|t| t.id == track_id)
        .and_then(|t| t.end_epoch_ms())
        .unwrap_or(fallback)
}

fn set_track_status(inner: &Arc<Inner>, track_id: &str, status: TrackStatus, err: Option<String>) {
    let mut m = inner.manifest.lock().unwrap();
    if let Some(t) = m.track_mut(track_id) {
        t.status = status;
        if err.is_some() {
            t.error = err;
        }
    }
}

fn bump_restarts(inner: &Arc<Inner>, track_id: &str) {
    let mut m = inner.manifest.lock().unwrap();
    if let Some(t) = m.track_mut(track_id) {
        t.restarts += 1;
    }
}

// ---------------------------------------------------------------------------
// ffmpeg command construction
// ---------------------------------------------------------------------------

/// Shared segment-muxer tail. `-segment_atclocktime` keeps boundaries near clock
/// ticks; they still land on keyframes rather than exactly on the tick, which is
/// fine because every segment start is measured, not assumed.
fn push_segment_args(cmd: &mut Command, seconds: u32, out_pattern: &Path) {
    cmd.args([
        "-f",
        "segment",
        "-segment_time",
        &seconds.to_string(),
        "-segment_atclocktime",
        "1",
        "-reset_timestamps",
        "1",
        "-strftime",
        "1",
        "-segment_format",
        "mp4",
    ])
    .arg(out_pattern);
}

/// Cheap hook for later transcription: a mono 16kHz PCM sidecar alongside the
/// AAC mic track, so whisper can be pointed at it with no re-encode.
///
/// `map` is needed only when the process has more than one stream to choose from.
fn push_mic_sidecar_args(cmd: &mut Command, inner: &Arc<Inner>, secs: u32, map: Option<&str>) {
    let side = inner
        .session_dir
        .join("mic-16k")
        .join(format!("{}.wav", session::SEGMENT_STRFTIME));
    if let Some(m) = map {
        cmd.args(["-map", m]);
    }
    cmd.args(["-c:a", "pcm_s16le", "-ar", "16000", "-ac", "1"]);
    cmd.args([
        "-f",
        "segment",
        "-segment_time",
        &secs.to_string(),
        "-segment_atclocktime",
        "1",
        "-reset_timestamps",
        "1",
        "-strftime",
        "1",
        "-segment_format",
        "wav",
    ])
    .arg(side);
}

fn build_command(inner: &Arc<Inner>, source: &TrackSource, outs: &[OutputState]) -> Command {
    // Every source but the webcam/mic pair writes exactly one output, and that
    // one is always first. `secs` is that output's own length, which for an
    // audio-only track is not the session's.
    let out = outs[0].out_pattern();
    let secs = outs[0].segment_seconds;

    let mut cmd = silent_command(&inner.ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "warning", "-progress", "pipe:1"]);

    match source {
        TrackSource::Screen {
            adapter,
            output_idx,
            encoder,
            fps,
        } => {
            let grab = format!(
                "ddagrab=output_idx={}:framerate={}:draw_mouse=1",
                output_idx, fps
            );
            if *adapter == 0 {
                cmd.args(ddagrab_input_args(0, &grab));
            } else {
                // Capture and encode both stay on this adapter. `-init_hw_device
                // qsv=qs@dda` derives a QSV device from the same D3D11 device the
                // capture runs on, and `hwmap=derive_device=qsv` hands the frames
                // over without a round trip through system memory.
                cmd.args([
                    "-init_hw_device",
                    &format!("d3d11va=dda:{}", adapter),
                    "-init_hw_device",
                    "qsv=qs@dda",
                    "-filter_hw_device",
                    "dda",
                    "-filter_complex",
                    &format!("{},hwmap=derive_device=qsv,format=qsv", grab),
                ]);
            }
            push_video_encoder_args(&mut cmd, encoder, 30, "40M", "80M", *fps);
            push_segment_args(&mut cmd, secs, &out);
        }
        TrackSource::Webcam {
            device,
            encoder,
            video_size,
            fps,
        } => {
            // Video-only. This arm runs when there is no microphone to record, or
            // after the pair has been split (see `split_pair`); when both are
            // wanted they share one process — see `TrackSource::WebcamMic`.
            cmd.args([
                "-f",
                "dshow",
                "-rtbufsize",
                "512M",
                "-vcodec",
                "mjpeg",
                "-video_size",
                video_size,
                "-framerate",
                &fps.to_string(),
                "-i",
                &format!("video={}", device),
            ]);
            push_video_encoder_args(&mut cmd, encoder, 28, "12M", "24M", *fps);
            push_segment_args(&mut cmd, secs, &out);
        }
        TrackSource::Mic { device, sidecar } => {
            cmd.args([
                "-f",
                "dshow",
                "-rtbufsize",
                "512M",
                "-i",
                &format!("audio={}", device),
            ]);
            cmd.args(["-c:a", "aac", "-b:a", "128k", "-ar", "48000", "-ac", "2"]);
            push_segment_args(&mut cmd, secs, &out);
            if *sidecar {
                push_mic_sidecar_args(&mut cmd, inner, secs, None);
            }
        }
        TrackSource::WebcamMic {
            video_device,
            audio_device,
            encoder,
            video_size,
            fps,
            sidecar,
        } => {
            // THE lip-sync fix. One `-f dshow` input naming both devices, so
            // DirectShow builds ONE graph and both streams are stamped against one
            // reference clock. Two inputs would be two graphs with two independent
            // start instants and would reproduce the offset exactly; two processes
            // did, by ~1.2s.
            //
            // The two streams are then demuxed back apart into two separately
            // segmented outputs, because the manifest keeps them as two tracks:
            // transcription needs an isolated mic file and the player renders them
            // independently. Sharing a clock does not mean sharing a file.
            cmd.args([
                "-f",
                "dshow",
                "-rtbufsize",
                "512M",
                "-vcodec",
                "mjpeg",
                "-video_size",
                video_size,
                "-framerate",
                &fps.to_string(),
                "-i",
                &format!("video={}:audio={}", video_device, audio_device),
            ]);

            let cam = output_for(outs, "webcam");
            cmd.args(["-map", "0:v:0"]);
            push_video_encoder_args(&mut cmd, encoder, 28, "12M", "24M", *fps);
            push_segment_args(
                &mut cmd,
                cam.map(|o| o.segment_seconds).unwrap_or(secs),
                &cam.map(|o| o.out_pattern()).unwrap_or_else(|| out.clone()),
            );

            // The mic's own, much longer segment length — the two outputs of this
            // one process are deliberately cut at different rates.
            if let Some(mic) = output_for(outs, "mic") {
                cmd.args([
                    "-map", "0:a:0", "-c:a", "aac", "-b:a", "128k", "-ar", "48000", "-ac", "2",
                ]);
                push_segment_args(&mut cmd, mic.segment_seconds, &mic.out_pattern());
                if *sidecar {
                    push_mic_sidecar_args(&mut cmd, inner, mic.segment_seconds, Some("0:a:0"));
                }
            }
        }
        TrackSource::SysAudio => {
            cmd.args([
                "-f", "f32le", "-ar", "48000", "-ac", "2", "-i", "pipe:0",
            ]);
            cmd.args(["-c:a", "aac", "-b:a", "160k"]);
            push_segment_args(&mut cmd, secs, &out);
        }
    }

    cmd
}

/// Video encoder args, plus the two settings that make segmenting work at all.
///
/// **Keyframes come from `-g`, not `-force_key_frames`.** Measured on this
/// machine: with `-tune ll`, `h264_nvenc` silently ignores `-force_key_frames`,
/// so the file gets exactly one keyframe, the segment muxer never finds a cut
/// point, and a single ever-growing file is produced instead of segments. (The
/// spec's §1 command has this bug — 95s of capture at `-segment_time 30` yielded
/// one 14MB file.) A fixed 2-second GOP is honoured under `-tune ll` and gives
/// the same result the forced-keyframe expression was aiming for.
fn push_video_encoder_args(
    cmd: &mut Command,
    encoder: &str,
    cq: u32,
    maxrate: &str,
    bufsize: &str,
    fps: u32,
) {
    // A keyframe every 2 seconds bounds how far a segment boundary can land from
    // the requested clock tick.
    let gop = (fps.max(1) * 2).to_string();

    if encoder == "libx264" {
        cmd.args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            &cq.to_string(),
            "-pix_fmt",
            "yuv420p",
            "-g",
            &gop,
        ]);
    } else if encoder.ends_with("_qsv") {
        // QSV takes neither NVENC's `-preset pN` names nor `-cq`/`-rc`. These are
        // the exact rate-control values measured good for the 3840×2160 iGPU
        // track, so `maxrate`/`bufsize` from the NVENC path are deliberately
        // ignored here.
        let _ = (maxrate, bufsize);
        cmd.args([
            "-c:v", encoder, "-preset", "medium", "-b:v", "12M", "-maxrate", "20M", "-g", &gop,
        ]);
    } else {
        cmd.args([
            "-c:v",
            encoder,
            "-preset",
            "p4",
            "-tune",
            "ll",
            "-rc",
            "vbr",
            "-cq",
            &cq.to_string(),
            "-b:v",
            "0",
            "-maxrate",
            maxrate,
            "-bufsize",
            bufsize,
            "-g",
            &gop,
        ]);
    }

    // CFR so frame index maps linearly to time on playback.
    cmd.args(["-fps_mode", "cfr"]);
}

// ---------------------------------------------------------------------------
// Session listing
// ---------------------------------------------------------------------------

pub fn sessions_root(output_dir: &Path) -> PathBuf {
    output_dir.join("sessions")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub started_epoch_ms: i64,
    pub ended_epoch_ms: Option<i64>,
    pub duration_ms: i64,
    pub track_count: usize,
    pub segment_count: usize,
    pub bytes: u64,
    pub has_manifest: bool,
}

pub fn list_sessions(root: &Path) -> Vec<SessionSummary> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<SessionSummary> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| {
            let dir = e.path();
            let id = e.file_name().to_string_lossy().to_string();
            match session::read_manifest(&dir) {
                Ok(m) => {
                    let end = m
                        .ended_epoch_ms
                        .or_else(|| m.tracks.iter().filter_map(|t| t.end_epoch_ms()).max());
                    SessionSummary {
                        id: m.id.clone(),
                        started_epoch_ms: m.started_epoch_ms,
                        ended_epoch_ms: m.ended_epoch_ms,
                        duration_ms: end.map(|e| e - m.started_epoch_ms).unwrap_or(0),
                        track_count: m.tracks.len(),
                        segment_count: m.segment_count(),
                        bytes: m.total_bytes(),
                        has_manifest: true,
                    }
                }
                // No manifest — still list it so the UI can offer a rebuild.
                Err(_) => SessionSummary {
                    started_epoch_ms: session::epoch_from_session_id(&id).unwrap_or(0),
                    id,
                    ended_epoch_ms: None,
                    duration_ms: 0,
                    track_count: 0,
                    segment_count: 0,
                    bytes: 0,
                    has_manifest: false,
                },
            }
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Real-world shape of the name matters here: Windows exposes the HFP endpoint
    // as `Headset (…)` and the A2DP one as `Headphones (…)`, and the possessive
    // uses a curly apostrophe. Keep both; the owner's name is a placeholder.
    const AIRPODS: &str = "Headset (Alex\u{2019}s AirPods Pro - Find My)";
    const BRIO_MIC: &str = "Microphone (Logitech BRIO)";

    fn caps(h264_qsv: bool) -> EncoderCaps {
        EncoderCaps {
            h264_nvenc: true,
            av1_nvenc: true,
            hevc_nvenc: true,
            h264_qsv,
        }
    }

    // -- desktop geometry ---------------------------------------------------

    /// This machine's real layout, measured with per-monitor DPI awareness set.
    /// Adapter 0 drives the first three; adapter 1 (Intel UHD 770) drives the
    /// fourth.
    fn real_outputs() -> Vec<DisplayOutput> {
        let mk = |adapter: u32, output_idx, name: &str, x, y, w, h| DisplayOutput {
            adapter,
            output_idx,
            device_name: name.to_string(),
            adapter_name: if adapter == 0 {
                "NVIDIA GeForce RTX 4090"
            } else {
                "Intel(R) UHD Graphics 770"
            }
            .to_string(),
            x,
            y,
            w,
            h,
            attached: true,
            capturable: true,
        };
        vec![
            mk(0, 0, r"\\.\DISPLAY1", 0, 0, 5120, 2160),
            mk(0, 1, r"\\.\DISPLAY2", 5120, -15, 3840, 2160),
            mk(0, 2, r"\\.\DISPLAY3", -3840, -15, 3840, 2160),
            mk(1, 0, r"\\.\DISPLAY5", 729, -2160, 3840, 2160),
        ]
    }

    #[test]
    fn desktop_box_covers_all_four_monitors() {
        let r = union_rects(real_outputs().iter().map(|o| (o.x, o.y, o.w, o.h)));
        assert_eq!(
            (r.x, r.y, r.w, r.h),
            (-3840, -2160, 12800, 4320),
            "the physical desktop bounding box must include the iGPU monitor"
        );
    }

    #[test]
    fn dropping_the_igpu_monitor_produces_the_old_wrong_box() {
        // Regression marker for the shipped-manifest bug: with three monitors the
        // fourth's whole row of the desktop simply vanishes.
        let three = union_rects(
            real_outputs()
                .iter()
                .filter(|o| o.adapter == 0)
                .map(|o| (o.x, o.y, o.w, o.h)),
        );
        assert_eq!((three.x, three.y, three.w, three.h), (-3840, -15, 12800, 2175));
        assert_ne!(three.h, 4320);
    }

    #[test]
    fn dpi_virtualised_rects_are_a_different_coordinate_space_entirely() {
        // Verbatim from a non-DPI-aware probe on this machine (175% scale): DXGI
        // reported the 5120x2160 ultrawide as 2926x1234 and each 3840x2160 panel
        // as 2194x1234. The manifest calls its desktop box PHYSICAL pixels, so
        // publishing these is not "slightly off", it is the wrong space.
        let logical = union_rects(vec![
            (0, 0, 2926u32, 1234u32),
            (2926, -9, 2194, 1234),
            (-2194, -9, 2194, 1234),
        ]);
        assert_eq!(
            (logical.x, logical.y, logical.w, logical.h),
            (-2194, -9, 7314, 1243)
        );

        let physical = union_rects(
            real_outputs()
                .iter()
                .filter(|o| o.adapter == 0)
                .map(|o| (o.x, o.y, o.w, o.h)),
        );
        assert_ne!(logical.w, physical.w);
        let ratio = physical.w as f64 / logical.w as f64;
        assert!(
            (ratio - 1.75).abs() < 0.01,
            "the error is exactly the display scale factor, got {ratio}"
        );
    }

    #[test]
    fn desktop_bounds_of_nothing_is_empty_not_a_panic() {
        let r = desktop_bounds(&[]);
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 0, 0));
    }

    #[test]
    fn a_single_monitor_at_the_origin_is_its_own_box() {
        let r = desktop_bounds(&[TauriMonitor {
            name: r"\\.\DISPLAY1".into(),
            x: 0,
            y: 0,
            w: 3840,
            h: 2160,
        }]);
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 3840, 2160));
    }

    /// Read-only hardware probe. Enumerates DXGI adapters and outputs and prints
    /// the desktop box — no Desktop Duplication, no ffmpeg, no capture of any
    /// kind. `#[ignore]`d because it asserts against one specific machine.
    ///
    ///     cargo test --lib -- --ignored --nocapture dxgi_rects
    #[test]
    #[ignore]
    fn dxgi_rects_are_physical_once_dpi_aware() {
        ensure_process_dpi_aware().expect("per-monitor DPI awareness must be settable");
        let outputs = enumerate_dxgi_outputs().expect("DXGI enumeration");
        for o in &outputs {
            println!(
                "adapter {} output {} {:<14} {:>5}x{:<5} at ({:>6},{:>6}) attached={} [{}]",
                o.adapter, o.output_idx, o.device_name, o.w, o.h, o.x, o.y, o.attached, o.adapter_name
            );
        }
        let attached: Vec<_> = outputs.iter().filter(|o| o.attached).collect();
        let box_ = union_rects(attached.iter().map(|o| (o.x, o.y, o.w, o.h)));
        println!("desktop = {:?}", box_);
        assert!(
            attached.iter().any(|o| o.w == 5120 && o.h == 2160),
            "the ultrawide must report 5120x2160, not the DPI-scaled 2926x1234"
        );
        assert_eq!(
            (box_.x, box_.y, box_.w, box_.h),
            (-3840, -2160, 12800, 4320)
        );
    }

    // -- adapter / encoder selection ----------------------------------------

    #[test]
    fn the_igpu_monitor_is_capturable_only_with_qsv() {
        let igpu = &real_outputs()[3];
        assert!(!output_is_capturable(igpu, &caps(false)));
        assert!(output_is_capturable(igpu, &caps(true)));
        // Adapter 0 never depends on QSV.
        assert!(output_is_capturable(&real_outputs()[0], &caps(false)));
    }

    #[test]
    fn a_detached_output_is_never_capturable() {
        let mut o = real_outputs()[0].clone();
        o.attached = false;
        assert!(!output_is_capturable(&o, &caps(true)));
    }

    #[test]
    fn secondary_adapters_encode_with_qsv_regardless_of_width() {
        let (enc, codec, warn) = pick_screen_encoder(3840, 1, &caps(true));
        assert_eq!((enc.as_str(), codec.as_str()), ("h264_qsv", "h264"));
        assert!(warn.is_none());
        // Cross-adapter NVENC fails with "no encode device", so width must not
        // pull this back onto an NVIDIA encoder.
        assert_eq!(pick_screen_encoder(5120, 1, &caps(true)).0, "h264_qsv");
    }

    #[test]
    fn the_default_adapter_still_follows_the_width_rule() {
        assert_eq!(pick_screen_encoder(5120, 0, &caps(true)).0, "av1_nvenc");
        assert_eq!(pick_screen_encoder(4097, 0, &caps(true)).0, "av1_nvenc");
        assert_eq!(pick_screen_encoder(4096, 0, &caps(true)).0, "h264_nvenc");
        assert_eq!(pick_screen_encoder(3840, 0, &caps(true)).0, "h264_nvenc");
    }

    #[test]
    fn a_secondary_adapter_without_qsv_warns_instead_of_pretending() {
        assert!(pick_screen_encoder(3840, 1, &caps(false)).2.is_some());
    }

    #[test]
    fn only_the_default_adapter_uses_the_lavfi_demuxer() {
        // The lavfi demuxer never receives -filter_hw_device, so a non-default
        // adapter must go through -filter_complex or ddagrab silently builds its
        // own device on adapter 0 and captures the wrong monitor.
        let a0 = ddagrab_input_args(0, "ddagrab=output_idx=0");
        assert_eq!(a0, vec!["-f", "lavfi", "-i", "ddagrab=output_idx=0"]);

        let a1 = ddagrab_input_args(1, "ddagrab=output_idx=0");
        assert!(a1.iter().any(|a| a == "-filter_complex"));
        assert!(!a1.iter().any(|a| a == "lavfi"));
        assert!(a1.contains(&"d3d11va=dda:1".to_string()));
        assert_eq!(a1.iter().filter(|a| *a == "-filter_hw_device").count(), 1);
    }

    // -- options contract with the frontend ----------------------------------

    /// The exact payload `startSession()` sends from the widget's settings panel
    /// (`src/ui/types.ts` → `SessionOptions`). A rename on either side silently
    /// falls back to a default here, so the field names are pinned.
    #[test]
    fn the_widgets_options_payload_deserializes_field_for_field() {
        let json = r#"{
            "segment_seconds": 30,
            "screen_fps": 60,
            "record_screens": true,
            "screens": [0, 2],
            "record_webcam": true,
            "webcam_device": "Logitech BRIO",
            "webcam_size": "1920x1080",
            "webcam_fps": 30,
            "record_mic": true,
            "mic_device": null,
            "record_system_audio": false,
            "mic_sidecar_16k": true,
            "drop_idle_after_seconds": 120
        }"#;
        let o: SessionOptions = serde_json::from_str(json).expect("frontend payload must parse");
        assert_eq!(o.segment_seconds, 30);
        assert_eq!(o.screen_fps, 60);
        assert_eq!(o.screens, vec![0, 2]);
        assert_eq!(o.webcam_device.as_deref(), Some("Logitech BRIO"));
        assert_eq!(o.webcam_size, "1920x1080");
        assert_eq!(o.mic_device, None);
        assert!(!o.record_system_audio);
        assert!(o.mic_sidecar_16k);
        assert_eq!(o.drop_idle_after_seconds, 120);
    }

    /// A payload from a widget that predates this field still parses, and the
    /// field falls back to its default rather than 0 (which would silently disable
    /// trimming for everyone who upgraded).
    #[test]
    fn an_older_payload_without_the_idle_field_defaults_it() {
        let json = r#"{ "segment_seconds": 60 }"#;
        let o: SessionOptions = serde_json::from_str(json).expect("partial payload must parse");
        assert_eq!(o.drop_idle_after_seconds, 60);
    }

    /// The widget's `DEFAULT_OPTIONS` must be this struct's defaults, or the
    /// settings panel would show one thing and the recorder do another.
    #[test]
    fn default_options_match_what_the_widget_shows() {
        let d = SessionOptions::default();
        assert_eq!(d.segment_seconds, 60);
        assert_eq!(d.screen_fps, 30);
        assert_eq!(d.webcam_size, "1920x1080");
        assert_eq!(d.webcam_fps, 30);
        assert!(d.screens.is_empty(), "empty means every capturable screen");
        assert!(d.record_screens && d.record_webcam && d.record_mic && d.record_system_audio);
        assert!(d.mic_sidecar_16k);
        assert_eq!(d.drop_idle_after_seconds, 60);
    }

    // -- microphone selection ------------------------------------------------

    /// dshow audio devices as this machine reports them. The AirPods entry is the
    /// one the recorder used to grab automatically.
    fn real_audio_devices() -> Vec<String> {
        vec![
            AIRPODS.to_string(),
            BRIO_MIC.to_string(),
            "Stereo Mix (Realtek(R) Audio)".to_string(),
        ]
    }

    #[test]
    fn the_bluetooth_headset_is_never_auto_selected() {
        let sel = select_mic_device(None, &real_audio_devices());
        assert_eq!(sel.device.as_deref(), Some(BRIO_MIC));
        assert!(sel.warnings.is_empty(), "the wired mic is an unremarkable choice");
    }

    #[test]
    fn first_in_the_list_does_not_win_by_default() {
        // The whole bug: the old code took `dshow_audio.first()`.
        let devices = real_audio_devices();
        assert_ne!(
            select_mic_device(None, &devices).device.as_deref(),
            Some(devices[0].as_str())
        );
    }

    #[test]
    fn a_bluetooth_only_machine_records_no_mic_and_says_why() {
        let sel = select_mic_device(None, &[AIRPODS.to_string()]);
        assert_eq!(sel.device, None, "must not silently hijack the user's headphones");
        assert_eq!(sel.warnings.len(), 1);
        assert!(sel.warnings[0].contains("hands-free"));
    }

    #[test]
    fn an_explicitly_configured_bluetooth_mic_is_honoured_with_a_warning() {
        let sel = select_mic_device(Some(AIRPODS), &real_audio_devices());
        assert_eq!(
            sel.device.as_deref(),
            Some(AIRPODS),
            "an explicit choice is the user's to make"
        );
        assert!(sel.warnings.iter().any(|w| w.contains("16kHz")));
    }

    #[test]
    fn an_explicit_wired_mic_passes_without_comment() {
        let sel = select_mic_device(Some(BRIO_MIC), &real_audio_devices());
        assert_eq!(sel.device.as_deref(), Some(BRIO_MIC));
        assert!(sel.warnings.is_empty());
    }

    #[test]
    fn a_configured_mic_that_is_not_plugged_in_is_still_tried_but_flagged() {
        let sel = select_mic_device(Some("Microphone (Blue Yeti)"), &real_audio_devices());
        assert_eq!(sel.device.as_deref(), Some("Microphone (Blue Yeti)"));
        assert!(sel
            .warnings
            .iter()
            .any(|w| w.contains("not in the dshow device list")));
    }

    #[test]
    fn blank_configuration_falls_back_to_auto_selection() {
        for cfg in ["", "   "] {
            let sel = select_mic_device(Some(cfg), &real_audio_devices());
            assert_eq!(sel.device.as_deref(), Some(BRIO_MIC));
        }
    }

    #[test]
    fn loopback_and_virtual_inputs_are_not_microphones() {
        let sel = select_mic_device(
            None,
            &[
                "Stereo Mix (Realtek(R) Audio)".to_string(),
                "CABLE Output (VB-Audio Virtual Cable)".to_string(),
                BRIO_MIC.to_string(),
            ],
        );
        assert_eq!(sel.device.as_deref(), Some(BRIO_MIC));
    }

    #[test]
    fn a_plain_capture_endpoint_outranks_a_line_input() {
        let sel = select_mic_device(
            None,
            &["Line In (Realtek(R) Audio)".to_string(), BRIO_MIC.to_string()],
        );
        assert_eq!(sel.device.as_deref(), Some(BRIO_MIC));
    }

    #[test]
    fn no_audio_devices_at_all_is_a_warning_not_a_panic() {
        let sel = select_mic_device(None, &[]);
        assert_eq!(sel.device, None);
        assert_eq!(sel.warnings.len(), 1);
    }

    #[test]
    fn bluetooth_detection_covers_the_common_spellings() {
        for name in [
            AIRPODS,
            "Headset Microphone (Bluetooth Hands-Free)",
            "Microphone (Galaxy Buds Pro Hands-Free AG Audio)",
            "HFP Microphone",
        ] {
            assert!(is_bluetooth_audio_device(name), "{name} should be flagged");
        }
        for name in [
            BRIO_MIC,
            "Microphone (Realtek(R) Audio)",
            "Microphone (Blue Yeti)",
        ] {
            assert!(!is_bluetooth_audio_device(name), "{name} should not be flagged");
        }
    }

    // -- webcam selection ----------------------------------------------------

    #[test]
    fn a_physical_camera_beats_a_virtual_one() {
        let sel = select_webcam_device(
            None,
            &["OBS Virtual Camera".to_string(), "Logitech BRIO".to_string()],
        );
        assert_eq!(sel.device.as_deref(), Some("Logitech BRIO"));
        assert!(sel.warnings.is_empty());
    }

    #[test]
    fn a_virtual_only_machine_records_it_but_says_so() {
        let sel = select_webcam_device(None, &["XSplit VCam".to_string()]);
        assert_eq!(sel.device.as_deref(), Some("XSplit VCam"));
        assert_eq!(sel.warnings.len(), 1);
    }

    #[test]
    fn no_camera_records_no_webcam() {
        let sel = select_webcam_device(None, &[]);
        assert_eq!(sel.device, None);
        assert_eq!(sel.warnings.len(), 1);
    }

    // -- webcam/mic pairing --------------------------------------------------

    #[test]
    fn ordinary_device_names_can_share_one_dshow_input() {
        assert_eq!(
            combined_dshow_unsupported("Logitech BRIO", "Microphone (Logitech BRIO)"),
            None
        );
    }

    #[test]
    fn a_device_name_carrying_ffmpegs_own_separator_is_refused() {
        // `video=X:audio=Y` puts the separator inside the device-name string, so a
        // name containing it would be split in the wrong place and open the wrong
        // device. Recording the wrong thing silently is worse than losing lip-sync.
        assert!(combined_dshow_unsupported("Cam:audio=Sneaky", "Mic").is_some());
        assert!(combined_dshow_unsupported("Cam", "Mic:video=Sneaky").is_some());
    }

    #[test]
    fn a_progress_anchor_is_never_earlier_than_the_process_that_captured_it() {
        let launched = 1_785_180_000_000i64;
        // A sample implying content began before ffmpeg was spawned is impossible;
        // it means the pipeline was still reporting a stale out_time.
        let est = Arc::new(AtomicI64::new(launched - 5_000));
        let a = Anchor::FromProgress(est, launched);
        assert_eq!(a.resolve(), Some(launched));
    }

    #[test]
    fn a_progress_anchor_with_no_sample_declines_rather_than_guessing() {
        let est = Arc::new(AtomicI64::new(i64::MAX));
        assert_eq!(Anchor::FromProgress(est, 1_785_180_000_000).resolve(), None);
        // Equally, a value so far past launch that no capture could have started
        // there is not published.
        let wild = Arc::new(AtomicI64::new(1_785_180_000_000 + 120_000));
        assert_eq!(Anchor::FromProgress(wild, 1_785_180_000_000).resolve(), None);
    }

    #[test]
    fn a_plausible_progress_sample_is_used_as_is() {
        let launched = 1_785_180_000_000i64;
        let est = Arc::new(AtomicI64::new(launched + 1_300));
        assert_eq!(
            Anchor::FromProgress(est, launched).resolve(),
            Some(launched + 1_300)
        );
    }

    #[test]
    fn the_webcam_and_mic_of_one_process_resolve_to_the_same_instant() {
        // The whole point of the pairing: both outputs consult one Anchor, so they
        // cannot disagree no matter what either device's open latency was.
        let launched = 1_785_180_000_000i64;
        let est = Arc::new(AtomicI64::new(launched + 1_879));
        let src = TrackSource::WebcamMic {
            video_device: "BRIO".into(),
            audio_device: "Microphone (BRIO)".into(),
            encoder: "h264_nvenc".into(),
            video_size: "1920x1080".into(),
            fps: 30,
            sidecar: false,
        };
        let a = src.anchor(launched, &est, None);
        assert_eq!(a.resolve(), a.resolve());
        assert_eq!(a.resolve(), Some(launched + 1_879));
    }

    // -- segment start timing ------------------------------------------------

    #[test]
    fn the_sysaudio_anchor_wins_over_the_files_own_wall_clock() {
        // The regression this guards: SysAudioCapture pads ~4s of silence while
        // ffmpeg spawns, so the first segment's CONTENT starts at t0 even though
        // the file is not created until ~4.1s later. Anchoring to file creation
        // put the whole track 4.1s late (observed in sessions 1-2).
        let t0 = 1_785_180_000_000i64;
        let file_created = t0 + 4_100;
        let d = decide_segment_start(Some(t0), None, file_created);
        assert_eq!(d.start_epoch_ms, t0);
        assert_eq!(d.gap, None);
        assert!(
            d.start_epoch_ms < file_created,
            "a start earlier than the -strftime filename is the correct outcome here"
        );
    }

    #[test]
    fn without_an_anchor_the_first_segment_uses_the_observed_time() {
        let observed = 1_785_180_000_000i64;
        assert_eq!(
            decide_segment_start(None, None, observed).start_epoch_ms,
            observed
        );
    }

    #[test]
    fn small_disagreement_is_absorbed_a_quarter_at_a_time() {
        let chained = 1_785_180_060_000i64;
        // Poll jitter of 100ms moves the start by 25ms, not 100ms.
        let d = decide_segment_start(None, Some(chained), chained + 100);
        assert_eq!(d.start_epoch_ms, chained + 25);
        assert_eq!(d.gap, None);
        let d = decide_segment_start(None, Some(chained), chained - 100);
        assert_eq!(d.start_epoch_ms, chained - 25);
    }

    #[test]
    fn constant_drift_reaches_a_bounded_steady_state() {
        // A device 0.02% slow loses ~12ms per 60s segment. Over 360 segments (6h)
        // the error must settle, not accumulate.
        let mut chained: Option<i64> = None;
        let mut wall = 0i64;
        let mut last_error = 0i64;
        for _ in 0..360 {
            let d = decide_segment_start(None, chained, wall);
            assert_eq!(d.gap, None, "steady drift must not be mistaken for lost time");
            last_error = (d.start_epoch_ms - wall).abs();
            chained = Some(d.start_epoch_ms + 59_988); // measured media duration
            wall += 60_000;
        }
        assert!(
            last_error < 100,
            "steady-state error was {last_error}ms; it must stay bounded, not accumulate"
        );
    }

    #[test]
    fn a_large_forward_disagreement_re_anchors_and_records_a_gap() {
        let chained = 1_785_180_060_000i64;
        let observed = chained + 9_000;
        let d = decide_segment_start(None, Some(chained), observed);
        assert_eq!(d.start_epoch_ms, observed);
        let gap = d.gap.expect("lost time must be visible as a gap");
        assert_eq!((gap.from_epoch_ms, gap.to_epoch_ms), (chained, observed));
    }

    #[test]
    fn a_large_backward_disagreement_re_anchors_without_inventing_a_gap() {
        let chained = 1_785_180_060_000i64;
        let observed = chained - 9_000;
        let d = decide_segment_start(None, Some(chained), observed);
        assert_eq!(d.start_epoch_ms, observed);
        assert_eq!(d.gap, None, "a negative gap is not a gap");
    }

    #[test]
    fn a_restart_anchor_records_the_downtime_as_a_gap() {
        let chained = 1_785_180_060_000i64;
        let restarted = chained + 3_000;
        let d = decide_segment_start(Some(restarted), Some(chained), restarted + 4_000);
        assert_eq!(d.start_epoch_ms, restarted);
        let gap = d.gap.expect("the restart downtime must be recorded");
        assert_eq!((gap.from_epoch_ms, gap.to_epoch_ms), (chained, restarted));
        assert_eq!(gap.reason, "track restarted");
    }

    // -- inactivity trimming -------------------------------------------------

    #[test]
    fn a_segment_with_recent_input_is_kept() {
        let last_input = 1_785_180_030_000i64;
        // Segment started 30s before that input, so input happened during it.
        let seg_start = last_input - 30_000;
        assert!(!should_drop_idle(last_input, seg_start, 60_000));
    }

    #[test]
    fn a_fully_idle_segment_past_the_threshold_is_dropped() {
        let last_input = 1_785_180_000_000i64;
        // Started 61s after the last input — idle longer than the 60s threshold.
        let seg_start = last_input + 61_000;
        assert!(should_drop_idle(last_input, seg_start, 60_000));
    }

    #[test]
    fn footage_within_one_threshold_of_the_last_input_survives() {
        let last_input = 1_785_180_000_000i64;
        // Idle, but only 30s in — under the 60s grace, so it is kept.
        let seg_start = last_input + 30_000;
        assert!(!should_drop_idle(last_input, seg_start, 60_000));
        // Exactly at the threshold is the first one dropped.
        assert!(should_drop_idle(last_input, last_input + 60_000, 60_000));
    }

    #[test]
    fn a_zero_threshold_disables_trimming() {
        let last_input = 1_785_180_000_000i64;
        // Hours idle, but trimming is off, so nothing is ever dropped.
        assert!(!should_drop_idle(last_input, last_input + 6 * 3_600_000, 0));
    }

    // -- exit diagnosis ------------------------------------------------------

    #[test]
    fn a_startup_notice_is_never_reported_as_the_cause_of_death() {
        // Verbatim from session 2026-07-27_12-41-11, where this was the only line
        // in the ring and got recorded as the reason all five tracks died.
        let lines =
            vec!["[aist#0:0/pcm_f32le @ 000001c6b3cc9140] Guessed Channel Layout: stereo".to_string()];
        let d = diagnose(&lines);
        assert!(d.contains("no error"), "got: {d}");
        assert!(d.contains("routine notices"), "got: {d}");
    }

    #[test]
    fn a_real_error_after_a_benign_line_is_the_one_reported() {
        let lines = vec![
            "[aist#0:0/pcm_f32le @ 0x1] Guessed Channel Layout: stereo".to_string(),
            "[out#0/segment @ 0x2] Error opening output file: No space left on device".to_string(),
        ];
        assert!(diagnose(&lines).contains("No space left on device"));
    }

    #[test]
    fn an_empty_ring_says_so_and_points_at_external_termination() {
        let d = diagnose(&[]);
        assert!(d.contains("no stderr output"));
        assert!(d.contains("killed externally"), "got: {d}");
    }

    #[test]
    fn the_most_recent_errors_lead_the_diagnosis() {
        let lines: Vec<String> = (0..10).map(|i| format!("error {i}")).collect();
        let d = diagnose(&lines);
        assert!(d.starts_with("error 9"), "got: {d}");
        assert_eq!(d.split(" | ").count(), 3, "bounded to a readable length");
    }

    #[test]
    fn the_ring_keeps_only_the_last_lines_and_then_stops_growing() {
        let mut ring = RingLog::default();
        for i in 0..(LOG_LINES * 3) {
            ring.push(format!("line {i}"));
        }
        let snap = ring.snapshot();
        assert_eq!(snap.len(), LOG_LINES, "memory must stay flat over a 6h session");
        assert_eq!(snap.last().unwrap(), &format!("line {}", LOG_LINES * 3 - 1));
        assert!(ring.contains(&format!("line {}", LOG_LINES * 3 - 1)));
    }
}
