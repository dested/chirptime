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
/// Give up on a track after this many *consecutive* restarts that fail for a
/// reason we cannot name. A run that records resets the count. A screen waiting
/// out a display outage counts toward it only until the track has recorded
/// something this session: an encoder that can never open must still fail, not
/// wait forever under an outage's name. See [`Supervision::failed`].
const MAX_RESTARTS: u32 = 20;
/// Longest wait between attempts while a screen waits out an outage. The wait
/// doubles from 1s up to this and then holds, for as long as the session runs.
/// See [`outage_backoff_ms`].
const OUTAGE_BACKOFF_CAP_MS: u64 = 30_000;
/// No keyboard or mouse input for this long counts as the user being away; the
/// next input is them coming back, which is when a sleeping display wakes. See
/// [`user_came_back`].
const WAKE_IDLE_MS: i64 = 30_000;
/// After the user comes back, wait this long before retrying a waiting screen.
/// Measured in the event logs: the monitors drop off the bus about 1s after the
/// first keypress and re-appear 1.5–2s later (2026-08-01: key 15:20:35.7, gone
/// 36.8, back 38.3), and a capture running across that re-link loses access.
const WAKE_SETTLE_MS: u64 = 3_000;
/// How long a screen capture may take to deliver its first frame while the user
/// is active. A healthy start takes about a second.
const SCREEN_FIRST_FRAME_ACTIVE_MS: i64 = 15_000;
/// The same, while the user is away. `ddagrab` will not finish opening until the
/// desktop presents a frame, and with the display off that never happens: it
/// polls `AcquireNextFrame` with a zero timeout, spinning a core and printing
/// nothing. This bounds what each attempt during a dark display costs.
///
/// A lit but unchanging desktop is not at risk. A fresh duplication hands over
/// the whole current image on its first acquire, change or no change: in all 28
/// sessions on disk every screen's first segment landed 0.4s after its staggered
/// launch (0.40 / 0.70 / 1.00 / 1.40s), on four monitors and two adapters, which
/// waiting on desktop activity could not produce. And every silent no-frame run
/// in the logs (60, all 2026-08-01 14:02–14:13) falls inside a display-off window.
const SCREEN_FIRST_FRAME_IDLE_MS: i64 = 5_000;
/// No input for this long, and a screen waiting out an outage is retried only
/// every [`AWAY_RETRY_MS`] once its backoff has reached the cap. The wake
/// shortcut ([`wait_for_retry`]) still retries 3s after the user comes back.
const AWAY_IDLE_MS: i64 = 60_000;
/// The spacing of outage attempts while the user is away. See [`outage_delay_ms`].
const AWAY_RETRY_MS: u64 = 120_000;
/// After this many consecutive relaunches that found no monitor, try one launch
/// at the screen's session-start output index anyway, in case matching is what
/// failed (a monitor back under a new name *and* a new rectangle).
const MISSING_PROBE_EVERY: u32 = 10;
/// A monitor must show up on another adapter in this many consecutive
/// enumerations before its track gives up; one look mid-reconnect is not proof.
const MOVED_ADAPTER_CONFIRMATIONS: u32 = 3;
/// The reason on every gap left by inactivity trimming.
const IDLE_GAP_REASON: &str = "idle — no keyboard or mouse activity";
/// The reason on the gap a pause leaves on every track that had recorded. The
/// Player's log matches it exactly.
pub const PAUSED_GAP_REASON: &str = "paused";
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
    /// How to find this monitor again after the display topology changes.
    pub identity: ScreenIdentity,
}

/// What identifies a monitor across a reconnect, as DXGI reports it.
///
/// `output_idx` is not it: DXGI renumbers outputs when a monitor drops off the
/// bus and comes back, which DisplayPort monitors do on every wake from display
/// sleep. The device name is not stable either (the same ultrawide was
/// `\\.\DISPLAY1` in one session and `\\.\DISPLAY11` in another). The desktop
/// rectangle is what Windows restores for a returning monitor, so it leads; the
/// name breaks ties and covers a resolution change. See [`relocate_screen`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenIdentity {
    pub device_name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Where a screen's monitor is now. See [`relocate_screen`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relocation {
    /// Present on the track's own adapter, at this output index (which may have
    /// changed since the session started).
    Found(u32),
    /// No attached output matches: unplugged, switched off hard, or mid-reconnect.
    Missing,
    /// Back, but on another adapter. The track's capture and encode path was
    /// built for its original adapter (NVENC vs QSV), so it cannot follow.
    OtherAdapter(u32),
}

/// Find a screen's monitor in a fresh DXGI enumeration. Pure, so the matching
/// rules can be tested against real layouts.
///
/// `identity` is the screen as it was at session start, never updated, and
/// `others` is every screen of the session the same way (it may include
/// `identity` itself). Matching against a moving identity is how one monitor
/// ends up recorded twice: two same-size monitors that trade `\\.\DISPLAYn`
/// names while the layout settles on wake would each take the other's place for
/// good.
///
/// On the track's own adapter, in order: the same name *and* rectangle; the same
/// rectangle under a new name (Windows reassigns `\\.\DISPLAYn` on reconnect);
/// the same name with a new rectangle (a resolution or layout change), unless it
/// sits on another screen's rectangle, where it is that screen's monitor under
/// our old name. An output that is exactly another screen's identity is never a
/// candidate. Within a rule, `home_idx` (the session-start output index) wins a
/// tie. Only an exact name-and-rectangle match counts as having moved to another
/// adapter, so a stranger that happens to land on the old rectangle there cannot
/// end the track. Detached outputs never match.
pub fn relocate_screen(
    identity: &ScreenIdentity,
    adapter: u32,
    home_idx: u32,
    others: &[ScreenIdentity],
    outputs: &[DisplayOutput],
) -> Relocation {
    let rect = |o: &DisplayOutput| (o.x, o.y, o.w, o.h);
    let same_rect = |o: &DisplayOutput| rect(o) == identity.rect();
    let same_name = |o: &DisplayOutput| o.device_name == identity.device_name;
    let others: Vec<&ScreenIdentity> = others.iter().filter(|i| *i != identity).collect();
    let on_anothers_rect = |o: &DisplayOutput| others.iter().any(|i| i.rect() == rect(o));
    let here: Vec<&DisplayOutput> = outputs
        .iter()
        .filter(|o| o.attached && o.adapter == adapter)
        .filter(|o| !others.iter().any(|i| i.is(o)))
        .collect();
    let pick = |matches: &dyn Fn(&DisplayOutput) -> bool| {
        let hits: Vec<&DisplayOutput> = here.iter().copied().filter(|o| matches(o)).collect();
        hits.iter()
            .find(|o| o.output_idx == home_idx)
            .or(hits.first())
            .map(|o| o.output_idx)
    };
    let found = pick(&|o| same_name(o) && same_rect(o))
        .or_else(|| pick(&|o| same_rect(o)))
        .or_else(|| pick(&|o| same_name(o) && !on_anothers_rect(o)));
    if let Some(idx) = found {
        return Relocation::Found(idx);
    }
    match outputs
        .iter()
        .find(|o| o.attached && o.adapter != adapter && same_name(o) && same_rect(o))
    {
        Some(o) => Relocation::OtherAdapter(o.adapter),
        None => Relocation::Missing,
    }
}

impl ScreenIdentity {
    fn rect(&self) -> (i32, i32, u32, u32) {
        (self.x, self.y, self.w, self.h)
    }

    /// Exactly this name on exactly this rectangle.
    fn is(&self, o: &DisplayOutput) -> bool {
        o.device_name == self.device_name && (o.x, o.y, o.w, o.h) == self.rect()
    }
}

/// Whether the attached output at a screen's session-start index is plainly
/// another screen's monitor: it sits on another screen's rectangle, under any
/// name. A launch there would record that monitor twice. See
/// [`MISSING_PROBE_EVERY`].
pub fn home_is_anothers(
    identity: &ScreenIdentity,
    adapter: u32,
    home_idx: u32,
    others: &[ScreenIdentity],
    outputs: &[DisplayOutput],
) -> bool {
    outputs
        .iter()
        .filter(|o| o.attached && o.adapter == adapter && o.output_idx == home_idx)
        .any(|o| {
            others
                .iter()
                .filter(|i| *i != identity)
                .any(|i| i.rect() == (o.x, o.y, o.w, o.h))
        })
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
/// - **A monitor renumbered, renamed or unplugged mid-session** is found again
///   before every relaunch of its track ([`relocate_screen`]). One that comes
///   back on a different adapter stops its track for good. A monitor plugged in
///   mid-session that was not there at the start is not picked up.
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
                // DXGI's own rectangle, not the Tauri-joined one above: a later
                // re-enumeration is matched against exactly what DXGI reports.
                identity: ScreenIdentity {
                    device_name: o.device_name.clone(),
                    x: o.x,
                    y: o.y,
                    w: o.w,
                    h: o.h,
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
                identity: ScreenIdentity {
                    device_name: m.name.clone(),
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
        /// Re-resolved before every relaunch; see [`relocate_screen`].
        output_idx: u32,
        /// The output index at session start. Never changes.
        home_idx: u32,
        /// The monitor as it was at session start. Never changes: every
        /// relocation matches against this, not against the last match.
        identity: ScreenIdentity,
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
    /// The session is ending. Never cleared.
    stop: AtomicBool,
    /// Every worker winds down as if stopping, for a pause. Cleared by resume
    /// once they have all exited. See [`Recorder::pause`].
    pausing: AtomicBool,
    /// Bumped by "Restart screens": every screen waiting out an outage retries
    /// at once. See [`wait_for_retry`].
    retry_asked: AtomicI64,
    /// Track ids with a supervisor running (or about to). Claimed before the
    /// thread is spawned and released when it ends, so Restart screens never
    /// starts a second supervisor for a track that has one.
    live_supervisors: Mutex<std::collections::HashSet<String>>,
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
    /// Live recovery state per manifest track, for the widget. Absent while a
    /// track records. See [`TrackHealth`].
    health: Mutex<std::collections::HashMap<String, TrackHealth>>,
    /// Every capturable screen as it was at session start, recorded or not. A
    /// screen relocating its monitor never takes one of these. See
    /// [`relocate_screen`].
    screen_identities: Vec<ScreenIdentity>,
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

    /// Whether workers should wind down: the session is stopping, or pausing.
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst) || self.pausing.load(Ordering::SeqCst)
    }

    fn set_health(&self, outs: &[OutputState], health: TrackHealth) {
        let mut h = self.health.lock().unwrap();
        for o in outs {
            h.insert(o.track_id.clone(), health.clone());
        }
    }

    fn clear_health(&self, outs: &[OutputState]) {
        let mut h = self.health.lock().unwrap();
        for o in outs {
            h.remove(&o.track_id);
        }
    }
}

pub struct Recorder {
    inner: Arc<Inner>,
    joins: Mutex<Vec<JoinHandle<()>>>,
    stopped: AtomicBool,
    /// What `start` launched, relaunched as-is by `resume`.
    specs: Vec<TrackSpec>,
    /// When the session was paused, while it is.
    paused_since: Mutex<Option<i64>>,
    /// Serialises pause, resume and stop.
    control: Mutex<()>,
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

    /// The session is ending (asked to, or stopped itself). A pause is not
    /// stopping: the edit gate and `start_session` read this.
    pub fn is_stopping(&self) -> bool {
        self.inner.stop.load(Ordering::SeqCst)
    }

    /// When the session was paused, while it is.
    pub fn paused_since(&self) -> Option<i64> {
        *self.paused_since.lock().unwrap()
    }

    /// Each track's id, kind and status, without cloning the segment lists:
    /// the tray reads this every second.
    pub fn track_overview(&self) -> Vec<(String, String, TrackStatus)> {
        self.inner
            .manifest
            .lock()
            .unwrap()
            .tracks
            .iter()
            .map(|t| (t.id.clone(), t.kind.clone(), t.status))
            .collect()
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

    /// Why `track` is not recording right now, in words, and whether it is
    /// waiting to come back. All `None` while it records.
    pub fn track_health(&self, track: &str) -> TrackHealth {
        self.inner
            .health
            .lock()
            .unwrap()
            .get(track)
            .cloned()
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

    /// Set the note on one of this live session's marks; empty means no note.
    /// The recorder owns the manifest while it records, so a live note goes
    /// through it (the sidecar's last line per instant wins on a rebuild).
    pub fn set_marker_label(&self, epoch_ms: i64, label: &str) -> Result<session::Marker, String> {
        let out = {
            let mut m = self.inner.manifest.lock().unwrap();
            let mk = m
                .markers
                .iter_mut()
                .find(|mk| mk.epoch_ms == epoch_ms)
                .ok_or("That mark is not in this session")?;
            mk.label = label.trim().to_string();
            mk.clone()
        };
        session::append_marker(&self.inner.session_dir, &out)?;
        self.inner.flush();
        Ok(out)
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
        let screen_identities: Vec<ScreenIdentity> =
            screens.iter().map(|t| t.identity.clone()).collect();
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
                        home_idx: t.output_idx,
                        identity: t.identity.clone(),
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
            pausing: AtomicBool::new(false),
            retry_asked: AtomicI64::new(0),
            live_supervisors: Mutex::new(Default::default()),
            logs: Mutex::new(Default::default()),
            warnings: Mutex::new(warnings),
            fatal: Mutex::new(None),
            extra_joins: Mutex::new(Vec::new()),
            last_input_ms: AtomicI64::new(seed_input_ms),
            drop_idle_after_ms,
            health: Mutex::new(Default::default()),
            screen_identities,
        });
        inner.flush();

        let mut joins = Vec::new();
        if let Err(e) = spawn_workers(&inner, &specs, None, &mut joins) {
            inner.stop.store(true, Ordering::SeqCst);
            for h in joins {
                let _ = h.join();
            }
            return Err(e);
        }

        Ok(Arc::new(Recorder {
            inner,
            joins: Mutex::new(joins),
            stopped: AtomicBool::new(false),
            specs,
            paused_since: Mutex::new(None),
            control: Mutex::new(()),
        }))
    }

    /// Join every worker, including supervisors spawned after `start`.
    fn join_workers(&self) {
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
    }

