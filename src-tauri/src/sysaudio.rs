//! Native WASAPI loopback capture of the default render endpoint ("what you hear").
//!
//! This machine has no loopback *device* (no VB-Cable, no Stereo Mix), so ffmpeg's
//! `-f dshow` cannot record system audio. We open the render endpoint in loopback
//! mode ourselves and pipe a fixed-format PCM stream into ffmpeg's stdin.
//!
//! Output is always **interleaved f32le, 48000 Hz, stereo**, written continuously to
//! the `sink` handed to [`SysAudioCapture::start`].
//!
//! ## Wall clock is the master
//!
//! WASAPI loopback delivers *zero* packets while nothing is playing. If we only wrote
//! what WASAPI hands us, the audio track would be shorter than wall-clock time and
//! desync progressively over a 6-hour session. So the writer is driven by the clock,
//! not by the device:
//!
//! - captured frames land in a bounded ring buffer,
//! - every tick we compute how many frames *should* exist by now
//!   (`now - t0_epoch_ms`) and write exactly that many in total,
//! - the ring supplies real audio when it has some, and zeros (silence) otherwise.
//!
//! The written frame count is therefore an exact function of wall-clock time, which
//! also means a device dying, changing, or drifting cannot shift the timeline.

#![cfg(windows)]

use std::collections::VecDeque;
use std::ffi::c_void;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::GUID;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};

// ---------------------------------------------------------------------------
// Fixed output contract
// ---------------------------------------------------------------------------

const OUT_RATE: u32 = 48_000;
const OUT_CH: usize = 2;

/// How far behind wall clock the writer deliberately runs, so that normal WASAPI
/// packet jitter never forces us to pad silence over audio that is about to arrive.
/// Repaid in full by the final flush in [`SysAudioCapture::stop`], so total duration
/// still matches wall clock.
const LAG_MS: i64 = 50;

/// Writer cadence. 10ms => ~480 frames (3840 bytes) per write.
const TICK: Duration = Duration::from_millis(10);

/// Ring capacity in *frames*. 2s of stereo f32 == 768 KB. Overflow drops the oldest
/// audio, which keeps memory flat over a multi-hour session.
const RING_MAX_FRAMES: usize = (OUT_RATE as usize) * 2;

/// Never write more than this per tick, so a long stall can't produce one huge write.
const MAX_CHUNK_FRAMES: u64 = OUT_RATE as u64; // 1s

/// Initial re-open backoff after the endpoint goes away. Doubles per consecutive
/// failure up to [`REOPEN_BACKOFF_MAX`].
const REOPEN_INTERVAL: Duration = Duration::from_millis(500);
const REOPEN_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// How often to check whether the default render endpoint changed under us.
const DEVICE_POLL: Duration = Duration::from_millis(500);

/// How long a *different* default endpoint must stay the default before we act on
/// it. Bluetooth headsets flap: opening a mic on AirPods forces them into hands-free
/// mode, which swaps the default render endpoint back and forth between the A2DP and
/// HFP endpoints several times in a couple of seconds. Chasing every transition
/// would tear down and rebuild the capture repeatedly for no benefit.
const DEVICE_DEBOUNCE: Duration = Duration::from_secs(2);

/// WASAPI buffer size request: 200ms in 100ns units.
const BUFFER_DURATION_100NS: i64 = 2_000_000;

/// `AUDCLNT_BUFFERFLAGS_SILENT` — buffer contents are undefined, treat as silence.
const BUFFERFLAGS_SILENT: u32 = 0x2;