    /// Stop every track's ffmpeg (each finishes its segment, as at stop) and
    /// the cursor sampler, keeping the session open. Returns when the pause
    /// began. Blocking — call from `spawn_blocking`.
    ///
    /// Nothing is written for the pause until `resume` or `stop`, which close
    /// it as one [`PAUSED_GAP_REASON`] gap per track that had recorded.
    pub fn pause(&self) -> Result<i64, String> {
        let _g = self.control.lock().unwrap();
        if self.stopped.load(Ordering::SeqCst) || self.inner.stop.load(Ordering::SeqCst) {
            return Err("This session has already stopped".to_string());
        }
        if let Some(t) = self.paused_since() {
            return Ok(t);
        }
        let at = now_ms();
        self.inner.pausing.store(true, Ordering::SeqCst);
        self.join_workers();
        *self.paused_since.lock().unwrap() = Some(at);
        self.inner.flush();
        Ok(at)
    }

    /// Restart every track into the same session. Each track that had recorded
    /// gets one `paused` gap from where its footage (or its last gap) ended to
    /// where its footage resumes. Blocking — call from `spawn_blocking`.
    pub fn resume(&self) -> Result<(), String> {
        let _g = self.control.lock().unwrap();
        if self.stopped.load(Ordering::SeqCst) || self.inner.stop.load(Ordering::SeqCst) {
            return Err(self
                .fatal()
                .unwrap_or_else(|| "This session has already stopped".to_string()));
        }
        if self.paused_since().is_none() {
            return Ok(());
        }
        if let Some(free) = free_space_bytes(&self.inner.session_dir) {
            if free < MIN_FREE_BYTES {
                return Err(format!(
                    "Only {} MB free on the recording drive. Free some space, then resume.",
                    free / 1024 / 1024
                ));
            }
        }
        let now = now_ms();
        let edges = {
            let mut m = self.inner.manifest.lock().unwrap();
            close_pause(&mut m, now)
        };
        self.inner.flush();
        // Pressing Resume was input; the poller's reading is up to 2 s stale.
        self.inner.last_input_ms.fetch_max(now, Ordering::Relaxed);
        // Restarting until footage shows, from before any thread exists.
        for spec in &self.specs {
            for o in &spec.outputs {
                set_track_status(&self.inner, &o.track_id, TrackStatus::Restarting, None);
            }
        }
        self.inner.pausing.store(false, Ordering::SeqCst);
        let mut joins = Vec::new();
        if let Err(e) = spawn_workers(&self.inner, &self.specs, Some(&edges), &mut joins) {
            // Still paused: wind down whatever did start.
            self.inner.pausing.store(true, Ordering::SeqCst);
            for h in joins {
                let _ = h.join();
            }
            self.inner.flush();
            return Err(e);
        }
        *self.joins.lock().unwrap() = joins;
        *self.paused_since.lock().unwrap() = None;
        self.inner.flush();
        Ok(())
    }

    /// "Restart screens": every screen waiting out an outage retries now, and
    /// every screen that gave up gets a new supervisor, its downtime gap
    /// carrying on until footage resumes. Returns how many screens were down.
    pub fn restart_screens(&self) -> Result<usize, String> {
        let _g = self.control.lock().unwrap();
        if self.stopped.load(Ordering::SeqCst) || self.inner.stop.load(Ordering::SeqCst) {
            return Err("This session has already stopped".to_string());
        }
        if self.paused_since().is_some() {
            return Err("Paused: resume to restart the screens".to_string());
        }
        self.inner.retry_asked.fetch_add(1, Ordering::SeqCst);
        let (waiting, failed): (usize, Vec<(String, Option<Gap>)>) = {
            let m = self.inner.manifest.lock().unwrap();
            let screens = m.tracks.iter().filter(|t| t.kind == "screen");
            let waiting = screens
                .clone()
                .filter(|t| t.status == TrackStatus::Restarting)
                .count();
            let failed = screens
                .filter(|t| t.status == TrackStatus::Failed)
                .map(|t| (t.id.clone(), t.gaps.last().cloned()))
                .collect();
            (waiting, failed)
        };
        for (id, last_gap) in &failed {
            let Some(spec) = self.specs.iter().find(|s| s.id == *id).cloned() else {
                continue;
            };
            let Some(claim) = claim_supervisor(&self.inner, &spec.id) else {
                continue;
            };
            let mut outs: Vec<OutputState> = spec
                .outputs
                .iter()
                .map(|o| OutputState::new(&self.inner.session_dir, o))
                .collect();
            let carried = carried_downtime(last_gap.as_ref());
            for o in outs.iter_mut() {
                o.downtime = carried.clone();
            }
            self.inner.clear_health(&outs);
            set_all_status(&self.inner, &outs, TrackStatus::Restarting, None);
            let h = spawn_supervisor(claim, spec, move |inner, spec| {
                supervise_outputs(inner, spec, outs, true)
            })?;
            self.joins.lock().unwrap().push(h);
        }
        self.inner.flush();
        Ok(waiting + failed.len())
    }

    /// Signal every track to finish the in-flight segment gracefully, then join.
    /// Blocking — call from `spawn_blocking`.
    pub fn stop(&self) -> SessionManifest {
        let _g = self.control.lock().unwrap();
        if !self.stopped.swap(true, Ordering::SeqCst) {
            self.inner.stop.store(true, Ordering::SeqCst);
            self.join_workers();
            {
                let mut m = self.inner.manifest.lock().unwrap();
                let now = now_ms();
                // Stopped while paused: the pause runs to the end.
                if self.paused_since.lock().unwrap().take().is_some() {
                    close_pause(&mut m, now);
                }
                m.ended_epoch_ms = Some(now);
                m.error = self.inner.fatal.lock().unwrap().clone();
            }
            self.inner.flush();
        }
        self.manifest()
    }
}

/// The outage gap a restarted screen picks back up, so it does not start a
/// second one. A pause or an idle stretch is its own story and is not stretched
/// over the outage that follows.
fn carried_downtime(last_gap: Option<&Gap>) -> Option<Downtime> {
    let g = last_gap?;
    if g.reason == PAUSED_GAP_REASON || g.reason == IDLE_GAP_REASON {
        return None;
    }
    Some(Downtime {
        from: g.from_epoch_ms,
        reason: g.reason.clone(),
    })
}

/// Where each recorded track's pause begins: the later of its footage's end and
/// its last gap's end (an idle stretch or an outage that ran into the pause
/// keeps its own name). Tracks that never recorded have no hole to describe.
fn pause_edges(m: &SessionManifest) -> Vec<(String, i64)> {
    m.tracks
        .iter()
        .filter_map(|t| {
            let end = t.end_epoch_ms()?;
            let gaps = t.gaps.iter().map(|g| g.to_epoch_ms).max().unwrap_or(end);
            Some((t.id.clone(), end.max(gaps)))
        })
        .collect()
}

/// Write each recorded track's `paused` gap up to `now` and return the edges
/// they start from. Pure over the manifest, for the tests.
fn close_pause(m: &mut SessionManifest, now: i64) -> Vec<(String, i64)> {
    let edges = pause_edges(m);
    for (track, from) in &edges {
        if let Some(t) = m.track_mut(track) {
            let d = Downtime {
                from: *from,
                reason: PAUSED_GAP_REASON.to_string(),
            };
            stretch_downtime_gap(&mut t.gaps, &d, now);
        }
    }
    edges
}

/// Releases a track's claim in [`Inner::live_supervisors`] when its
/// supervisor ends, however it ends.
struct SupervisorClaim {
    inner: Arc<Inner>,
    id: String,
}

impl Drop for SupervisorClaim {
    fn drop(&mut self) {
        self.inner.live_supervisors.lock().unwrap().remove(&self.id);
    }
}

/// Claim `id` for a new supervisor; `None` when one already runs.
fn claim_supervisor(inner: &Arc<Inner>, id: &str) -> Option<SupervisorClaim> {
    if !inner.live_supervisors.lock().unwrap().insert(id.to_string()) {
        return None;
    }
    Some(SupervisorClaim {
        inner: inner.clone(),
        id: id.to_string(),
    })
}

/// Spawn one supervisor thread holding `claim` for its whole life.
fn spawn_supervisor(
    claim: SupervisorClaim,
    spec: TrackSpec,
    run: impl FnOnce(Arc<Inner>, TrackSpec) + Send + 'static,
) -> Result<JoinHandle<()>, String> {
    let inner = claim.inner.clone();
    std::thread::Builder::new()
        .name(format!("rec-{}", spec.id))
        .spawn(move || {
            let _claim = claim;
            run(inner, spec)
        })
        .map_err(|e| format!("spawn supervisor: {}", e))
}

/// Start a run's workers: one supervisor per spec, the disk guard, the idle
/// poller (when trimming is on) and the cursor sampler. `resumed` carries each
/// track's pause edge on a resume.
///
/// Each handle lands in `joins` as it is spawned, so on an error the caller
/// can wind down and join what did start.
fn spawn_workers(
    inner: &Arc<Inner>,
    specs: &[TrackSpec],
    resumed: Option<&[(String, i64)]>,
    joins: &mut Vec<JoinHandle<()>>,
) -> Result<(), String> {
    // Spawn every supervisor as close together as possible; each records its
    // own measured start instant, so a few ms of skew here is recovered later.
    for spec in specs.iter().cloned() {
        let Some(claim) = claim_supervisor(inner, &spec.id) else {
            continue;
        };
        let edges: Option<Vec<(String, i64)>> = resumed.map(|r| r.to_vec());
        joins.push(spawn_supervisor(claim, spec, move |inner, spec| match edges {
            None => supervise_track(inner, spec),
            Some(edges) => supervise_resumed(inner, spec, &edges),
        })?);
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

    // Cursor track.
    {
        let inner = inner.clone();
        joins.push(
            std::thread::Builder::new()
                .name("rec-cursor".into())
                .spawn(move || cursor_loop(inner))
                .map_err(|e| format!("spawn cursor sampler: {}", e))?,
        );
    }

    Ok(())
}

/// Sample the cursor every [`crate::cursor::SAMPLE_MS`] into a new run file
/// until the session stops or pauses. A failure costs the cursor track only,
/// and says so once.
fn cursor_loop(inner: Arc<Inner>) {
    let mut w = match crate::cursor::CursorWriter::create(&inner.session_dir, now_ms()) {
        Ok(w) => w,
        Err(e) => {
            inner.warn(format!("cursor track not recorded: {}", e));
            return;
        }
    };
    while !inner.stopping() {
        if let Some((x, y)) = crate::cursor::cursor_pos() {
            if let Err(e) = w.sample(now_ms(), x, y) {
                inner.warn(format!("cursor track stopped: {}", e));
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(crate::cursor::SAMPLE_MS));
    }
    let _ = w.flush();
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
    Died(Death),
    /// Could not even spawn.
    SpawnFailed(String),
}

/// What a run that died left behind.
struct Death {
    /// The diagnosis, quoting ffmpeg's stderr. Goes in the track log and `error`.
    reason: String,
    /// This run's stderr ring, read after the drain thread was joined.
    stderr: Vec<String>,
    /// The run never produced media: no positive `out_time_us` on `-progress`
    /// and no segment file. For a screen that is `ddagrab` still waiting for its
    /// first frame.
    no_media: bool,
}

// ---------------------------------------------------------------------------
// Screen failure classification
// ---------------------------------------------------------------------------
//
// Root cause, 2026-08-01 (session 2026-08-01_10-30-12), from the track logs, the
// manifest and the System event log:
//
// - 13:45:44.983 Kernel-Power 566, MonitorReason 12, into session type 1: the
//   display-off idle timeout (900s on AC). 1.8s later the Intel-driven monitor
//   (screen-3, QSV) lost Desktop Duplication (`AcquireNextFrame failed:
//   887a0026`, DXGI_ERROR_ACCESS_LOST), and every relaunch was refused with
//   `Desktop duplication access denied` (E_ACCESSDENIED) while the display stayed
//   off. 20 restarts at 0.5–3s apart were spent in 68 seconds.
// - 14:02:19.869 a DisplayMonitor PnP device was deleted and 2s later re-created
//   (Intel Graphics Software log) with the displays still off: a monitor dropping
//   off the bus. The three NVIDIA screens lost access at 14:02:20. Their
//   relaunches did open, but `ddagrab` waits for a first frame that a dark
//   display never presents, silently, so each was killed by the 30s progress
//   watchdog. 20 x ~33s ran out at 14:13:14.
// - 15:20:35.673 Kernel-Power 566, MonitorReason 31 (keyboard): display back on.
//   Every screen had given up 67+ minutes earlier; camera and audio ran to 19:02.
//
// The same display-off/on pair (566 with reasons 12 and 31) brackets the screen-3
// failures of 2026-07-31, 2026-08-13 and 2026-08-14 to within 1.4–3.4s. Outages
// lasted 2m17s to 1h35m; the restart budget lasted 68s. So: wait outages out
// instead of counting them, and say what happened in words.

/// Why a screen cannot be captured right now, when it is a condition that ends
/// on its own. A track in one of these waits, retrying forever; see
/// [`outage_backoff_ms`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenFault {
    /// `Desktop duplication access denied` (E_ACCESSDENIED): the secure desktop
    /// (lock screen, UAC, Ctrl+Alt+Del) or a powered-down display. Every one in
    /// the logs falls inside a display-off period.
    AccessDenied,
    /// `ddagrab` opened but never delivered a first frame, and said nothing. A
    /// display that is off presents no frames.
    NoFrames,
    /// DXGI_ERROR_ACCESS_LOST (`887a0026`, or the `DDA ReleaseFrame failed!` it
    /// also causes): the display slept, the desktop switched, the mode changed,
    /// or a monitor dropped off the bus and came back. The trigger, rarely the
    /// state: the attempts after it say more, see [`Outage::record`].
    AccessLost,
    /// The monitor is not among the attached outputs (`Failed to enumerate DXGI
    /// output N`, or [`relocate_screen`] found nothing). Seen for ~1s while a
    /// monitor re-links on wake, or for as long as one is unplugged.
    MonitorMissing,
    /// DXGI_ERROR_DEVICE_REMOVED / _HUNG / _RESET: a driver crash (TDR) or update.
    DriverReset,
    /// The GPU refused memory (`Cannot allocate memory`, `Failed creating
    /// internal buffer texture`, a QSV encoder that fails to initialise). Seen at
    /// the start of session 2026-10-10_11-00-45, cleared after 4 restarts.
    GpuBusy,
}

impl ScreenFault {
    /// The gap reason and the widget's line: a sentence, not stderr.
    pub fn human(self) -> &'static str {
        match self {
            ScreenFault::AccessDenied | ScreenFault::NoFrames => "Display was locked or asleep",
            ScreenFault::AccessLost => "Display slept, locked or changed mode",
            ScreenFault::MonitorMissing => "Monitor disconnected or switched off",
            ScreenFault::DriverReset => "Graphics driver restarted",
            ScreenFault::GpuBusy => "Graphics card ran out of memory",
        }
    }
}

/// Why a screen can never be captured again in this session. A track with one of
/// these stops for good, at once, rather than spending restarts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatalFault {
    /// `Unknown encoder` / `Encoder not found`.
    EncoderMissing,
    /// `No such filter` / `Filter not found`: no `ddagrab` in this FFmpeg.
    CaptureMissing,
    /// `Unrecognized option` / `Option not found`.
    FfmpegTooOld,
    /// The monitor came back on another adapter; see [`Relocation::OtherAdapter`].
    MovedAdapter,
}

impl FatalFault {
    pub fn human(self) -> &'static str {
        match self {
            FatalFault::EncoderMissing => "This FFmpeg has no encoder for this screen",
            FatalFault::CaptureMissing => "This FFmpeg cannot capture screens",
            FatalFault::FfmpegTooOld => "This FFmpeg is too old for screen recording",
            FatalFault::MovedAdapter => "Monitor moved to a different graphics card",
        }
    }
}

/// How a screen track's failed run is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenFailure {
    /// Capture is impossible right now and will be possible again. Wait it out,
    /// however long it lasts; never counts toward [`MAX_RESTARTS`].
    Outage(ScreenFault),
    /// Will never work this session. Hard-fail now.
    Fatal(FatalFault),
    /// Nothing we recognise. The bounded restart budget applies, as for every
    /// other kind of track.
    Unknown,
}

/// Human line for a screen whose failure is [`ScreenFailure::Unknown`].
const SCREEN_UNKNOWN_HUMAN: &str = "Screen capture stopped unexpectedly";

/// Read a screen run's death. `stderr` is its whole ring; `no_media` is
/// [`Death::no_media`]. Pure, and tested against the real stderr of the sessions
/// above.
///
/// Fatal markers win, then the outage markers (most specific first), then a
/// silent run that never produced media. Lines that only follow from an outage
/// (`Could not open encoder before EOF`, `-22 Invalid argument` after an access
/// denial) are deliberately not markers of anything.
pub fn classify_screen_failure(stderr: &[String], no_media: bool) -> ScreenFailure {
    let has = |needle: &str| stderr.iter().any(|l| l.contains(needle));

    if has("Unknown encoder") || has("Encoder not found") {
        return ScreenFailure::Fatal(FatalFault::EncoderMissing);
    }
    if has("No such filter") || has("Filter not found") {
        return ScreenFailure::Fatal(FatalFault::CaptureMissing);
    }
    if has("Unrecognized option") || has("Option not found") {
        return ScreenFailure::Fatal(FatalFault::FfmpegTooOld);
    }

    if has("Failed to enumerate DXGI output") {
        return ScreenFailure::Outage(ScreenFault::MonitorMissing);
    }
    if has("Desktop duplication access denied") {
        return ScreenFailure::Outage(ScreenFault::AccessDenied);
    }
    // DXGI_ERROR_DEVICE_REMOVED, _HUNG, _RESET.
    if has("887a0005") || has("887a0006") || has("887a0007") {
        return ScreenFailure::Outage(ScreenFault::DriverReset);
    }
    if has("Cannot allocate memory")
        || has("Failed creating internal buffer texture")
        || stderr
            .iter()
            .any(|l| l.contains("h264_qsv") && l.contains("Error initializing the encoder"))
    {
        return ScreenFailure::Outage(ScreenFault::GpuBusy);
    }
    if has("887a0026") || has("DDA ReleaseFrame failed") {
        return ScreenFailure::Outage(ScreenFault::AccessLost);
    }

    if no_media && stderr.iter().all(|l| is_benign_stderr(l)) {
        return ScreenFailure::Outage(ScreenFault::NoFrames);
    }
    ScreenFailure::Unknown
}

/// Delay before the next attempt, `attempt` being 1 for the first failure of an
/// outage: 1, 2, 4, 8, 16, then 30s for as long as the session runs. There is no
/// last attempt.
pub fn outage_backoff_ms(attempt: u32) -> u64 {
    let doublings = attempt.saturating_sub(1).min(5);
    (1_000u64 << doublings).min(OUTAGE_BACKOFF_CAP_MS)
}

/// The wait before the next outage attempt, given how long the user has been
/// idle (`None` when it cannot be read). [`outage_backoff_ms`], except that once
/// it has reached its cap and nobody has touched the machine for
/// [`AWAY_IDLE_MS`], attempts drop to one per [`AWAY_RETRY_MS`]: a dark display
/// stays dark until someone comes back, and coming back cuts the wait short
/// anyway. The first ~30s keep the fast ramp, because a mode change during a
/// film nobody is touching the mouse for is also an outage, and a short one.
pub fn outage_delay_ms(attempt: u32, idle_ms: Option<i64>) -> u64 {
    let backoff = outage_backoff_ms(attempt);
    match idle_ms {
        Some(idle) if idle >= AWAY_IDLE_MS && backoff >= OUTAGE_BACKOFF_CAP_MS => AWAY_RETRY_MS,
        _ => backoff,
    }
}

/// Whether input just ended a stretch of the user being away. One-shot per
/// return: `away` latches once there has been no input for [`WAKE_IDLE_MS`] and
/// is cleared by the input that ends it, so someone typing through an outage
/// (a fullscreen game holding the display) never triggers a wake on every
/// keystroke.
pub fn user_came_back(away: &mut bool, now: i64, last_input: Option<i64>) -> bool {
    let Some(last_input) = last_input else {
        return false;
    };
    if now - last_input >= WAKE_IDLE_MS {
        *away = true;
        false
    } else if *away {
        *away = false;
        true
    } else {
        false
    }
}

/// How long a screen run may take to show its first frame, given how long the
/// user has been idle (`None` when it cannot be read).
pub fn screen_first_frame_ms(idle_ms: Option<i64>) -> i64 {
    match idle_ms {
        Some(idle) if idle >= WAKE_IDLE_MS => SCREEN_FIRST_FRAME_IDLE_MS,
        _ => SCREEN_FIRST_FRAME_ACTIVE_MS,
    }
}

/// A screen waiting for capture to become possible again.
#[derive(Debug, Clone)]
struct Outage {
    /// When the track stopped recording.
    since: i64,
    /// What the gap and the widget say.
    fault: ScreenFault,
    /// Attempts since the outage began, or since the user came back. Drives the
    /// backoff.
    attempts: u32,
    /// See [`user_came_back`].
    away: bool,
}

impl Outage {
    fn new(since: i64, fault: ScreenFault) -> Self {
        Self {
            since,
            fault,
            attempts: 1,
            away: false,
        }
    }

    /// Count an attempt that failed with `fault`. A later fault replaces the
    /// earlier one, except `AccessLost`, which is how almost every outage starts
    /// and is never the most useful thing to say once a relaunch has failed for a
    /// more specific reason.
    fn record(&mut self, fault: ScreenFault) {
        if fault != ScreenFault::AccessLost {
            self.fault = fault;
        }
        self.attempts += 1;
    }
}

/// What a supervisor does after a failed run. See [`Supervision::failed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Stop the track for good, now.
    Fatal(FatalFault),
    /// Wait out [`Supervision::outage`]; `began` when this failure started it.
    Wait { began: bool },
    /// Relaunch after a short backoff. The restart has been counted.
    Restart,
    /// Out of restarts: stop the track with its raw error.
    GiveUp,
}

/// What to do before relaunching a screen. See [`Supervision::plan_relaunch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelaunchPlan {
    /// Launch at this output index, or wherever it was last (`None`).
    Launch(Option<u32>),
    /// Nothing matched for [`MISSING_PROBE_EVERY`] relaunches: try one launch
    /// at the session-start index anyway.
    Probe(u32),
    /// Launch nothing and wait. `Some(adapter)` while the monitor shows up on
    /// another adapter, not yet [`MOVED_ADAPTER_CONFIRMATIONS`] times running.
    Wait(Option<u32>),
    /// The monitor is settled on this other adapter. Stop for good.
    MovedAdapter(u32),
}

/// Where a screen's monitor is, as one DXGI enumeration sees it. See
/// [`locate_screen`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Located {
    found: Relocation,
    home_idx: u32,
    /// See [`home_is_anothers`].
    home_taken: bool,
}

/// A supervisor's decisions between runs, kept apart from the processes so
/// the state machine can be tested without ffmpeg or displays.
#[derive(Debug, Default)]
struct Supervision {
    /// Consecutive failures counted against [`MAX_RESTARTS`].
    restarts: u32,
    /// Set while a screen waits out an outage. See [`ScreenFailure::Outage`].
    outage: Option<Outage>,
    /// The track has footage from this session. Until it does, outages spend
    /// the restart budget like any other failure.
    recorded: bool,
    /// Consecutive relocations that found no monitor.
    missing: u32,
    /// Consecutive relocations that found it on another adapter.
    other_adapter: u32,
}

impl Supervision {
    fn new(recorded: bool) -> Self {
        Self {
            recorded,
            ..Default::default()
        }
    }

    /// A run ended. One that recorded proves the track works: the next failure
    /// starts a fresh count and a fresh outage. Without this a long session's
    /// unrelated hiccups accumulated until the track was abandoned.
    fn ran(&mut self, produced: bool) {
        if produced {
            self.restarts = 0;
            self.outage = None;
            self.recorded = true;
        }
    }

    /// Whether an unrecognised failure is read as the outage already under way.
    /// Once a screen is waiting one out, a relaunch that fails in a way we do
    /// not recognise is still the outage: a dark or locked display can fail in
    /// more ways than the logs have shown us. Only fatal markers or footage end
    /// the wait.
    fn promotes(&self, class: ScreenFailure, is_screen: bool) -> bool {
        is_screen && class == ScreenFailure::Unknown && self.outage.is_some()
    }

    /// Decide what a failed run of class `class` leads to, at `now`.
    fn failed(&mut self, class: ScreenFailure, is_screen: bool, now: i64) -> Verdict {
        let class = match &self.outage {
            Some(o) if self.promotes(class, is_screen) => ScreenFailure::Outage(o.fault),
            _ => class,
        };
        match class {
            ScreenFailure::Fatal(fault) => Verdict::Fatal(fault),
            ScreenFailure::Outage(fault) => {
                // Before any footage an outage may be no outage at all: a QSV
                // encoder that can never open reads as the GPU running out of
                // memory, every time.
                if !self.recorded && self.spend_restart() {
                    return Verdict::GiveUp;
                }
                let began = self.outage.is_none();
                match &mut self.outage {
                    Some(o) => o.record(fault),
                    None => self.outage = Some(Outage::new(now, fault)),
                }
                Verdict::Wait { began }
            }
            ScreenFailure::Unknown => {
                if self.spend_restart() {
                    Verdict::GiveUp
                } else {
                    Verdict::Restart
                }
            }
        }
    }

    /// Count a restart; `true` once the budget is spent.
    fn spend_restart(&mut self) -> bool {
        self.restarts += 1;
        self.restarts > MAX_RESTARTS
    }

    /// What to launch, given where the monitor is now (`None`: not a screen, or
    /// DXGI could not be enumerated, so the old index is tried again).
    fn plan_relaunch(&mut self, located: Option<Located>) -> RelaunchPlan {
        let Some(at) = located else {
            self.missing = 0;
            self.other_adapter = 0;
            return RelaunchPlan::Launch(None);
        };
        match at.found {
            Relocation::Found(idx) => {
                self.missing = 0;
                self.other_adapter = 0;
                RelaunchPlan::Launch(Some(idx))
            }
            Relocation::OtherAdapter(adapter) => {
                self.missing = 0;
                self.other_adapter += 1;
                if self.other_adapter >= MOVED_ADAPTER_CONFIRMATIONS {
                    RelaunchPlan::MovedAdapter(adapter)
                } else {
                    RelaunchPlan::Wait(Some(adapter))
                }
            }
            Relocation::Missing => {
                self.other_adapter = 0;
                self.missing += 1;
                if self.missing < MISSING_PROBE_EVERY {
                    return RelaunchPlan::Wait(None);
                }
                self.missing = 0;
                if at.home_taken {
                    RelaunchPlan::Wait(None)
                } else {
                    RelaunchPlan::Probe(at.home_idx)
                }
            }
        }
    }
}

/// A track's live recovery state, for the widget. Every field is `None` while
/// the track records.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct TrackHealth {
    /// One sentence on why the track is not recording: "Display was locked or
    /// asleep". The raw stderr stays in `logs/<track>.log` and `Track::error`.
    pub last_error_human: Option<String>,
    /// Set while the track waits out an outage it expects to end. It keeps
    /// retrying until capture works again or the session stops.
    pub waiting_since_epoch_ms: Option<i64>,
    /// When the next attempt is due, while restarting or waiting.
    pub next_attempt_epoch_ms: Option<i64>,
}