// KSDATAFORMAT_SUBTYPE_* for WAVE_FORMAT_EXTENSIBLE. Spelled out here rather than
// pulled from Win32_Media_KernelStreaming to keep the feature list small.
const SUBTYPE_PCM: GUID = GUID::from_values(
    0x0000_0001,
    0x0000,
    0x0010,
    [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
);
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_values(
    0x0000_0003,
    0x0000,
    0x0010,
    [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
);
const WAVE_FORMAT_PCM_TAG: u16 = 0x0001;
const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 0x0003;
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

/// A running WASAPI loopback capture. Dropping it stops and joins the worker.
pub struct SysAudioCapture {
    stop: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SysAudioCapture {
    /// Start WASAPI loopback on the default render endpoint. Writes interleaved
    /// f32le stereo @48kHz continuously to `sink`, padding SILENCE during gaps,
    /// anchored at `t0_epoch_ms`. Returns immediately; runs on its own thread.
    ///
    /// Fails only if COM cannot be initialised or no render endpoint can be opened
    /// within ~2s. Once started, later device loss is handled internally (silence is
    /// padded, the endpoint is re-opened) and does not stop the stream.
    pub fn start(sink: Box<dyn Write + Send>, t0_epoch_ms: i64) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let healthy = Arc::new(AtomicBool::new(true));

        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let stop_w = stop.clone();
        let healthy_w = healthy.clone();

        let handle = thread::Builder::new()
            .name("sysaudio-loopback".into())
            .spawn(move || {
                worker(sink, t0_epoch_ms, stop_w, healthy_w, ready_tx);
            })
            .map_err(|e| format!("failed to spawn sysaudio thread: {e}"))?;

        match ready_rx.recv_timeout(Duration::from_secs(6)) {
            Ok(Ok(())) => Ok(SysAudioCapture {
                stop,
                healthy,
                handle: Some(handle),
            }),
            Ok(Err(e)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err("sysaudio worker did not become ready within 6s".into())
            }
        }
    }

    /// Signal the worker to finish, flush the trailing silence, close the sink and
    /// join. Returns once the thread is gone (normally within ~1 tick).
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// False once the sink has failed (e.g. ffmpeg exited and the pipe broke) or the
    /// worker thread has terminated. Losing the audio *device* does not make the
    /// capture unhealthy — the stream keeps producing correctly-timed silence.
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for SysAudioCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// COM apartment
// ---------------------------------------------------------------------------

/// Owns this thread's COM apartment and uninitialises it on drop.
///
/// **Declare this before any COM interface pointer in the same scope.** Rust drops
/// locals in reverse declaration order, so declaring it first guarantees
/// `CoUninitialize` runs *after* every interface has been released.
///
/// This is not a style preference. Releasing a COM pointer after `CoUninitialize`
/// dereferences a torn-down proxy and raises `STATUS_ACCESS_VIOLATION` — it is a hard
/// crash of the whole process, not a leak. The previous hand-rolled version of this
/// hit exactly that on the "no render endpoint could be opened" path, where an early
/// `return` ran `CoUninitialize()` while the `IMMDeviceEnumerator` was still live and
/// about to be dropped. RAII makes every exit path correct by construction, including
/// ones added later.
struct ComApartment;

impl ComApartment {
    fn new() -> Result<ComApartment, String> {
        // S_FALSE (already initialised on this thread) is a success code.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_err() {
            return Err(format!("CoInitializeEx failed: {hr:?}"));
        }
        Ok(ComApartment)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

fn worker(
    mut sink: Box<dyn Write + Send>,
    t0_epoch_ms: i64,
    stop: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) {
    // MTA: this thread only ever pumps audio, it has no message loop.
    //
    // `_com` MUST stay the first binding in this function — see ComApartment. Every
    // COM pointer below is declared after it and is therefore released before
    // CoUninitialize runs.
    let _com = match ComApartment::new() {
        Ok(c) => c,
        Err(e) => {
            healthy.store(false, Ordering::SeqCst);
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    let enumerator: IMMDeviceEnumerator =
        match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
            Ok(e) => e,
            Err(e) => {
                healthy.store(false, Ordering::SeqCst);
                let _ = ready_tx.send(Err(format!("MMDeviceEnumerator unavailable: {e}")));
                return;
            }
        };

    // Anchor the clock BEFORE the (possibly slow) first device open, so any time spent
    // opening is padded with silence rather than silently swallowed.
    let anchor_instant = Instant::now();
    let anchor_epoch_ms = now_epoch_ms();

    // Initial open, with a couple of retries — a device can be momentarily busy right
    // after a session switch.
    let mut stream: Option<Stream> = None;
    let mut open_err = String::new();
    for attempt in 0..4 {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match unsafe { Stream::open_default(&enumerator) } {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => {
                open_err = e;
                if attempt < 3 {
                    thread::sleep(REOPEN_INTERVAL);
                }
            }
        }
    }
    if stream.is_none() {
        healthy.store(false, Ordering::SeqCst);
        let _ = ready_tx.send(Err(format!("WASAPI loopback unavailable: {open_err}")));
        return;
    }
    let _ = ready_tx.send(Ok(()));

    let mut ring: VecDeque<f32> = VecDeque::with_capacity(RING_MAX_FRAMES * OUT_CH);
    let mut frames_written: u64 = 0;
    let mut scratch: Vec<u8> = Vec::with_capacity(MAX_CHUNK_FRAMES as usize * OUT_CH * 4);
    let mut last_device_poll = Instant::now();
    let mut next_reopen = Instant::now();
    let mut backoff = REOPEN_INTERVAL;
    let mut watcher = EndpointWatcher::new(
        stream
            .as_ref()
            .map(|s| s.device_id.clone())
            .unwrap_or_default(),
    );

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // --- 1. pull whatever WASAPI has for us -----------------------------
        if let Some(s) = stream.as_mut() {
            if let Err(_e) = unsafe { s.drain(&mut ring) } {
                // Endpoint invalidated / disconnected. Drop it; the clock keeps
                // running and the loop below keeps padding silence.
                stream = None;
                next_reopen = Instant::now() + REOPEN_INTERVAL;
            }
        }

        // --- 2. notice the default endpoint changing under us ---------------
        if last_device_poll.elapsed() >= DEVICE_POLL {
            last_device_poll = Instant::now();
            if let Some(s) = stream.as_ref() {
                if watcher.bound_to() != s.device_id {
                    watcher.rebind(s.device_id.clone());
                }
                if let Ok(id) = unsafe { default_render_id(&enumerator) } {
                    if watcher
                        .observe(&id, Instant::now(), DEVICE_DEBOUNCE)
                        .is_some()
                    {
                        // A genuinely settled endpoint change (headphones plugged in,
                        // AirPods finished connecting). Dropping the stream releases
                        // its COM objects; the ring keeps what was already captured
                        // and the write clock is untouched, so the timeline does not
                        // move -- the swap gap is padded, not shifted.
                        stream = None;
                        next_reopen = Instant::now();
                        backoff = REOPEN_INTERVAL;
                    }
                }
            }
        }

        // --- 3. (re)open if we have no endpoint -----------------------------
        if stream.is_none() && Instant::now() >= next_reopen {
            match unsafe { Stream::open_default(&enumerator) } {
                Ok(s) => {
                    watcher.rebind(s.device_id.clone());
                    stream = Some(s);
                    backoff = REOPEN_INTERVAL;
                }
                Err(_) => {
                    // Bounded exponential backoff so a device that keeps failing to
                    // open cannot turn into an open/release storm over a 6h session.
                    next_reopen = Instant::now() + backoff;
                    backoff = (backoff * 2).min(REOPEN_BACKOFF_MAX);
                }
            }
        }

        // --- 4. write up to the wall-clock target ---------------------------
        let target = target_frames(anchor_instant, anchor_epoch_ms, t0_epoch_ms, LAG_MS);
        if let Err(_e) = flush_to(
            target,
            &mut frames_written,
            &mut ring,
            &mut sink,
            &mut scratch,
        ) {
            // ffmpeg died / pipe broke. Nothing useful left to do.
            healthy.store(false, Ordering::SeqCst);
            break;
        }

        trim_ring(&mut ring);
        thread::sleep(TICK);
    }

    // Final flush with no lag, so the total written duration lines up with wall clock
    // at the moment stop() was called.
    if healthy.load(Ordering::Relaxed) {
        if let Some(s) = stream.as_mut() {
            let _ = unsafe { s.drain(&mut ring) };
        }
        let target = target_frames(anchor_instant, anchor_epoch_ms, t0_epoch_ms, 0);
        if flush_to(
            target,
            &mut frames_written,
            &mut ring,
            &mut sink,
            &mut scratch,
        )
        .is_err()
        {
            healthy.store(false, Ordering::SeqCst);
        }
    }
    let _ = sink.flush();
    drop(sink); // closes ffmpeg's stdin -> clean EOF

    // `stream` and `enumerator` are released here by ordinary scope exit, and `_com`
    // (declared before both) uninitialises the apartment after them.
    healthy.store(false, Ordering::SeqCst);
}

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Total frames that should have been written by now, given the `t0` anchor.
///
/// Elapsed time is measured monotonically (`Instant`) so an NTP step or a DST change
/// mid-session cannot jump the audio timeline; the epoch is only used once, to work
/// out our offset from `t0_epoch_ms`.
fn target_frames(anchor: Instant, anchor_epoch_ms: i64, t0_epoch_ms: i64, lag_ms: i64) -> u64 {
    let offset_us = (anchor_epoch_ms - t0_epoch_ms) as i128 * 1000;
    let elapsed_us = anchor.elapsed().as_micros() as i128;
    let us = offset_us + elapsed_us - (lag_ms as i128 * 1000);
    if us <= 0 {
        return 0;
    }
    (us * OUT_RATE as i128 / 1_000_000) as u64
}

/// Write frames until `frames_written == target`, drawing on the ring and padding with
/// silence when it runs dry. This is the only place bytes reach the sink, which is what
/// makes output length a pure function of wall-clock time.
fn flush_to(
    target: u64,
    frames_written: &mut u64,
    ring: &mut VecDeque<f32>,
    sink: &mut Box<dyn Write + Send>,
    scratch: &mut Vec<u8>,
) -> std::io::Result<()> {
    while *frames_written < target {
        let need = (target - *frames_written).min(MAX_CHUNK_FRAMES);
        scratch.clear();
        for _ in 0..need {
            let (l, r) = if ring.len() >= OUT_CH {
                (
                    ring.pop_front().unwrap_or(0.0),
                    ring.pop_front().unwrap_or(0.0),
                )
            } else {
                (0.0, 0.0)
            };
            scratch.extend_from_slice(&l.to_le_bytes());
            scratch.extend_from_slice(&r.to_le_bytes());
        }
        sink.write_all(scratch)?;
        *frames_written += need;
    }
    Ok(())
}

/// Keep the ring bounded. Overflow means the endpoint clock ran ahead of the system
/// clock (or we stalled); drop the oldest audio so memory stays flat and the stream
/// stays anchored to wall clock.
fn trim_ring(ring: &mut VecDeque<f32>) {
    let max = RING_MAX_FRAMES * OUT_CH;
    if ring.len() > max {
        let excess = ring.len() - max;
        // keep the pairing intact
        drop(ring.drain(..(excess + 1) & !1));
    }
}

// ---------------------------------------------------------------------------
// Endpoint
// ---------------------------------------------------------------------------

struct Stream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    device_id: String,
    fmt: SrcFormat,
    resampler: Option<Resampler>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

impl Stream {
    unsafe fn open_default(enumerator: &IMMDeviceEnumerator) -> Result<Stream, String> {
        let device: IMMDevice = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| format!("no default render endpoint: {e}"))?;
        let device_id = device_id_of(&device)?;

        let client: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("IAudioClient activate failed: {e}"))?;

        let pfmt = client
            .GetMixFormat()
            .map_err(|e| format!("GetMixFormat failed: {e}"))?;
        if pfmt.is_null() {
            return Err("GetMixFormat returned null".into());
        }
        let fmt = parse_format(pfmt);

        let init = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            BUFFER_DURATION_100NS,
            0,
            pfmt,
            None,
        );
        CoTaskMemFree(Some(pfmt as *const c_void));
        init.map_err(|e| format!("IAudioClient::Initialize (loopback) failed: {e}"))?;

        let fmt = fmt?;

        let capture: IAudioCaptureClient = client
            .GetService()
            .map_err(|e| format!("IAudioCaptureClient unavailable: {e}"))?;
        client
            .Start()
            .map_err(|e| format!("IAudioClient::Start failed: {e}"))?;

        let resampler = if fmt.rate == OUT_RATE {
            None
        } else {
            Some(Resampler::new(fmt.rate, OUT_RATE))
        };

        Ok(Stream {
            client,
            capture,
            device_id,
            fmt,
            resampler,
        })
    }

    /// Move every queued packet into `ring` as interleaved stereo f32 @48k.
    unsafe fn drain(&mut self, ring: &mut VecDeque<f32>) -> Result<(), String> {
        loop {
            let packet = self
                .capture
                .GetNextPacketSize()
                .map_err(|e| format!("GetNextPacketSize: {e}"))?;
            if packet == 0 {
                return Ok(());
            }

            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            self.capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .map_err(|e| format!("GetBuffer: {e}"))?;

            if frames > 0 {
                if flags & BUFFERFLAGS_SILENT != 0 || data.is_null() {
                    self.push_silence(frames as usize, ring);
                } else {
                    let bytes = std::slice::from_raw_parts(
                        data,
                        frames as usize * self.fmt.block_align.max(1),
                    );
                    self.push_pcm(bytes, frames as usize, ring);
                }
            }

            self.capture
                .ReleaseBuffer(frames)
                .map_err(|e| format!("ReleaseBuffer: {e}"))?;

            // Defensive: never spin forever inside one drain call.
            if ring.len() > RING_MAX_FRAMES * OUT_CH * 2 {
                return Ok(());
            }
        }
    }

    fn push_silence(&mut self, frames: usize, ring: &mut VecDeque<f32>) {
        match self.resampler.as_mut() {
            None => {
                for _ in 0..frames {
                    ring.push_back(0.0);
                    ring.push_back(0.0);
                }
            }
            Some(rs) => {
                for _ in 0..frames {
                    rs.push(0.0, 0.0, ring);
                }
            }
        }
    }

    fn push_pcm(&mut self, bytes: &[u8], frames: usize, ring: &mut VecDeque<f32>) {
        let stride = self.fmt.block_align.max(1);
        for f in 0..frames {
            let off = f * stride;
            if off + stride > bytes.len() {
                break;
            }
            let (l, r) = self.fmt.frame_to_stereo(&bytes[off..off + stride]);
            match self.resampler.as_mut() {
                None => {
                    ring.push_back(l);
                    ring.push_back(r);
                }
                Some(rs) => rs.push(l, r, ring),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Default-endpoint debounce
// ---------------------------------------------------------------------------

/// Decides *when* a change of default render endpoint is real enough to act on.
///
/// A candidate must remain the default continuously for the whole debounce window
/// before it wins. Any observation of a different id — including the endpoint we are
/// already bound to — restarts the window. That is what makes flapping a no-op: an
/// A -> B -> A -> B oscillation never accumulates an uninterrupted window on B, so
/// we simply stay on A until the churn settles.
///
/// Pure state machine with an injected clock, so it is testable without a device.
struct EndpointWatcher {
    bound: String,
    pending: Option<(String, Instant)>,
}

impl EndpointWatcher {
    fn new(bound: String) -> Self {
        EndpointWatcher {
            bound,
            pending: None,
        }
    }

    fn bound_to(&self) -> &str {
        &self.bound
    }

    /// Adopt `id` as the endpoint we are bound to and forget any candidate.
    fn rebind(&mut self, id: String) {
        self.bound = id;
        self.pending = None;
    }

    /// Feed one observation of the current default endpoint. Returns `Some(id)` only
    /// once `id` has been the default for `debounce` without interruption.
    fn observe(&mut self, id: &str, now: Instant, debounce: Duration) -> Option<String> {
        if id == self.bound {
            // Flapped back to where we already are: cancel any pending switch.
            self.pending = None;
            return None;
        }
        match &self.pending {
            Some((candidate, since)) if candidate == id => {
                if now.duration_since(*since) >= debounce {
                    return Some(id.to_string());
                }
                None
            }
            // First sighting, or a *different* candidate than last time: restart.
            _ => {
                self.pending = Some((id.to_string(), now));
                None
            }
        }
    }
}

unsafe fn default_render_id(enumerator: &IMMDeviceEnumerator) -> Result<String, String> {
    let device = enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)
        .map_err(|e| format!("{e}"))?;
    device_id_of(&device)
}

unsafe fn device_id_of(device: &IMMDevice) -> Result<String, String> {
    let id = device.GetId().map_err(|e| format!("GetId failed: {e}"))?;
    let s = id.to_string().unwrap_or_default();
    CoTaskMemFree(Some(id.0 as *const c_void));
    Ok(s)
}

// ---------------------------------------------------------------------------
// Source format handling
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SampleKind {
    F32,
    I16,
    I32,
    /// 24 valid bits in a 32-bit container
    I24In32,
    /// packed 3-byte samples
    I24Packed,
}

#[derive(Clone, Copy, Debug)]
struct SrcFormat {
    channels: usize,
    rate: u32,
    kind: SampleKind,
    block_align: usize,
    sample_bytes: usize,
}

impl SrcFormat {
    #[inline]
    fn sample(&self, frame: &[u8], ch: usize) -> f32 {
        let off = ch * self.sample_bytes;
        if off + self.sample_bytes > frame.len() {
            return 0.0;
        }
        let b = &frame[off..off + self.sample_bytes];
        match self.kind {
            SampleKind::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            SampleKind::I16 => i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0,
            SampleKind::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            SampleKind::I24In32 => {
                (i32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 8) as f32 / 8_388_608.0
            }
            SampleKind::I24Packed => {
                let v = ((b[2] as i32) << 24 | (b[1] as i32) << 16 | (b[0] as i32) << 8) >> 8;
                v as f32 / 8_388_608.0
            }
        }
    }

    /// Downmix one source frame to stereo.
    ///
    /// Mono duplicates. 2..=5 channels take the first two (front L/R by convention).
    /// 6+ channels get the standard ITU downmix using FL/FR/FC/SL/SR at -3dB, LFE
    /// dropped — matching what a player would do.
    #[inline]
    fn frame_to_stereo(&self, frame: &[u8]) -> (f32, f32) {
        match self.channels {
            0 => (0.0, 0.0),
            1 => {
                let m = self.sample(frame, 0);
                (m, m)
            }
            2..=5 => (self.sample(frame, 0), self.sample(frame, 1)),
            _ => {
                const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
                let fl = self.sample(frame, 0);
                let fr = self.sample(frame, 1);
                let fc = self.sample(frame, 2);
                // index 3 is LFE, deliberately dropped
                let sl = self.sample(frame, 4);
                let sr = self.sample(frame, 5);
                let l = (fl + K * fc + K * sl).clamp(-1.0, 1.0);
                let r = (fr + K * fc + K * sr).clamp(-1.0, 1.0);
                (l, r)
            }
        }
    }
}

/// `WAVEFORMATEX(TENSIBLE)` are `#[repr(packed)]`, so every field is read through
/// `read_unaligned` rather than a reference.
fn parse_format(pfmt: *const WAVEFORMATEX) -> Result<SrcFormat, String> {
    let w = unsafe { std::ptr::read_unaligned(pfmt) };
    let channels = w.nChannels as usize;
    let rate = w.nSamplesPerSec;
    let bits = w.wBitsPerSample;
    let block_align = w.nBlockAlign as usize;

    if channels == 0 || rate == 0 || bits == 0 || block_align == 0 {
        return Err(format!(
            "nonsense mix format: {channels}ch {rate}Hz {bits}bit align {block_align}"
        ));
    }

    let mut tag = w.wFormatTag;
    let mut valid_bits = bits;
    if tag == WAVE_FORMAT_EXTENSIBLE_TAG && w.cbSize >= 22 {
        let ext = unsafe { std::ptr::read_unaligned(pfmt as *const WAVEFORMATEXTENSIBLE) };
        let vb = unsafe { ext.Samples.wValidBitsPerSample };
        if vb > 0 {
            valid_bits = vb;
        }
        let sub = ext.SubFormat;
        tag = if sub == SUBTYPE_IEEE_FLOAT {
            WAVE_FORMAT_IEEE_FLOAT_TAG
        } else if sub == SUBTYPE_PCM {
            WAVE_FORMAT_PCM_TAG
        } else {
            return Err(format!("unsupported mix subformat {sub:?}"));
        };
    }

    let kind = match (tag, bits, valid_bits) {
        (WAVE_FORMAT_IEEE_FLOAT_TAG, 32, _) => SampleKind::F32,
        (WAVE_FORMAT_PCM_TAG, 16, _) => SampleKind::I16,
        (WAVE_FORMAT_PCM_TAG, 24, _) => SampleKind::I24Packed,
        (WAVE_FORMAT_PCM_TAG, 32, 24) => SampleKind::I24In32,
        (WAVE_FORMAT_PCM_TAG, 32, _) => SampleKind::I32,
        _ => return Err(format!("unsupported mix format: tag {tag} {bits}bit")),
    };

    Ok(SrcFormat {
        channels,
        rate,
        kind,
        block_align,
        sample_bytes: (bits as usize) / 8,
    })
}

// ---------------------------------------------------------------------------
// Resampling (only used when the endpoint mix rate isn't 48kHz)
// ---------------------------------------------------------------------------

/// Push-driven Catmull-Rom resampler for stereo, with a Butterworth low-pass ahead of
/// it when decimating so we don't fold HF energy back into the band.
struct Resampler {
    /// input frames consumed per output frame
    ratio: f64,
    t: f64,
    hist: [[f32; 4]; OUT_CH],
    primed: usize,
    lpf: Option<[Biquad; OUT_CH]>,
}

impl Resampler {
    fn new(src_rate: u32, dst_rate: u32) -> Resampler {
        let ratio = src_rate as f64 / dst_rate as f64;
        let lpf = if src_rate > dst_rate {
            let cutoff = dst_rate as f64 * 0.45;
            Some([
                Biquad::lowpass(src_rate as f64, cutoff),
                Biquad::lowpass(src_rate as f64, cutoff),
            ])
        } else {
            None
        };
        Resampler {
            ratio,
            t: 0.0,
            hist: [[0.0; 4]; OUT_CH],
            primed: 0,
            lpf,
        }
    }

    fn push(&mut self, l: f32, r: f32, out: &mut VecDeque<f32>) {
        let (l, r) = match self.lpf.as_mut() {
            Some([a, b]) => (a.run(l), b.run(r)),
            None => (l, r),
        };
        for (c, v) in [l, r].into_iter().enumerate() {
            self.hist[c].rotate_left(1);
            self.hist[c][3] = v;
        }
        if self.primed < 4 {
            self.primed += 1;
            if self.primed < 4 {
                return;
            }
        }
        while self.t < 1.0 {
            let t = self.t as f32;
            out.push_back(catmull_rom(&self.hist[0], t));
            out.push_back(catmull_rom(&self.hist[1], t));
            self.t += self.ratio;
        }
        self.t -= 1.0;
    }
}

#[inline]
fn catmull_rom(x: &[f32; 4], t: f32) -> f32 {
    let c0 = x[1];
    let c1 = 0.5 * (x[2] - x[0]);
    let c2 = x[0] - 2.5 * x[1] + 2.0 * x[2] - 0.5 * x[3];
    let c3 = 0.5 * (x[3] - x[0]) + 1.5 * (x[1] - x[2]);
    ((c3 * t + c2) * t + c1) * t + c0
}

/// Direct-form-1 biquad, RBJ cookbook low-pass (Q = 1/sqrt(2)).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn lowpass(rate: f64, cutoff: f64) -> Biquad {
        let w0 = 2.0 * std::f64::consts::PI * (cutoff / rate);
        let (sn, cs) = (w0.sin(), w0.cos());
        // Q = 1/sqrt(2) (Butterworth) => alpha = sin(w0) / (2Q)
        let alpha = sn / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let b0 = (1.0 - cs) / 2.0;
        let b1 = 1.0 - cs;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cs;
        let a2 = 1.0 - alpha;
        Biquad {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
            ..Default::default()
        }
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const DB: Duration = Duration::from_secs(2);

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn stable_change_switches_after_the_window() {
        let mut w = EndpointWatcher::new("A".into());
        let t = t0();
        assert_eq!(w.observe("B", t, DB), None, "first sighting must not switch");
        assert_eq!(w.observe("B", t + Duration::from_millis(1000), DB), None);
        assert_eq!(w.observe("B", t + Duration::from_millis(1999), DB), None);
        assert_eq!(
            w.observe("B", t + Duration::from_millis(2000), DB),
            Some("B".to_string()),
            "must switch once B has been default for the full window"
        );
    }

    #[test]
    fn flapping_between_two_endpoints_never_switches() {
        // The AirPods A2DP <-> hands-free case: alternating every 500ms for 20s.
        let mut w = EndpointWatcher::new("A".into());
        let t = t0();
        for i in 0..40 {
            let id = if i % 2 == 0 { "B" } else { "A" };
            let at = t + Duration::from_millis(500 * i as u64);
            assert_eq!(
                w.observe(id, at, DB),
                None,
                "flapping must never trigger a switch (i={i})"
            );
        }
    }

    #[test]
    fn a_brief_blip_is_ignored_and_resets_the_window() {
        let mut w = EndpointWatcher::new("A".into());
        let t = t0();
        assert_eq!(w.observe("B", t, DB), None);
        // blip back to A after 100ms -> candidate B is cancelled
        assert_eq!(w.observe("A", t + Duration::from_millis(100), DB), None);
        // B reappears; the window restarts from here, so the original t is stale
        assert_eq!(w.observe("B", t + Duration::from_millis(200), DB), None);
        assert_eq!(
            w.observe("B", t + Duration::from_millis(2100), DB),
            None,
            "2.1s after the FIRST sighting is only 1.9s after the restart"
        );
        assert_eq!(
            w.observe("B", t + Duration::from_millis(2200), DB),
            Some("B".to_string())
        );
    }

    #[test]
    fn a_different_candidate_restarts_the_window() {
        let mut w = EndpointWatcher::new("A".into());
        let t = t0();
        assert_eq!(w.observe("B", t, DB), None);
        assert_eq!(w.observe("C", t + Duration::from_millis(1900), DB), None);
        assert_eq!(
            w.observe("B", t + Duration::from_millis(2000), DB),
            None,
            "B's earlier window must not carry over"
        );
        assert_eq!(
            w.observe("C", t + Duration::from_millis(2100), DB),
            None,
            "nor C's"
        );
    }

    #[test]
    fn three_way_churn_settles_on_whichever_endpoint_stops_moving() {
        let mut w = EndpointWatcher::new("A".into());
        let t = t0();
        for (i, id) in ["B", "C", "B", "C"].iter().enumerate() {
            assert_eq!(w.observe(id, t + Duration::from_millis(300 * i as u64), DB), None);
        }
        // C then settles
        let s = t + Duration::from_millis(1200);
        assert_eq!(w.observe("C", s, DB), None);
        assert_eq!(
            w.observe("C", s + Duration::from_millis(2000), DB),
            Some("C".to_string())
        );
    }

    #[test]
    fn rebind_adopts_the_new_endpoint_and_clears_the_candidate() {
        let mut w = EndpointWatcher::new("A".into());
        let t = t0();
        assert_eq!(w.observe("B", t, DB), None);
        w.rebind("B".into());
        assert_eq!(w.bound_to(), "B");
        // Now bound to B, so B is quiet and A becomes the candidate.
        assert_eq!(w.observe("B", t + Duration::from_millis(2500), DB), None);
        assert_eq!(w.observe("A", t + Duration::from_millis(2600), DB), None);
        assert_eq!(
            w.observe("A", t + Duration::from_millis(4700), DB),
            Some("A".to_string())
        );
    }

    /// Live diagnostic, not a regression test: opens loopback on the CURRENT
    /// default render endpoint, plays a Windows sound to that endpoint partway
    /// through, and reports what WASAPI actually delivered. Run with
    /// `cargo test --lib live_loopback_probe -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_loopback_probe() {
        let _com = ComApartment::new().expect("COM");
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.expect("enum");

        let default_id = unsafe { default_render_id(&enumerator) }.expect("default id");
        println!("default render endpoint: {default_id}");

        let mut s = unsafe { Stream::open_default(&enumerator) }.expect("open loopback");
        println!(
            "opened: device={} fmt={:?} {}ch {}Hz",
            s.device_id, s.fmt.kind, s.fmt.channels, s.fmt.rate
        );

        let mut ring: VecDeque<f32> = VecDeque::new();
        let mut played = false;
        let started = Instant::now();
        let mut per_second: Vec<(u64, usize, usize, f32)> = Vec::new(); // (sec, samples, nonzero, peak)
        let mut cur = (0u64, 0usize, 0usize, 0f32);

        while started.elapsed() < Duration::from_secs(8) {
            if !played && started.elapsed() >= Duration::from_secs(2) {
                played = true;
                // SoundPlayer renders via waveOut -> default console endpoint.
                let _ = std::process::Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-c",
                        "(New-Object Media.SoundPlayer 'C:\\Windows\\Media\\Alarm01.wav').Play(); Start-Sleep -s 4",
                    ])
                    .spawn();
                println!("--- playback launched at t+2s ---");
            }
            if let Err(e) = unsafe { s.drain(&mut ring) } {
                println!("drain error: {e}");
                break;
            }
            let sec = started.elapsed().as_secs();
            if sec != cur.0 {
                per_second.push(cur);
                cur = (sec, 0, 0, 0.0);
            }
            for v in ring.drain(..) {
                cur.1 += 1;
                if v.abs() > 1e-4 {
                    cur.2 += 1;
                }
                cur.3 = cur.3.max(v.abs());
            }
            thread::sleep(TICK);
        }
        per_second.push(cur);
        for (sec, n, nz, peak) in per_second {
            println!("t={sec}s: {n} samples delivered, {nz} non-silent, peak={peak:.4}");
        }
    }

    #[test]
    fn no_switch_is_reported_twice_for_the_same_settled_candidate() {
        // The worker calls rebind() after acting; until it does, the watcher will
        // keep reporting -- which is correct, but the worker must not spin. Verify
        // that rebind makes it quiet.
        let mut w = EndpointWatcher::new("A".into());
        let t = t0();
        w.observe("B", t, DB);
        let got = w.observe("B", t + DB, DB);
        assert_eq!(got, Some("B".to_string()));
        w.rebind("B".into());
        assert_eq!(w.observe("B", t + DB + Duration::from_secs(10), DB), None);
    }
}