/// One output's open downtime gap: where it began and what it is called.
#[derive(Debug, Clone)]
struct Downtime {
    from: i64,
    reason: String,
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
    /// The gap covering the time this output has been down, open from its first
    /// failure until footage resumes. See [`note_downtime`].
    downtime: Option<Downtime>,
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
            downtime: None,
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

/// Run one track (or a webcam+mic pair) for the whole session, relaunching
/// ffmpeg whenever it dies. How long it keeps trying depends on why it died:
///
/// - A screen in an outage ([`ScreenFailure::Outage`]) waits it out, retrying on
///   [`outage_delay_ms`] until capture works again or the session stops. Until
///   the track has recorded, those attempts also spend [`MAX_RESTARTS`].
/// - A fatal failure ([`ScreenFailure::Fatal`]) stops the track at once.
/// - Anything else gets [`MAX_RESTARTS`] consecutive attempts.
///
/// Either way the time it is down becomes one gap with a human reason (see
/// [`note_downtime`]), and [`TrackHealth`] says what it is waiting for. The
/// decisions themselves live in [`Supervision`].
fn supervise_track(inner: Arc<Inner>, spec: TrackSpec) {
    let outs: Vec<OutputState> = spec
        .outputs
        .iter()
        .map(|o| OutputState::new(&inner.session_dir, o))
        .collect();
    supervise_outputs(inner, spec, outs, false);
}

/// [`supervise_track`] after a resume: each output that had recorded starts
/// down, with the pause as its open downtime, so its first segment closes the
/// `paused` gap where its content begins and no drift gap lands on top.
fn supervise_resumed(inner: Arc<Inner>, spec: TrackSpec, edges: &[(String, i64)]) {
    let mut outs: Vec<OutputState> = spec
        .outputs
        .iter()
        .map(|o| OutputState::new(&inner.session_dir, o))
        .collect();
    for o in outs.iter_mut() {
        if let Some((_, from)) = edges.iter().find(|(t, _)| *t == o.track_id) {
            o.downtime = Some(Downtime {
                from: *from,
                reason: PAUSED_GAP_REASON.to_string(),
            });
        }
    }
    // The outputs may have been renumbered while paused: find the monitor
    // before the first launch, from the session-start identity.
    supervise_outputs(inner, spec, outs, true);
}

/// [`supervise_track`] with its outputs already built, so a microphone split
/// out of the webcam pair keeps the open downtime and idle tail it had inside
/// it. See [`split_pair`].
///
/// `relocate_first`: a supervisor that starts mid-session (resume, Restart
/// screens) cannot trust the session-start output index, so a screen locates
/// its monitor before its first launch as it would before a relaunch.
fn supervise_outputs(inner: Arc<Inner>, spec: TrackSpec, mut outs: Vec<OutputState>, relocate_first: bool) {
    // The source can change under us: a shared webcam+mic process that cannot run
    // gives up the pairing and continues as a solo capture (see `split_pair`), and
    // a screen follows its monitor to a new output index (see `relocate_screen`).
    let mut source = spec.source.clone();
    // A track with footage from before this supervisor (a split-out mic) has
    // already shown it works.
    let recorded = outs.iter().any(|o| track_end(&inner, &o.track_id).is_some());
    let mut sup = Supervision::new(recorded);
    let mut relaunch = relocate_first;

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

        // A screen finds its monitor again before every relaunch: the events that
        // kill screen captures are the same ones that renumber, rename and remove
        // DXGI outputs.
        if relaunch {
            match sup.plan_relaunch(locate_screen(&inner, &source)) {
                RelaunchPlan::Launch(Some(idx)) => point_screen_at(&inner, &spec.id, &mut source, idx),
                RelaunchPlan::Launch(None) => {}
                RelaunchPlan::Probe(idx) => {
                    note_track_log(
                        &inner,
                        &spec.id,
                        &format!(
                            "monitor still not matched after {} looks; trying one launch \
                             at its session-start output {}",
                            MISSING_PROBE_EVERY, idx
                        ),
                    );
                    point_screen_at(&inner, &spec.id, &mut source, idx);
                }
                RelaunchPlan::Wait(seen_on) => {
                    // Nothing to launch into, so launch nothing and wait.
                    let line = match seen_on {
                        Some(a) => format!(
                            "monitor seen on adapter {}; not launching until that holds or \
                             it comes back",
                            a
                        ),
                        None => "monitor not among the attached DXGI outputs; not launching"
                            .to_string(),
                    };
                    let class = ScreenFailure::Outage(ScreenFault::MonitorMissing);
                    let verdict = sup.failed(class, true, now_ms());
                    if !after_failure(&inner, &spec, &mut source, &mut outs, &mut sup, verdict, &line) {
                        return;
                    }
                    if inner.stopping() {
                        break;
                    }
                    continue;
                }
                RelaunchPlan::MovedAdapter(adapter) => {
                    let detail = format!("the monitor is now on adapter {}", adapter);
                    fail_for_good(&inner, &spec.id, &mut outs, FatalFault::MovedAdapter, &detail);
                    return;
                }
            }
        }
        relaunch = true;

        // A waiting track stays Restarting until footage actually appears
        // (`run_track_once` flips it then); anything else counts from launch.
        if sup.outage.is_none() {
            set_all_status(&inner, &outs, TrackStatus::Recording, None);
        }

        let outcome = run_track_once(&inner, &spec, &source, &mut outs);
        sup.ran(outs.iter().any(|o| o.produced > 0));

        let (reason, death) = match outcome {
            RunOutcome::Finished => break,
            RunOutcome::Died(d) => (d.reason.clone(), Some(d)),
            RunOutcome::SpawnFailed(r) => (r, None),
        };
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

        let is_screen = matches!(source, TrackSource::Screen { .. });
        let class = match &death {
            Some(d) if is_screen => classify_screen_failure(&d.stderr, d.no_media),
            _ => ScreenFailure::Unknown,
        };
        if sup.promotes(class, is_screen) {
            note_track_log(&inner, &spec.id, "not recognised; still waiting out the outage");
        }
        let verdict = sup.failed(class, is_screen, now_ms());
        if !after_failure(&inner, &spec, &mut source, &mut outs, &mut sup, verdict, &reason) {
            return;
        }
    }

    // A track still down when the session stops was down until the end.
    let now = now_ms();
    let pausing = inner.pausing.load(Ordering::SeqCst);
    for o in outs.iter_mut() {
        if let Some(d) = o.downtime.take() {
            upsert_downtime_gap(&inner, &o.track_id, &d, now);
        }
        // Footage just dropped for inactivity keeps its own name, and the
        // pause begins where it ends (`pause_edges`).
        if pausing {
            if let Some(t) = o.idle_tail.take() {
                record_gap(&inner, &o.track_id, t.from, t.to, IDLE_GAP_REASON);
            }
        }
    }
    inner.clear_health(&outs);
    set_all_status(&inner, &outs, TrackStatus::Stopped, None);
    inner.flush();
}

/// Carry out a [`Verdict`]: wait, back off, or stop the track. `reason` is the
/// raw failure. `false` when the track is finished for good (the caller
/// returns); `true` to go round again, also when the session is stopping, which
/// the caller checks.
fn after_failure(
    inner: &Arc<Inner>,
    spec: &TrackSpec,
    source: &mut TrackSource,
    outs: &mut Vec<OutputState>,
    sup: &mut Supervision,
    verdict: Verdict,
    reason: &str,
) -> bool {
    let is_screen = matches!(source, TrackSource::Screen { .. });
    match verdict {
        Verdict::Fatal(fault) => {
            fail_for_good(inner, &spec.id, outs, fault, reason);
            false
        }
        Verdict::Wait { began } => {
            wait_in_outage(inner, &spec.id, outs, sup, began, reason);
            true
        }
        Verdict::GiveUp => {
            give_up(inner, &spec.id, outs, sup, is_screen, reason);
            false
        }
        Verdict::Restart => {
            let now = now_ms();
            // A screen's gap gets a sentence. The other tracks' gaps keep
            // ffmpeg's own words, as before: their failures are not classified.
            let (human, gap_reason) = if is_screen {
                (SCREEN_UNKNOWN_HUMAN, SCREEN_UNKNOWN_HUMAN.to_string())
            } else {
                ("Stopped unexpectedly; restarting", reason.to_string())
            };
            for o in outs.iter_mut() {
                note_downtime(inner, o, now, &gap_reason);
            }
            inner.warn(format!(
                "track {} died ({}), restart {}/{}",
                spec.id, reason, sup.restarts, MAX_RESTARTS
            ));
            set_all_status(inner, outs, TrackStatus::Restarting, Some(reason.to_string()));
            for o in outs.iter() {
                bump_restarts(inner, &o.track_id);
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
                if let Some(solo) = split_pair(inner, source, outs, reason) {
                    *source = solo;
                    for o in outs.iter_mut() {
                        o.barren_runs = 0;
                    }
                    sup.restarts = 0;
                }
            }
            // Back off a little so a permanently missing device doesn't spin.
            let backoff = (500 * sup.restarts.min(6) as u64).max(500);
            inner.set_health(
                outs,
                TrackHealth {
                    last_error_human: Some(human.to_string()),
                    waiting_since_epoch_ms: None,
                    next_attempt_epoch_ms: Some(now + backoff as i64),
                },
            );
            inner.flush();
            sleep_unless_stopping(inner, backoff);
            true
        }
    }
}

/// Publish and wait out the outage `sup` is in. `began` when this failure
/// started it, which is the only time it counts as a restart in the manifest:
/// a ten-hour display-off is one restart, not twelve hundred.
fn wait_in_outage(
    inner: &Arc<Inner>,
    id: &str,
    outs: &mut [OutputState],
    sup: &mut Supervision,
    began: bool,
    reason: &str,
) {
    let recorded = sup.recorded;
    let Some(o) = sup.outage.as_mut() else {
        return;
    };
    if began {
        let how_long = if recorded {
            "for as long as the session runs".to_string()
        } else {
            format!("up to {} times, as it has not recorded yet", MAX_RESTARTS)
        };
        inner.warn(format!(
            "{}: {}. Waiting for capture to work again, retrying every {}s or sooner \
             (every {}s while nobody is at the machine) {} ({})",
            id,
            o.fault.human(),
            OUTAGE_BACKOFF_CAP_MS / 1000,
            AWAY_RETRY_MS / 1000,
            how_long,
            reason
        ));
        for out in outs.iter() {
            bump_restarts(inner, &out.track_id);
        }
    }
    let now = now_ms();
    let delay = outage_delay_ms(o.attempts, poll_last_input_ms().map(|t| now - t));
    note_track_log(
        inner,
        id,
        &format!(
            "read as an outage: {} (attempt {}, next in {}s): {}",
            o.fault.human(),
            o.attempts,
            delay / 1000,
            reason
        ),
    );
    wait_out_outage(inner, outs, o, delay, Some(reason.to_string()));
}

/// Stop a track whose restart budget is spent, with its raw error.
fn give_up(
    inner: &Arc<Inner>,
    id: &str,
    outs: &mut [OutputState],
    sup: &Supervision,
    is_screen: bool,
    reason: &str,
) {
    let now = now_ms();
    let gap_reason = match (&sup.outage, is_screen) {
        (Some(o), _) => o.fault.human().to_string(),
        (None, true) => SCREEN_UNKNOWN_HUMAN.to_string(),
        (None, false) => reason.to_string(),
    };
    for o in outs.iter_mut() {
        note_downtime(inner, o, now, &gap_reason);
    }
    inner.warn(format!("track {} gave up after {} restarts ({})", id, MAX_RESTARTS, reason));
    let gave_up = if is_screen {
        "Screen capture kept failing and was stopped"
    } else {
        "Kept failing and was stopped"
    };
    inner.set_health(
        outs,
        TrackHealth {
            last_error_human: Some(gave_up.to_string()),
            ..Default::default()
        },
    );
    set_all_status(
        inner,
        outs,
        TrackStatus::Failed,
        Some(format!("gave up after {} restarts: {}", MAX_RESTARTS, reason)),
    );
    inner.flush();
}

/// Find a screen's monitor in a fresh DXGI enumeration. `None` for any other
/// track, or when DXGI cannot be enumerated at all, in which case the old index
/// is simply tried again.
fn locate_screen(inner: &Inner, source: &TrackSource) -> Option<Located> {
    if !matches!(source, TrackSource::Screen { .. }) {
        return None;
    }
    let outputs = enumerate_dxgi_outputs().ok().filter(|o| !o.is_empty())?;
    locate_in(source, &inner.screen_identities, &outputs)
}

/// [`locate_screen`] against a given enumeration. Pure, for the tests.
fn locate_in(source: &TrackSource, others: &[ScreenIdentity], outputs: &[DisplayOutput]) -> Option<Located> {
    let TrackSource::Screen {
        adapter,
        home_idx,
        identity,
        ..
    } = source
    else {
        return None;
    };
    Some(Located {
        found: relocate_screen(identity, *adapter, *home_idx, others, outputs),
        home_idx: *home_idx,
        home_taken: home_is_anothers(identity, *adapter, *home_idx, others, outputs),
    })
}

/// Point a screen track at DXGI output `idx`, saying so when that is a move.
fn point_screen_at(inner: &Arc<Inner>, id: &str, source: &mut TrackSource, idx: u32) {
    let TrackSource::Screen {
        output_idx,
        identity,
        ..
    } = source
    else {
        return;
    };
    if idx != *output_idx {
        inner.warn(format!(
            "{}: the monitor that was {} at session start is now DXGI output {} (was {}); \
             the outputs were renumbered and the track follows its monitor",
            id, identity.device_name, idx, output_idx
        ));
        *output_idx = idx;
    }
    // A supervisor started mid-session holds the session-start index, while the
    // manifest may still name where the last one followed the monitor to.
    let mut m = inner.manifest.lock().unwrap();
    if let Some(mon) = m.track_mut(id).and_then(|t| t.monitor.as_mut()) {
        mon.output_idx = idx;
    }
}

/// How a wait between attempts ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    Due,
    /// Input after the user had been away: the display is waking.
    UserBack,
    Stopping,
}

/// Sleep up to `ms`, ending early (after [`WAKE_SETTLE_MS`]) when the user comes
/// back. Almost every outage that kills a screen ends with a keypress, so this is
/// what makes recovery take seconds rather than up to a full backoff step.
fn wait_for_retry(inner: &Arc<Inner>, ms: u64, away: &mut bool) -> Wake {
    let deadline = std::time::Instant::now() + Duration::from_millis(ms);
    let asked = inner.retry_asked.load(Ordering::SeqCst);
    while std::time::Instant::now() < deadline {
        if inner.stopping() {
            return Wake::Stopping;
        }
        // "Restart screens": the user says the display is back, so retry now
        // and start the schedule over, as when they come back to the desk.
        if inner.retry_asked.load(Ordering::SeqCst) != asked {
            return Wake::UserBack;
        }
        if user_came_back(away, now_ms(), poll_last_input_ms()) {
            return if sleep_unless_stopping(inner, WAKE_SETTLE_MS) {
                Wake::UserBack
            } else {
                Wake::Stopping
            };
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if inner.stopping() {
        Wake::Stopping
    } else {
        Wake::Due
    }
}

/// Stretch the downtime gap, publish the waiting state, then wait `delay` ms
/// for the next attempt. `error` replaces the track's raw `error` when there is
/// a new one. `false` when the session is stopping.
fn wait_out_outage(
    inner: &Arc<Inner>,
    outs: &mut [OutputState],
    o: &mut Outage,
    delay: u64,
    error: Option<String>,
) -> bool {
    let now = now_ms();
    let human = o.fault.human();
    for out in outs.iter_mut() {
        note_downtime(inner, out, now, human);
    }
    set_all_status(inner, outs, TrackStatus::Restarting, error);
    inner.set_health(
        outs,
        TrackHealth {
            last_error_human: Some(human.to_string()),
            waiting_since_epoch_ms: Some(o.since),
            next_attempt_epoch_ms: Some(now + delay as i64),
        },
    );
    inner.flush();
    match wait_for_retry(inner, delay, &mut o.away) {
        Wake::Stopping => false,
        Wake::UserBack => {
            // Start the schedule over: the display is coming back now.
            o.attempts = 0;
            true
        }
        Wake::Due => true,
    }
}

/// Stop a track for good on a failure that cannot clear up this session.
fn fail_for_good(
    inner: &Arc<Inner>,
    id: &str,
    outs: &mut [OutputState],
    fault: FatalFault,
    detail: &str,
) {
    let now = now_ms();
    for o in outs.iter_mut() {
        note_downtime(inner, o, now, fault.human());
    }
    inner.warn(format!("{} stopped for good: {} ({})", id, fault.human(), detail));
    note_track_log(inner, id, &format!("stopped for good: {}", fault.human()));
    inner.set_health(
        outs,
        TrackHealth {
            last_error_human: Some(fault.human().to_string()),
            ..Default::default()
        },
    );
    set_all_status(
        inner,
        outs,
        TrackStatus::Failed,
        Some(format!("{}: {}", fault.human(), detail)),
    );
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

    // The mic's state moves with it: an open downtime gap closes where its first
    // solo segment begins (rather than that segment also recording a "track
    // restarted" gap over the same hole), and an idle run still collapses into
    // one gap. Its health is cleared, as for any track that is launching.
    let mic_out = outs
        .iter()
        .position(|o| o.track_id == "mic")
        .map(|i| outs.remove(i));
    if let Some(m) = &mic_out {
        inner.clear_health(std::slice::from_ref(m));
    }

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
        .spawn(move || match mic_out {
            Some(out) => supervise_outputs(inner2, mic_spec, vec![out], false),
            None => supervise_track(inner2, mic_spec),
        })
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
    // Any positive `out_time_us` at all: media is flowing. A heartbeat line alone
    // is not proof — ffmpeg can report progress while its input is still opening.
    let media = Arc::new(AtomicBool::new(false));

    // stdout: `-progress pipe:1`. Doubles as the health heartbeat and, for the
    // first few seconds, as the content-start estimator (see `Anchor`).
    let stdout_join = child.stdout.take().map(|out| {
        let hb = last_progress.clone();
        let t0 = progress_t0.clone();
        let flowing = media.clone();
        let deadline = launched + ANCHOR_WINDOW_MS;
        std::thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                let Ok(l) = line else { break };
                let now = now_ms();
                hb.store(now, Ordering::Relaxed);
                // `out_time_us` only. ffmpeg's sibling `out_time_ms` key has
                // carried microseconds for years and is not worth trusting.
                let Some(us) = l
                    .strip_prefix("out_time_us=")
                    .and_then(|v| v.trim().parse::<i64>().ok())
                    .filter(|us| *us > 0)
                else {
                    continue;
                };
                flowing.store(true, Ordering::Relaxed);
                if now <= deadline {
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

    // A screen that never shows a first frame is not starting slowly: `ddagrab`
    // waits for one forever while the display is off, silently. Tighter while
    // the user is away, which is when that happens. See SCREEN_FIRST_FRAME_*.
    let first_frame_ms = matches!(source, TrackSource::Screen { .. })
        .then(|| screen_first_frame_ms(poll_last_input_ms().map(|t| launched - t)));
    // Any segment file seen in this run.
    let mut got_file = false;

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
        /// A screen showed no frame within this many seconds of launch.
        NoFirstFrame(i64),
        SysAudioStopped,
        Internal(String),
    }

    let exit: ExitKind;

    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));

        // 1. New segment files?
        let mut fresh = false;
        for (i, o) in outs.iter_mut().enumerate() {
            if let Some(seen_at) = poll_output(inner, o, &anchor) {
                last_new_segment[i] = seen_at;
                fresh = true;
            }
        }
        if fresh && !got_file {
            got_file = true;
            // Footage again. A track that was waiting or restarting is recording
            // from here, not from when the process launched.
            set_all_status(inner, outs, TrackStatus::Recording, None);
            inner.clear_health(outs);
            inner.flush();
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

        // 4b. First frame: a screen that has shown nothing at all yet.
        if let Some(limit) = first_frame_ms {
            if !got_file && !media.load(Ordering::Relaxed) && now - launched > limit {
                kill_and_reap(&mut child, &mut stdin);
                flush_outputs(inner, outs, &anchor);
                exit = ExitKind::NoFirstFrame(limit / 1000);
                break;
            }
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

    let stderr = inner.track_log(&spec.id);
    let reason = match exit {
        ExitKind::Finished => None,
        ExitKind::Exited(status) => {
            Some(format!("ffmpeg exited ({}): {}", status, diagnose(&stderr)))
        }
        ExitKind::NoSegments(track, secs) => Some(format!(
            "no new {} segment for {}s: {}",
            track,
            secs,
            diagnose(&stderr)
        )),
        ExitKind::ProgressStalled => Some(format!(
            "ffmpeg stopped reporting progress: {}",
            diagnose(&stderr)
        )),
        ExitKind::NoFirstFrame(secs) => Some(format!(
            "no picture from the display within {}s of launch: {}",
            secs,
            diagnose(&stderr)
        )),
        ExitKind::SysAudioStopped => Some(format!(
            "WASAPI loopback capture stopped: {}",
            diagnose(&stderr)
        )),
        ExitKind::Internal(e) => Some(e),
    };
    let outcome = match reason {
        None => RunOutcome::Finished,
        Some(reason) => RunOutcome::Died(Death {
            reason,
            stderr,
            no_media: !got_file && !media.load(Ordering::Relaxed),
        }),
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
        RunOutcome::Died(d) => d.reason.clone(),
        RunOutcome::SpawnFailed(r) => r.clone(),
    }
}

/// Append one line of the recorder's own to `<session>/logs/<track>.log`: how it
/// read the failure dumped just above. Best-effort, like [`dump_track_log`].
fn note_track_log(inner: &Arc<Inner>, track: &str, line: &str) {
    use std::io::Write as _;
    let dir = inner.session_dir.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{}.log", track)))
    {
        let _ = writeln!(f, "[chirptime] {}", line);
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
                    // Footage resumed after downtime, even though this piece of it
                    // is being dropped: the downtime gap ends here and the idle
                    // stretch takes over from the same instant.
                    if let Some(d) = o.downtime.take() {
                        upsert_downtime_gap(inner, &o.track_id, &d, observed);
                    }
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
        record_gap(inner, &o.track_id, t.from, t.to, IDLE_GAP_REASON);
        t.to
    });

    // Only the first segment of a run is placed by the anchor; the rest chain off
    // it, so a late-resolving estimate cannot shunt the track mid-run.
    let a = if o.anchor_pending { anchor.resolve() } else { None };
    o.anchor_pending = false;
    o.produced += 1;
    let resumed = o.downtime.take();
    index_segment(
        inner,
        &o.track_id,
        &o.dir,
        file,
        observed,
        a,
        o.segment_seconds,
        chain_override,
        resumed,
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
///
/// `resumed` is the downtime this segment ends, if any. It is closed where this
/// segment's content begins, and the chain is not consulted, so the restart
/// leaves one named gap rather than that plus a "dropped time" drift gap over
/// the same hole.
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
    resumed: Option<Downtime>,
) {
    let path = dir.join(file);
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let probe = match session::probe_segment(&inner.ffprobe, &path) {
        Ok(p) => p,
        Err(e) => {
            inner.warn(format!("{}/{} unreadable: {}", track_id, file, e));
            if let Some(d) = &resumed {
                upsert_downtime_gap(inner, track_id, d, observed);
            }
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

        let start = place_segment(track, observed, anchor, chain_override, resumed.as_ref());

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

/// Where a closed segment starts on `track`'s timeline, adding whatever gap
/// that leaves before it. Pure, for [`index_segment`].
///
/// After an idle run, `chain_override` carries the end of the deleted stretch
/// so the segment chains onto it (delta ≈ 0, no drift gap) rather than onto the
/// last *kept* segment before the hole. After downtime (`resumed`) there is
/// nothing to chain onto: the downtime gap already covers the hole, and it is
/// closed where this segment begins.
fn place_segment(
    track: &mut Track,
    observed: i64,
    anchor: Option<i64>,
    chain_override: Option<i64>,
    resumed: Option<&Downtime>,
) -> i64 {
    let chained = if resumed.is_some() {
        None
    } else {
        chain_override.or_else(|| {
            track
                .segments
                .last()
                .map(|s| s.start_epoch_ms + s.duration_ms)
        })
    };
    let decided = decide_segment_start(anchor, chained, observed);
    let start = decided.start_epoch_ms;
    if let Some(g) = decided.gap {
        track.gaps.push(g);
    }
    if let Some(d) = resumed {
        stretch_downtime_gap(&mut track.gaps, d, start);
    }
    start
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

fn track_end(inner: &Arc<Inner>, track_id: &str) -> Option<i64> {
    let m = inner.manifest.lock().unwrap();
    m.tracks
        .iter()
        .find(|t| t.id == track_id)
        .and_then(|t| t.end_epoch_ms())
}

/// Record an output's downtime as ONE gap: opened at its first failure,
/// stretched on every failed attempt, closed where footage resumes
/// ([`index_segment`]).
///
/// Before this, every failed restart pushed a gap of its own from the last
/// segment to "now": on 2026-08-01 each screen carried 21 nested gaps, all
/// starting at the same instant and each quoting raw stderr.
fn note_downtime(inner: &Arc<Inner>, o: &mut OutputState, now: i64, reason: &str) {
    let from = match &o.downtime {
        // The first failure after a resume ends the pause; the outage after it
        // is a gap of its own, not more of the pause.
        Some(d) if d.reason == PAUSED_GAP_REASON => {
            upsert_downtime_gap(inner, &o.track_id, d, now);
            now
        }
        Some(d) => d.from,
        None => {
            // Footage deleted for inactivity just before the failure closes as its
            // own idle gap first, so the downtime begins where that ends instead
            // of overlapping it.
            let start = match o.idle_tail.take() {
                Some(t) => {
                    record_gap(inner, &o.track_id, t.from, t.to, IDLE_GAP_REASON);
                    Some(t.to)
                }
                None => track_end(inner, &o.track_id),
            };
            // A track that never recorded has no hole between footage to describe.
            let Some(from) = start else {
                return;
            };
            from
        }
    };
    let d = Downtime {
        from,
        reason: reason.to_string(),
    };
    upsert_downtime_gap(inner, &o.track_id, &d, now);
    o.downtime = Some(d);
}

fn upsert_downtime_gap(inner: &Arc<Inner>, track_id: &str, d: &Downtime, to: i64) {
    let mut m = inner.manifest.lock().unwrap();
    if let Some(t) = m.track_mut(track_id) {
        stretch_downtime_gap(&mut t.gaps, d, to);
    }
}

/// Make the downtime gap `d` end at `to`, adding it if it is not there yet. While
/// a track is down nothing else is appended to its gaps, so an open downtime gap
/// is always the last one. One that would end where it begins is removed rather
/// than written.
fn stretch_downtime_gap(gaps: &mut Vec<Gap>, d: &Downtime, to: i64) {
    let open = gaps
        .last()
        .is_some_and(|g| g.from_epoch_ms == d.from && g.reason != IDLE_GAP_REASON);
    if to <= d.from {
        if open {
            gaps.pop();
        }
        return;
    }
    match gaps.last_mut() {
        Some(g) if open => {
            g.to_epoch_ms = to;
            g.reason = d.reason.clone();
        }
        _ => gaps.push(Gap {
            from_epoch_ms: d.from,
            to_epoch_ms: to,
            reason: d.reason.clone(),
        }),
    }
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
            ..
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

    // -- screen failure classification ----------------------------------------
    //
    // Every ring below is copied from a real `logs/screen-*.log`, except the
    // fatal ones, which are this machine's FFmpeg 7.1.1 run with a bad encoder,
    // filter or option.

    fn ring(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    /// 2026-08-01 13:45:46.804, screen-3 (Intel, QSV): the display-off timeout
    /// fired 1.8s earlier.
    fn igpu_access_lost() -> Vec<String> {
        ring(&[
            "[Parsed_ddagrab_0 @ 000001e7a80c2600] AcquireNextFrame failed: 887a0026",
            "[Parsed_ddagrab_0 @ 000001e7a80c2600] EOF timestamp not reliable",
            "[fc#0 @ 000001e7a9a88800] Error requesting a frame from the filtergraph: Generic error in an external library",
            "[fc#0 @ 000001e7a9a88800] Error sending frames to consumers: Generic error in an external library",
            "[fc#0 @ 000001e7a9a88800] Task finished with error code: -542398533 (Generic error in an external library)",
            "[fc#0 @ 000001e7a9a88800] Terminating thread with return code -542398533 (Generic error in an external library)",
        ])
    }

    /// 2026-08-01 13:45:48.123 and the 19 relaunches after it, display still off.
    fn igpu_access_denied() -> Vec<String> {
        ring(&[
            "[Parsed_ddagrab_0 @ 000002098d472600] Desktop duplication access denied",
            "[Parsed_ddagrab_0 @ 000002098d472600] Failed to configure output pad on Parsed_ddagrab_0",
            "[fc#0 @ 000002098edc8800] Error configuring filter graph: Operation not permitted",
            "[fc#0 @ 000002098edc8800] Task finished with error code: -1 (Operation not permitted)",
            "[fc#0 @ 000002098edc8800] Terminating thread with return code -1 (Operation not permitted)",
            "[vost#0:0/h264_qsv @ 000002098edf6940] Could not open encoder before EOF",
            "[vost#0:0/h264_qsv @ 000002098edf6940] Task finished with error code: -22 (Invalid argument)",
            "[vost#0:0/h264_qsv @ 000002098edf6940] Terminating thread with return code -22 (Invalid argument)",
            "[out#0/segment @ 000002098edc8900] Nothing was written into output file, because at least one of its streams received no packets.",
        ])
    }

    #[test]
    fn the_igpu_screen_losing_access_at_display_off_is_an_outage() {
        assert_eq!(
            classify_screen_failure(&igpu_access_lost(), false),
            ScreenFailure::Outage(ScreenFault::AccessLost)
        );
    }

    #[test]
    fn access_denied_while_the_display_is_off_is_waited_out_not_fatal() {
        // The ring also says "Could not open encoder" and "Invalid argument". Those
        // follow from the denial; reading them as a broken encoder would end the
        // track for good at the first display sleep.
        assert_eq!(
            classify_screen_failure(&igpu_access_denied(), true),
            ScreenFailure::Outage(ScreenFault::AccessDenied)
        );
    }

    #[test]
    fn the_nvidia_screens_losing_access_at_the_monitor_relink_is_an_outage() {
        // 2026-08-01 14:02:20.955, screen-0, 1s after a DisplayMonitor PnP delete.
        let r = ring(&[
            "[Parsed_ddagrab_0 @ 000001500f4ee440] AcquireNextFrame failed: 887a0026",
            "[Parsed_ddagrab_0 @ 000001500f4ee440] EOF timestamp not reliable",
            "[in#0/lavfi @ 000001500f4e9940] Error during demuxing: Generic error in an external library",
        ]);
        assert_eq!(
            classify_screen_failure(&r, false),
            ScreenFailure::Outage(ScreenFault::AccessLost)
        );
    }

    #[test]
    fn a_silent_relaunch_that_never_shows_a_frame_is_a_dark_display() {
        // 2026-08-01 14:02:51 onward, screens 0-2: twenty relaunches, each killed
        // with "ffmpeg stopped reporting progress: no stderr output".
        assert_eq!(
            classify_screen_failure(&[], true),
            ScreenFailure::Outage(ScreenFault::NoFrames)
        );
        let notices_only = ring(&["[swscaler @ 0000] deprecated pixel format used, make sure you did set range correctly"]);
        assert_eq!(
            classify_screen_failure(&notices_only, true),
            ScreenFailure::Outage(ScreenFault::NoFrames)
        );
    }

    #[test]
    fn a_silent_death_after_recording_is_not_assumed_to_be_the_display() {
        // Footage flowed, then the process vanished: an external kill looks like
        // this. Nothing says "display", so the bounded budget applies.
        assert_eq!(classify_screen_failure(&[], false), ScreenFailure::Unknown);
    }

    #[test]
    fn a_monitor_mid_relink_is_reported_as_missing() {
        // 2026-08-18 20:09:27.021, screen-2, right after an ACCESS_LOST.
        let r = ring(&[
            "[Parsed_ddagrab_0 @ 0000017d5ddad940] Failed to enumerate DXGI output 2",
            "[Parsed_ddagrab_0 @ 0000017d5ddad940] Failed to configure output pad on Parsed_ddagrab_0",
            "[in#0 @ 0000017d5dda90c0] Error opening input: Generic error in an external library",
            "Error opening input file ddagrab=output_idx=2:framerate=30:draw_mouse=1.",
            "Error opening input files: Generic error in an external library",
        ]);
        assert_eq!(
            classify_screen_failure(&r, true),
            ScreenFailure::Outage(ScreenFault::MonitorMissing)
        );
    }

    #[test]
    fn a_failed_frame_release_is_the_same_access_loss() {
        // 2026-07-31 20:03:16.798, screen-2.
        let r = ring(&[
            "[Parsed_ddagrab_0 @ 000002041a69e440] DDA ReleaseFrame failed!",
            "[Parsed_ddagrab_0 @ 000002041a69e440] EOF timestamp not reliable",
            "[in#0/lavfi @ 000002041a699940] Error during demuxing: Generic error in an external library",
        ]);
        assert_eq!(
            classify_screen_failure(&r, false),
            ScreenFailure::Outage(ScreenFault::AccessLost)
        );
    }

    #[test]
    fn the_igpu_running_out_of_memory_at_start_is_retried_not_fatal() {
        // 2026-10-10 11:00:58 to 11:01:03, screen-3: four different spellings of
        // the same thing, then it recorded.
        let encode = ring(&[
            "[h264_qsv @ 00000273c3866ec0] Error allocating the output packet",
            "[vost#0:0/h264_qsv @ 00000273c3866a80] Error submitting video frame to the encoder",
            "[vost#0:0/h264_qsv @ 00000273c3866a80] Error encoding a frame: Cannot allocate memory",
            "[vost#0:0/h264_qsv @ 00000273c3866a80] Task finished with error code: -12 (Cannot allocate memory)",
            "[vost#0:0/h264_qsv @ 00000273c3866a80] Terminating thread with return code -12 (Cannot allocate memory)",
        ]);
        let texture = ring(&[
            "[Parsed_ddagrab_0 @ 0000017b305525c0] Failed creating internal buffer texture.",
            "[Parsed_ddagrab_0 @ 0000017b305525c0] Failed to configure output pad on Parsed_ddagrab_0",
            "[fc#0 @ 0000017b31e988c0] Error configuring filter graph: Cannot allocate memory",
            "[vost#0:0/h264_qsv @ 0000017b31ec2a80] Could not open encoder before EOF",
            "[out#0/segment @ 0000017b31ebb000] Nothing was written into output file, because at least one of its streams received no packets.",
        ]);
        let init = ring(&[
            "[h264_qsv @ 000001fc0b292ec0] Error initializing the encoder: unknown error (-1)",
            "[vost#0:0/h264_qsv @ 000001fc0b292a80] Error while opening encoder - maybe incorrect parameters such as bit_rate, rate, width or height.",
            "[fc#0 @ 000001fc0b2688c0] Error sending frames to consumers: Unknown error occurred",
            "[vost#0:0/h264_qsv @ 000001fc0b292a80] Could not open encoder before EOF",
            "[out#0/segment @ 000001fc0b28b000] Nothing was written into output file, because at least one of its streams received no packets.",
        ]);
        for r in [encode, texture, init] {
            assert_eq!(
                classify_screen_failure(&r, true),
                ScreenFailure::Outage(ScreenFault::GpuBusy),
                "{r:?}"
            );
        }
    }

    #[test]
    fn a_missing_encoder_filter_or_option_is_fatal_at_once() {
        let encoder = ring(&[
            "[vost#0:0 @ 0000022f1c2a4f40] Unknown encoder 'nosuch_enc'",
            "[vost#0:0 @ 0000022f1c2a4f40] Error selecting an encoder",
            "Error opening output file seg.mp4.",
            "Error opening output files: Encoder not found",
        ]);
        let filter = ring(&[
            "[AVFilterGraph @ 000001d1f8c5a3c0] No such filter: 'nosuchgrab'",
            "[in#0 @ 000001d1f8c4b2c0] Error opening input: Filter not found",
            "Error opening input files: Filter not found",
        ]);
        let option = ring(&[
            "Unrecognized option 'nosuch_opt'.",
            "Error splitting the argument list: Option not found",
        ]);
        assert_eq!(
            classify_screen_failure(&encoder, true),
            ScreenFailure::Fatal(FatalFault::EncoderMissing)
        );
        assert_eq!(
            classify_screen_failure(&filter, true),
            ScreenFailure::Fatal(FatalFault::CaptureMissing)
        );
        assert_eq!(
            classify_screen_failure(&option, true),
            ScreenFailure::Fatal(FatalFault::FfmpegTooOld)
        );
    }

    #[test]
    fn a_fatal_marker_wins_over_an_outage_in_the_same_ring() {
        let mut r = igpu_access_denied();
        r.push("Error opening output files: Encoder not found".to_string());
        assert_eq!(
            classify_screen_failure(&r, true),
            ScreenFailure::Fatal(FatalFault::EncoderMissing)
        );
    }

    #[test]
    fn a_busy_camera_is_not_mistaken_for_a_display_outage() {
        // 2026-08-01 21:18:49.044, webcam.
        let r = ring(&[
            "[dshow @ 000001cd18f2a2c0] Could not run graph (sometimes caused by a device already in use by other application)",
            "[in#0 @ 000001cd18f2a0c0] Error opening input: I/O error",
            "Error opening input files: I/O error",
        ]);
        assert_eq!(classify_screen_failure(&r, true), ScreenFailure::Unknown);
    }

    #[test]
    fn every_reason_reads_as_a_sentence_not_stderr() {
        let outage = [
            ScreenFault::AccessDenied,
            ScreenFault::NoFrames,
            ScreenFault::AccessLost,
            ScreenFault::MonitorMissing,
            ScreenFault::DriverReset,
            ScreenFault::GpuBusy,
        ]
        .map(|f| f.human());
        let fatal = [
            FatalFault::EncoderMissing,
            FatalFault::CaptureMissing,
            FatalFault::FfmpegTooOld,
            FatalFault::MovedAdapter,
        ]
        .map(|f| f.human());
        for h in outage.iter().chain(fatal.iter()).chain([&SCREEN_UNKNOWN_HUMAN]) {
            assert!(h.chars().next().is_some_and(|c| c.is_ascii_uppercase()), "{h}");
            for raw in ["@", "0x", "887a", "[", "ffmpeg", "DXGI", "ddagrab"] {
                assert!(!h.contains(raw), "{h:?} quotes {raw:?}");
            }
            assert!(!h.ends_with('.'), "{h:?}");
        }
        assert_eq!(ScreenFault::AccessDenied.human(), "Display was locked or asleep");
        assert_eq!(ScreenFault::MonitorMissing.human(), "Monitor disconnected or switched off");
    }

    #[test]
    fn the_first_of_aug_reads_as_locked_or_asleep_not_access_lost() {
        // What screen-3 saw on 2026-08-01, in order: the loss, then denials.
        let mut o = Outage::new(0, ScreenFault::AccessLost);
        for _ in 0..20 {
            o.record(ScreenFault::AccessDenied);
        }
        assert_eq!(o.fault, ScreenFault::AccessDenied);
        assert_eq!(o.attempts, 21);
        // A later access loss (the monitor re-linking on wake) does not replace
        // the better explanation.
        o.record(ScreenFault::AccessLost);
        assert_eq!(o.fault.human(), "Display was locked or asleep");
        // Anything more specific does.
        o.record(ScreenFault::MonitorMissing);
        assert_eq!(o.fault, ScreenFault::MonitorMissing);
    }

    // -- outage backoff and wake -----------------------------------------------

    #[test]
    fn the_outage_backoff_doubles_to_thirty_seconds_and_holds_forever() {
        let schedule: Vec<u64> = (1..=9).map(outage_backoff_ms).collect();
        assert_eq!(
            schedule,
            [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000, 30_000]
        );
        assert_eq!(outage_backoff_ms(0), 1_000, "a reset schedule starts at 1s");
        assert_eq!(
            outage_backoff_ms(u32::MAX),
            OUTAGE_BACKOFF_CAP_MS,
            "no overflow, no last attempt"
        );
    }

    #[test]
    fn the_longest_outage_on_record_is_waited_out() {
        // 2026-08-01 screen-3: down 13:45:46.8, display back 15:20:35.7. The old
        // budget (20 restarts, 0.5-3s apart) ran out after 68s. The new schedule
        // is still trying, at most 30s apart, when the display comes back.
        let at = |h: u64, m: u64, s: u64, ms: u64| ((h * 60 + m) * 60 + s) * 1000 + ms;
        let outage_ms = at(15, 20, 35, 700) - at(13, 45, 46, 800);
        let mut elapsed = 0;
        let mut attempt = 1;
        while elapsed < outage_ms {
            let wait = outage_backoff_ms(attempt);
            assert!(wait <= OUTAGE_BACKOFF_CAP_MS);
            elapsed += wait;
            attempt += 1;
        }
        assert!(attempt > MAX_RESTARTS, "outage attempts never count toward the budget");
    }

    #[test]
    fn coming_back_is_input_after_being_away() {
        let mut away = false;
        let now = 1_000_000;
        // Unknown input time: never a wake.
        assert!(!user_came_back(&mut away, now, None));
        // Idle long enough: away, not back.
        assert!(!user_came_back(&mut away, now, Some(now - WAKE_IDLE_MS)));
        assert!(away);
        assert!(!user_came_back(&mut away, now + 5_000, Some(now - WAKE_IDLE_MS)));
        // A keypress: back, once.
        assert!(user_came_back(&mut away, now + 6_000, Some(now + 5_900)));
        assert!(!away);
        assert!(!user_came_back(&mut away, now + 6_250, Some(now + 6_200)));
    }

    #[test]
    fn typing_through_an_outage_never_counts_as_coming_back() {
        // A fullscreen app holding the display while the user plays.
        let mut away = false;
        for t in (0..120_000).step_by(250) {
            assert!(!user_came_back(&mut away, t, Some(t - 200)));
        }
    }

    #[test]
    fn a_screen_gets_less_time_for_its_first_frame_while_the_user_is_away() {
        assert_eq!(screen_first_frame_ms(None), SCREEN_FIRST_FRAME_ACTIVE_MS);
        assert_eq!(screen_first_frame_ms(Some(0)), SCREEN_FIRST_FRAME_ACTIVE_MS);
        assert_eq!(screen_first_frame_ms(Some(WAKE_IDLE_MS - 1)), SCREEN_FIRST_FRAME_ACTIVE_MS);
        assert_eq!(screen_first_frame_ms(Some(WAKE_IDLE_MS)), SCREEN_FIRST_FRAME_IDLE_MS);
        assert_eq!(screen_first_frame_ms(Some(3_600_000)), SCREEN_FIRST_FRAME_IDLE_MS);
        // Both are well inside the 30s progress watchdog, which used to be the
        // only thing that ended a dark relaunch.
        assert!(SCREEN_FIRST_FRAME_ACTIVE_MS < PROGRESS_STALL_MS);
    }

    // -- relocating a screen ---------------------------------------------------

    fn identity_of(o: &DisplayOutput) -> ScreenIdentity {
        ScreenIdentity {
            device_name: o.device_name.clone(),
            x: o.x,
            y: o.y,
            w: o.w,
            h: o.h,
        }
    }

    fn identities(outs: &[DisplayOutput]) -> Vec<ScreenIdentity> {
        outs.iter().map(identity_of).collect()
    }

    /// Find `before[i]` in `after`, every screen of `before` being the session's.
    fn relocate(before: &[DisplayOutput], i: usize, after: &[DisplayOutput]) -> Relocation {
        let b = &before[i];
        relocate_screen(&identity_of(b), b.adapter, b.output_idx, &identities(before), after)
    }

    #[test]
    fn an_unchanged_layout_finds_every_screen_where_it_was() {
        let outs = real_outputs();
        for (i, o) in outs.iter().enumerate() {
            assert_eq!(
                relocate(&outs, i, &outs),
                Relocation::Found(o.output_idx),
                "{}",
                o.device_name
            );
        }
    }

    #[test]
    fn a_renumbered_output_is_followed_to_its_new_index() {
        let before = real_outputs();
        let mut after = real_outputs();
        after[0].output_idx = 1;
        after[1].output_idx = 0;
        assert_eq!(
            relocate(&before, 1, &after),
            Relocation::Found(0)
        );
    }

    #[test]
    fn a_reconnected_monitor_is_found_under_its_new_name() {
        let before = real_outputs();
        let mut after = real_outputs();
        after[0].device_name = r"\\.\DISPLAY11".to_string();
        assert_eq!(
            relocate(&before, 0, &after),
            Relocation::Found(0)
        );
    }

    #[test]
    fn a_resolution_change_keeps_the_screen() {
        let before = real_outputs();
        let mut after = real_outputs();
        (after[0].w, after[0].h) = (3840, 2160);
        assert_eq!(
            relocate(&before, 0, &after),
            Relocation::Found(0)
        );
    }

    #[test]
    fn an_unplugged_or_detached_monitor_is_missing() {
        let before = real_outputs();
        let unplugged: Vec<DisplayOutput> = real_outputs().into_iter().take(2).collect();
        assert_eq!(
            relocate(&before, 2, &unplugged),
            Relocation::Missing
        );
        let mut detached = real_outputs();
        detached[2].attached = false;
        assert_eq!(
            relocate(&before, 2, &detached),
            Relocation::Missing
        );
        assert_eq!(relocate(&before, 2, &[]), Relocation::Missing);
    }

    #[test]
    fn a_monitor_on_another_adapter_cannot_be_followed() {
        let before = real_outputs();
        let mut after = real_outputs();
        after[3].adapter = 0;
        after[3].output_idx = 3;
        assert_eq!(
            relocate(&before, 3, &after),
            Relocation::OtherAdapter(0)
        );
    }

    #[test]
    fn a_stranger_on_the_old_rectangle_elsewhere_does_not_end_the_track() {
        // screen-2's monitor is gone, and an unrelated monitor on the iGPU now sits
        // where it was. That is a missing monitor, not a moved one.
        let before = real_outputs();
        let mut after: Vec<DisplayOutput> = real_outputs().into_iter().take(2).collect();
        let mut stranger = before[3].clone();
        (stranger.x, stranger.y, stranger.w, stranger.h) = (-3840, -15, 3840, 2160);
        after.push(stranger);
        assert_eq!(
            relocate(&before, 2, &after),
            Relocation::Missing
        );
    }

    // -- downtime gaps ---------------------------------------------------------

    fn down(from: i64, reason: &str) -> Downtime {
        Downtime {
            from,
            reason: reason.to_string(),
        }
    }

    #[test]
    fn twenty_failed_restarts_leave_one_gap_not_twenty_one() {
        // 2026-08-01 screen-0 carried 21 gaps, all starting at its last segment.
        let mut gaps = Vec::new();
        let d = down(1_000, ScreenFault::AccessLost.human());
        for i in 1..=21 {
            stretch_downtime_gap(&mut gaps, &d, 1_000 + i * 33_000);
        }
        let d = down(1_000, ScreenFault::NoFrames.human());
        stretch_downtime_gap(&mut gaps, &d, 900_000);
        assert_eq!(
            gaps,
            [Gap {
                from_epoch_ms: 1_000,
                to_epoch_ms: 900_000,
                reason: "Display was locked or asleep".to_string(),
            }]
        );
    }

    #[test]
    fn a_downtime_gap_closes_where_footage_resumes() {
        let mut gaps = Vec::new();
        let d = down(1_000, "Display was locked or asleep");
        stretch_downtime_gap(&mut gaps, &d, 60_000);
        // The resumed segment's content began a little before the last failure
        // was stamped: the gap shrinks to it.
        stretch_downtime_gap(&mut gaps, &d, 58_500);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].to_epoch_ms, 58_500);
    }

    #[test]
    fn an_idle_gap_ending_where_downtime_begins_is_left_alone() {
        let mut gaps = vec![Gap {
            from_epoch_ms: 0,
            to_epoch_ms: 1_000,
            reason: IDLE_GAP_REASON.to_string(),
        }];
        let d = down(1_000, "Display was locked or asleep");
        stretch_downtime_gap(&mut gaps, &d, 5_000);
        assert_eq!(gaps.len(), 2);
        assert_eq!(gaps[0].reason, IDLE_GAP_REASON);
        assert_eq!((gaps[1].from_epoch_ms, gaps[1].to_epoch_ms), (1_000, 5_000));
    }

    #[test]
    fn a_downtime_that_ends_where_it_began_leaves_no_gap() {
        let mut gaps = Vec::new();
        let d = down(1_000, "Display was locked or asleep");
        stretch_downtime_gap(&mut gaps, &d, 1_000);
        assert!(gaps.is_empty());
        stretch_downtime_gap(&mut gaps, &d, 2_000);
        stretch_downtime_gap(&mut gaps, &d, 900);
        assert!(gaps.is_empty(), "footage from before the failure erases the open gap");
    }

    // -- two same-size monitors trading names ----------------------------------

    /// `o` under another device name and output index. In `real_outputs`,
    /// screen-1 (DISPLAY2, right of the ultrawide) and screen-2 (DISPLAY3, left
    /// of it) are both 3840x2160 on adapter 0.
    fn renamed(mut o: DisplayOutput, name: &str, idx: u32) -> DisplayOutput {
        o.device_name = name.to_string();
        o.output_idx = idx;
        o
    }

    #[test]
    fn two_same_size_monitors_trading_names_are_never_recorded_twice() {
        let before = real_outputs();
        let (right, left) = (1, 2);

        // A clean swap of names, and of names and indices: each follows its own
        // rectangle.
        let names_only = vec![
            before[0].clone(),
            renamed(before[right].clone(), r"\\.\DISPLAY3", 1),
            renamed(before[left].clone(), r"\\.\DISPLAY2", 2),
            before[3].clone(),
        ];
        assert_eq!(relocate(&before, right, &names_only), Relocation::Found(1));
        assert_eq!(relocate(&before, left, &names_only), Relocation::Found(2));
        let both = vec![
            before[0].clone(),
            renamed(before[right].clone(), r"\\.\DISPLAY3", 2),
            renamed(before[left].clone(), r"\\.\DISPLAY2", 1),
            before[3].clone(),
        ];
        assert_eq!(relocate(&before, right, &both), Relocation::Found(2));
        assert_eq!(relocate(&before, left, &both), Relocation::Found(1));

        // Mid-settle: the left monitor is back first, under the right one's old
        // name. That is the left monitor, so the right screen waits instead of
        // recording it (the old name-only fallback took it, and then kept it).
        let settling = vec![
            before[0].clone(),
            renamed(before[left].clone(), r"\\.\DISPLAY2", 1),
            before[3].clone(),
        ];
        assert_eq!(relocate(&before, right, &settling), Relocation::Missing);
        assert_eq!(relocate(&before, left, &settling), Relocation::Found(1));

        // Then the right one arrives under the left one's old name. Both find
        // their own, because both still match against how the session began.
        let settled = vec![
            before[0].clone(),
            renamed(before[left].clone(), r"\\.\DISPLAY2", 1),
            renamed(before[right].clone(), r"\\.\DISPLAY3", 2),
            before[3].clone(),
        ];
        assert_eq!(relocate(&before, right, &settled), Relocation::Found(2));
        assert_eq!(relocate(&before, left, &settled), Relocation::Found(1));
    }

    #[test]
    fn another_screens_exact_identity_is_never_a_candidate() {
        // Mirrored or mid-reconfigure: screen-1's rectangle also holds an output
        // that is exactly screen-2 as the session began. It is not screen-1's.
        let before = real_outputs();
        let mut sessions = identities(&before);
        sessions[2].x = 5120; // screen-2 began on screen-1's rectangle, mirrored
        let impostor = DisplayOutput {
            device_name: r"\\.\DISPLAY3".to_string(),
            output_idx: 1,
            ..before[1].clone()
        };
        let after = vec![before[0].clone(), impostor];
        let me = identity_of(&before[1]);
        assert_eq!(relocate_screen(&me, 0, 1, &sessions, &after), Relocation::Missing);
    }

    #[test]
    fn a_tie_goes_to_the_session_start_index() {
        // Two outputs on screen-1's rectangle under unknown names (a clone).
        let before = real_outputs();
        let after = vec![
            before[0].clone(),
            renamed(before[1].clone(), r"\\.\DISPLAY8", 4),
            renamed(before[1].clone(), r"\\.\DISPLAY7", 1),
        ];
        assert_eq!(relocate(&before, 1, &after), Relocation::Found(1));
        // Without the home index among them, the first one.
        let after = vec![
            renamed(before[1].clone(), r"\\.\DISPLAY8", 4),
            renamed(before[1].clone(), r"\\.\DISPLAY7", 5),
        ];
        assert_eq!(relocate(&before, 1, &after), Relocation::Found(4));
    }

    #[test]
    fn the_home_index_is_off_limits_only_when_another_screen_sits_there() {
        let before = real_outputs();
        let ids = identities(&before);
        let me = identity_of(&before[1]);
        // screen-2's monitor took output 1, under any name.
        let taken = vec![renamed(before[2].clone(), r"\\.\DISPLAY9", 1)];
        assert!(home_is_anothers(&me, 0, 1, &ids, &taken));
        // A stranger there, or nothing there, may be tried.
        let mut stranger = renamed(before[1].clone(), r"\\.\DISPLAY9", 1);
        stranger.x = 9000;
        assert!(!home_is_anothers(&me, 0, 1, &ids, &[stranger]));
        assert!(!home_is_anothers(&me, 0, 1, &ids, &[]));
    }

    // -- the supervisor's state machine ------------------------------------------

    const LOST: ScreenFailure = ScreenFailure::Outage(ScreenFault::AccessLost);
    const DENIED: ScreenFailure = ScreenFailure::Outage(ScreenFault::AccessDenied);
    const GPU: ScreenFailure = ScreenFailure::Outage(ScreenFault::GpuBusy);

    #[test]
    fn an_unrecognised_failure_during_an_outage_is_the_outage() {
        let mut sup = Supervision::new(true);
        assert!(!sup.promotes(ScreenFailure::Unknown, true), "no outage yet");
        assert_eq!(sup.failed(DENIED, true, 1_000), Verdict::Wait { began: true });
        assert!(sup.promotes(ScreenFailure::Unknown, true));
        assert_eq!(sup.failed(ScreenFailure::Unknown, true, 2_000), Verdict::Wait { began: false });
        let o = sup.outage.as_ref().map(|o| (o.fault, o.attempts, o.since));
        assert_eq!(o, Some((ScreenFault::AccessDenied, 2, 1_000)));
        assert_eq!(sup.restarts, 0, "waited out, not spent");
        // Fatal still ends it, and non-screens are never promoted.
        let fatal = ScreenFailure::Fatal(FatalFault::EncoderMissing);
        assert_eq!(sup.failed(fatal, true, 3_000), Verdict::Fatal(FatalFault::EncoderMissing));
        assert!(!sup.promotes(ScreenFailure::Unknown, false));
        assert_eq!(sup.failed(ScreenFailure::Unknown, false, 3_000), Verdict::Restart);
    }

    #[test]
    fn footage_ends_the_outage_and_the_next_one_starts_fresh() {
        let mut sup = Supervision::new(true);
        for _ in 0..5 {
            sup.failed(DENIED, true, 1_000);
        }
        sup.ran(false);
        assert_eq!(sup.outage.as_ref().map(|o| o.attempts), Some(5), "nothing recorded, still waiting");
        sup.ran(true);
        assert!(sup.outage.is_none());
        assert_eq!(sup.restarts, 0);
        assert_eq!(sup.failed(LOST, true, 9_000), Verdict::Wait { began: true });
        let o = sup.outage.as_ref().map(|o| (o.attempts, o.since));
        assert_eq!(o, Some((1, 9_000)));
    }

    #[test]
    fn a_screen_that_has_recorded_waits_out_any_outage_forever() {
        // Ten hours at the 120s away spacing is 300 attempts; go well past it.
        let mut sup = Supervision::new(true);
        let mut began = 0;
        for i in 0..5_000 {
            match sup.failed(if i % 2 == 0 { DENIED } else { LOST }, true, i) {
                Verdict::Wait { began: b } => began += b as u32,
                v => panic!("attempt {i}: {v:?}"),
            }
        }
        assert_eq!(began, 1, "one outage, so one restart in the manifest");
    }

    #[test]
    fn an_encoder_that_never_opens_fails_with_its_raw_error_instead_of_waiting() {
        // A QSV encoder that cannot initialise reads as GpuBusy every time. Before
        // any footage that spends the budget like any other failure.
        let mut sup = Supervision::new(false);
        for i in 1..=MAX_RESTARTS {
            assert_eq!(
                sup.failed(GPU, true, i as i64),
                Verdict::Wait { began: i == 1 },
                "attempt {i}"
            );
        }
        assert_eq!(sup.failed(GPU, true, 99), Verdict::GiveUp);
        // Footage once is enough to make outages free again.
        let mut sup = Supervision::new(false);
        sup.failed(GPU, true, 0);
        sup.ran(true);
        for i in 0..(MAX_RESTARTS * 5) {
            assert!(matches!(sup.failed(GPU, true, i as i64), Verdict::Wait { .. }));
        }
    }

    #[test]
    fn unrecognised_failures_spend_the_budget() {
        let mut sup = Supervision::new(true);
        for _ in 0..MAX_RESTARTS {
            assert_eq!(sup.failed(ScreenFailure::Unknown, false, 0), Verdict::Restart);
        }
        assert_eq!(sup.failed(ScreenFailure::Unknown, false, 0), Verdict::GiveUp);
    }

    fn at(found: Relocation, home_taken: bool) -> Option<Located> {
        Some(Located {
            found,
            home_idx: 2,
            home_taken,
        })
    }

    #[test]
    fn a_missing_monitor_launches_nothing_but_probes_its_home_every_tenth_look() {
        let mut sup = Supervision::new(true);
        for round in 0..3 {
            for look in 1..MISSING_PROBE_EVERY {
                assert_eq!(
                    sup.plan_relaunch(at(Relocation::Missing, false)),
                    RelaunchPlan::Wait(None),
                    "round {round}, look {look}"
                );
            }
            assert_eq!(sup.plan_relaunch(at(Relocation::Missing, false)), RelaunchPlan::Probe(2));
        }
        // Another screen's monitor at home: no probe, keep waiting.
        for _ in 1..MISSING_PROBE_EVERY {
            sup.plan_relaunch(at(Relocation::Missing, true));
        }
        assert_eq!(sup.plan_relaunch(at(Relocation::Missing, true)), RelaunchPlan::Wait(None));
        // Finding it resets the count.
        for _ in 1..MISSING_PROBE_EVERY {
            sup.plan_relaunch(at(Relocation::Missing, false));
        }
        assert_eq!(sup.plan_relaunch(at(Relocation::Found(0), false)), RelaunchPlan::Launch(Some(0)));
        assert_eq!(sup.plan_relaunch(at(Relocation::Missing, false)), RelaunchPlan::Wait(None));
        // Not a screen, or no enumeration: launch where it was.
        assert_eq!(sup.plan_relaunch(None), RelaunchPlan::Launch(None));
    }

    #[test]
    fn a_missing_monitor_waits_as_an_outage_without_a_launch() {
        // The no-launch path feeds the same outage as a failed run would.
        let mut sup = Supervision::new(true);
        assert_eq!(sup.plan_relaunch(at(Relocation::Missing, false)), RelaunchPlan::Wait(None));
        let missing = ScreenFailure::Outage(ScreenFault::MonitorMissing);
        assert_eq!(sup.failed(missing, true, 5), Verdict::Wait { began: true });
        assert_eq!(sup.failed(missing, true, 6), Verdict::Wait { began: false });
        assert_eq!(sup.outage.as_ref().map(|o| o.fault), Some(ScreenFault::MonitorMissing));
    }

    #[test]
    fn a_monitor_on_another_adapter_must_stay_there_three_looks_running() {
        let mut sup = Supervision::new(true);
        let other = at(Relocation::OtherAdapter(1), false);
        assert_eq!(sup.plan_relaunch(other), RelaunchPlan::Wait(Some(1)));
        assert_eq!(sup.plan_relaunch(other), RelaunchPlan::Wait(Some(1)));
        // Interrupted: start again.
        assert_eq!(sup.plan_relaunch(at(Relocation::Missing, false)), RelaunchPlan::Wait(None));
        assert_eq!(sup.plan_relaunch(other), RelaunchPlan::Wait(Some(1)));
        assert_eq!(sup.plan_relaunch(other), RelaunchPlan::Wait(Some(1)));
        assert_eq!(sup.plan_relaunch(other), RelaunchPlan::MovedAdapter(1));
    }

    #[test]
    fn nobody_at_the_machine_means_one_attempt_every_two_minutes() {
        let away = Some(AWAY_IDLE_MS);
        // The ramp is untouched, so a short outage nobody is touching the mouse
        // through (a film changing mode) still recovers in seconds.
        for attempt in 1..=5 {
            assert_eq!(outage_delay_ms(attempt, away), outage_backoff_ms(attempt));
        }
        assert_eq!(outage_delay_ms(6, away), AWAY_RETRY_MS);
        assert_eq!(outage_delay_ms(600, Some(36_000_000)), AWAY_RETRY_MS);
        // Someone there, or no reading: the normal cap.
        assert_eq!(outage_delay_ms(600, Some(AWAY_IDLE_MS - 1)), OUTAGE_BACKOFF_CAP_MS);
        assert_eq!(outage_delay_ms(600, None), OUTAGE_BACKOFF_CAP_MS);
        // A display-off night of 10h is 300 attempts, not 1200.
        let night: u64 = 10 * 3_600_000;
        assert_eq!(night / AWAY_RETRY_MS, 300);
    }

    // -- waiting, against a throwaway session directory ------------------------

    /// An `Inner` with one screen track, writing its manifest under the temp
    /// dir. Nothing here launches ffmpeg or touches a display.
    fn test_inner(tag: &str) -> Arc<Inner> {
        let dir = std::env::temp_dir().join(format!("chirptime-{}-{}", tag, std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("screen-0"));
        let mut manifest = SessionManifest::new("test".into(), 0, 60, Rect::default());
        manifest
            .tracks
            .push(new_track("screen-0", "screen", "test", None, "h264", 0, 0, 30));
        Arc::new(Inner {
            session_dir: dir,
            segment_seconds: 60,
            ffmpeg: "unused".into(),
            ffprobe: "unused".into(),
            manifest: Mutex::new(manifest),
            stop: AtomicBool::new(false),
            pausing: AtomicBool::new(false),
            retry_asked: AtomicI64::new(0),
            live_supervisors: Mutex::new(Default::default()),
            logs: Mutex::new(Default::default()),
            warnings: Mutex::new(Vec::new()),
            fatal: Mutex::new(None),
            extra_joins: Mutex::new(Vec::new()),
            last_input_ms: AtomicI64::new(0),
            drop_idle_after_ms: 0,
            health: Mutex::new(Default::default()),
            screen_identities: Vec::new(),
        })
    }

    fn screen_out(inner: &Inner) -> OutputState {
        OutputState::new(
            &inner.session_dir,
            &OutputSpec {
                track_id: "screen-0".into(),
                dir_name: "screen-0".into(),
                segment_seconds: 60,
            },
        )
    }

    #[test]
    fn stop_during_an_outage_wait_returns_promptly() {
        let inner = test_inner("stopwait");
        let waiter = {
            let inner = inner.clone();
            std::thread::spawn(move || {
                let mut outs = vec![screen_out(&inner)];
                let mut sup = Supervision::new(true);
                let v = sup.failed(DENIED, true, now_ms());
                let started = std::time::Instant::now();
                wait_in_outage(&inner, "screen-0", &mut outs, &mut sup, v == Verdict::Wait { began: true }, "test");
                // And the plain wait, the longest one there is.
                let o = sup.outage.as_mut().expect("waiting");
                let more = wait_out_outage(&inner, &mut outs, o, AWAY_RETRY_MS, None);
                (started.elapsed(), more)
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        // Waiting is published while it lasts.
        let health = inner.health.lock().unwrap().get("screen-0").cloned();
        assert!(health.is_some_and(|h| h.waiting_since_epoch_ms.is_some()));
        inner.stop.store(true, Ordering::SeqCst);
        let (took, more) = waiter.join().expect("waiter");
        assert!(!more, "a stopping session ends the wait");
        assert!(took < Duration::from_secs(6), "took {took:?}");
        assert_eq!(inner.manifest.lock().unwrap().tracks[0].restarts, 1);
        let _ = std::fs::remove_dir_all(&inner.session_dir);
    }

    // -- placing the segment that ends downtime ----------------------------------

    fn track_down_since(end: i64, until: i64, reason: &str) -> (Track, Downtime) {
        let mut t = new_track("mic", "mic", "test", None, "aac", 0, 0, 0);
        t.segments.push(Segment {
            file: "a.mp4".into(),
            start_epoch_ms: 0,
            duration_ms: end,
            bytes: 1,
        });
        let d = down(end, reason);
        stretch_downtime_gap(&mut t.gaps, &d, until);
        (t, d)
    }

    #[test]
    fn the_segment_that_ends_downtime_closes_its_gap_and_adds_none() {
        // Anchored (a dshow track): the gap ends where the content began.
        let (mut t, d) = track_down_since(10_000, 50_000, "Stopped unexpectedly; restarting");
        let start = place_segment(&mut t, 49_500, Some(48_000), None, Some(&d));
        assert_eq!(start, 48_000);
        assert_eq!(t.gaps.len(), 1, "{:?}", t.gaps);
        assert_eq!((t.gaps[0].from_epoch_ms, t.gaps[0].to_epoch_ms), (10_000, 48_000));
        assert_eq!(t.gaps[0].reason, "Stopped unexpectedly; restarting");

        // Unanchored (a screen): at the file's own time, never chained across.
        let (mut t, d) = track_down_since(10_000, 50_000, "Display was locked or asleep");
        assert_eq!(place_segment(&mut t, 51_000, None, None, Some(&d)), 51_000);
        assert_eq!(t.gaps.len(), 1);
        assert_eq!(t.gaps[0].to_epoch_ms, 51_000);

        // What a split-out mic did before it kept its state: no downtime to
        // resume, so the anchor laid a second gap over the open one.
        let (mut t, _) = track_down_since(10_000, 50_000, "Stopped unexpectedly; restarting");
        place_segment(&mut t, 49_500, Some(48_000), None, None);
        assert_eq!(t.gaps.len(), 2);
        assert_eq!(t.gaps[1].reason, "track restarted");
    }

    // -- pause and resume --------------------------------------------------------

    fn seg(start: i64, dur: i64) -> Segment {
        Segment {
            file: format!("{}.mp4", start),
            start_epoch_ms: start,
            duration_ms: dur,
            bytes: 1,
        }
    }

    #[test]
    fn a_pause_is_one_paused_gap_per_recorded_track_from_its_last_edge() {
        let mut m = SessionManifest::new("p".into(), 0, 60, Rect::default());
        // Footage to 10 s, then an idle stretch that ran into the pause.
        let mut a = new_track("screen-0", "screen", "a", None, "h264", 0, 0, 30);
        a.segments.push(seg(0, 10_000));
        a.gaps.push(Gap {
            from_epoch_ms: 10_000,
            to_epoch_ms: 70_000,
            reason: IDLE_GAP_REASON.into(),
        });
        let mut b = new_track("mic", "mic", "b", None, "aac", 0, 0, 0);
        b.segments.push(seg(0, 12_000));
        // A camera that never opened has no hole to describe.
        let c = new_track("webcam", "webcam", "c", None, "h264", 0, 0, 30);
        m.tracks.extend([a, b, c]);

        let edges = close_pause(&mut m, 100_000);
        assert_eq!(edges, vec![("screen-0".to_string(), 70_000), ("mic".to_string(), 12_000)]);
        let last = |i: usize| m.tracks[i].gaps.last().cloned();
        let g = last(0).unwrap();
        assert_eq!((g.from_epoch_ms, g.to_epoch_ms, g.reason.as_str()), (70_000, 100_000, PAUSED_GAP_REASON));
        assert_eq!(m.tracks[0].gaps.len(), 2, "the idle stretch keeps its own name");
        let g = last(1).unwrap();
        assert_eq!((g.from_epoch_ms, g.to_epoch_ms), (12_000, 100_000));
        assert!(m.tracks[2].gaps.is_empty());
    }

    #[test]
    fn the_first_segment_after_resume_closes_the_paused_gap_with_no_drift_gap() {
        let mut t = new_track("screen-0", "screen", "a", None, "h264", 0, 0, 30);
        t.segments.push(seg(0, 12_000));
        let d = down(12_000, PAUSED_GAP_REASON);
        stretch_downtime_gap(&mut t.gaps, &d, 100_000);
        // The screen's first file appears 1.2 s after Resume.
        assert_eq!(place_segment(&mut t, 101_200, None, None, Some(&d)), 101_200);
        assert_eq!(t.gaps.len(), 1, "{:?}", t.gaps);
        assert_eq!((t.gaps[0].from_epoch_ms, t.gaps[0].to_epoch_ms), (12_000, 101_200));
        assert_eq!(t.gaps[0].reason, "paused");
    }

    #[test]
    fn a_failure_after_resume_ends_the_pause_and_opens_its_own_gap() {
        let inner = test_inner("pausefail");
        inner.manifest.lock().unwrap().tracks[0].segments.push(seg(0, 10_000));
        let edges = close_pause(&mut inner.manifest.lock().unwrap(), 20_000);
        assert_eq!(edges, vec![("screen-0".to_string(), 10_000)]);
        let mut o = screen_out(&inner);
        o.downtime = Some(down(10_000, PAUSED_GAP_REASON));
        note_downtime(&inner, &mut o, 21_000, "Display was locked or asleep");
        note_downtime(&inner, &mut o, 40_000, "Display was locked or asleep");
        let gaps = inner.manifest.lock().unwrap().tracks[0].gaps.clone();
        let got: Vec<(i64, i64, &str)> = gaps
            .iter()
            .map(|g| (g.from_epoch_ms, g.to_epoch_ms, g.reason.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![(10_000, 21_000, "paused"), (21_000, 40_000, "Display was locked or asleep")]
        );
        let _ = std::fs::remove_dir_all(&inner.session_dir);
    }

    #[test]
    fn pausing_winds_workers_down_without_reading_as_a_stop() {
        let inner = test_inner("pauseflag");
        inner.pausing.store(true, Ordering::SeqCst);
        assert!(inner.stopping(), "every worker loop exits on a pause");
        assert!(!inner.stop.load(Ordering::SeqCst), "the edit gate must not see a stop");
        inner.pausing.store(false, Ordering::SeqCst);
        assert!(!inner.stopping());
        let _ = std::fs::remove_dir_all(&inner.session_dir);
    }

    #[test]
    fn restart_screens_ends_an_outage_wait_at_once() {
        let inner = test_inner("asked");
        let waiter = {
            let inner = inner.clone();
            std::thread::spawn(move || {
                let mut away = false;
                let t = std::time::Instant::now();
                let woke = wait_for_retry(&inner, 60_000, &mut away);
                (matches!(woke, Wake::UserBack), t.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        inner.retry_asked.fetch_add(1, Ordering::SeqCst);
        let (asked, took) = waiter.join().expect("waiter");
        assert!(asked, "an asked-for retry starts the schedule over");
        assert!(took < Duration::from_secs(5), "took {took:?}");
        let _ = std::fs::remove_dir_all(&inner.session_dir);
    }

    #[test]
    fn after_a_renumber_each_resumed_screen_finds_its_own_monitor() {
        // The session started on `before`; while paused the outputs on adapter
        // 0 were renumbered. Resumed supervisors hold the session-start sources.
        let before = real_outputs();
        let mut after = real_outputs();
        (after[0].output_idx, after[1].output_idx, after[2].output_idx) = (2, 0, 1);
        let ids = identities(&before);
        let mut seen = Vec::new();
        for (i, b) in before.iter().enumerate() {
            let source = TrackSource::Screen {
                adapter: b.adapter,
                output_idx: b.output_idx,
                home_idx: b.output_idx,
                identity: identity_of(b),
                encoder: "h264_nvenc".into(),
                fps: 30,
            };
            let plan = Supervision::new(true).plan_relaunch(locate_in(&source, &ids, &after));
            assert_eq!(plan, RelaunchPlan::Launch(Some(after[i].output_idx)), "{}", b.device_name);
            seen.push((b.adapter, after[i].output_idx));
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), before.len(), "two screens on one monitor: {seen:?}");
    }

    #[test]
    fn a_track_has_at_most_one_supervisor() {
        let inner = test_inner("claims");
        let a = claim_supervisor(&inner, "screen-0").expect("first claim");
        assert!(claim_supervisor(&inner, "screen-0").is_none());
        assert!(claim_supervisor(&inner, "screen-1").is_some());
        drop(a);
        assert!(claim_supervisor(&inner, "screen-0").is_some());
        let _ = std::fs::remove_dir_all(&inner.session_dir);
    }

    #[test]
    fn restart_screens_carries_an_outage_but_not_a_pause_or_idle() {
        let g = |reason: &str| Gap {
            from_epoch_ms: 1_000,
            to_epoch_ms: 2_000,
            reason: reason.into(),
        };
        assert_eq!(
            carried_downtime(Some(&g("Display was locked or asleep"))).map(|d| d.from),
            Some(1_000)
        );
        assert!(carried_downtime(Some(&g(PAUSED_GAP_REASON))).is_none());
        assert!(carried_downtime(Some(&g(IDLE_GAP_REASON))).is_none());
        assert!(carried_downtime(None).is_none());
    }
}
