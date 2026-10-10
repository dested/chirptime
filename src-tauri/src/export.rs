//! Export composer (S5): turn a range of a session into one MP4.
//!
//! One ffmpeg run per export (two for Share's two-pass). Each track feeds in as
//! ONE input, a concat-demuxer list of only the segments the export shows, so an
//! 8 h export opens a handful of inputs, never hundreds. Each list places its
//! segments at their real wall-clock offsets (`duration` per entry), so a gap
//! in a track is a hole in its timestamps, not a shift. The filter graph then:
//!
//! 1. `select`s the frames inside the kept pieces (the range minus whole-session
//!    cuts, and minus idle stretches when "Skip idle stretches" is on) and
//!    `setpts` collapses the pieces end to end;
//! 2. composes the layout over a black canvas, each layer an `overlay` that is
//!    `enable`d only while its track has footage and is the one shown (Follow
//!    mouse switches screens as hard cuts). Where no screen has footage the
//!    camera fills the frame; with no camera it stays black;
//! 3. mixes mic and system audio with gains (and `sidechaincompress` ducking),
//!    silence filling the holes;
//! 4. applies the speed (the canvas runs at 30/speed fps, then `setpts`).
//!
//! Everything up to the ffmpeg arguments is pure (`build_plan`, `filter_graph`,
//! `follow_segments`, `share_plan`), and tested in `tests/export.rs`.
//!
//! On disk: `<recordings>/exports/<YYYY-MM-DD HH.mm> <title>.mp4` (the range's
//! start time), a first-frame `.jpg` next to it, and `exports/index.jsonl`, one
//! record per export, rewritten atomically. Work files live in
//! `exports/.work/<job>/` and go when the job ends, however it ends.

use crate::cursor::CursorSample;
use crate::recorder::EncoderCaps;
use crate::session::{self, CutState, SessionManifest, Track};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const EXPORTS_DIR: &str = "exports";
pub const INDEX_FILE: &str = "index.jsonl";
const WORK_DIR: &str = ".work";
/// Output frame rate, whatever the speed.
pub const OUT_FPS: u32 = 30;
/// Follow mouse ignores a visit to another screen shorter than this.
pub const FOLLOW_HOLD_MS: i64 = 1_500;
/// A hole between segments shorter than this is keyframe jitter, not a gap
/// (the player's `SEAM_TOLERANCE_MS`).
pub const SEAM_MS: i64 = 250;
/// Kept pieces shorter than this are dropped: a frame or two between two cuts.
pub const MIN_PIECE_MS: i64 = 200;
/// Share's ceiling. 5% is held back for the container and rate-control overshoot.
pub const SHARE_LIMIT_BYTES: u64 = 25_000_000;
const SHARE_HEADROOM: f64 = 0.95;
/// Lowest video bitrate (kbps) each Share height still looks acceptable at.
const SHARE_TIERS: [(u32, u32); 3] = [(1080, 1_800), (720, 800), (480, 280)];
const SHARE_MAX_VIDEO_KBPS: u32 = 6_000;
/// Follow-mouse select terms within this of each other merge (the overlay's
/// `enable` stays exact; this only keeps the `select` expression short).
const SELECT_SLACK_MS: i64 = 5_000;

const CANCELLED: &str = "Cancelled";
const NOTHING_LEFT: &str = "Nothing is left in this range once deleted time is taken out";
const NOTHING_LEFT_IDLE: &str =
    "Nothing is left in this range once deleted time and idle stretches are taken out";

// ---------------------------------------------------------------------------
// The request (the composer's right card)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// The shown screen full frame, the camera in a corner.
    #[default]
    Devlog,
    /// 9:16: a crop of the screen above the camera.
    Short,
    /// The screen on the left half, the camera on the right.
    SideBySide,
    /// Every screen where it really sits, scaled, the camera in a corner.
    Desk,
}

impl Layout {
    pub fn key(self) -> &'static str {
        match self {
            Layout::Devlog => "devlog",
            Layout::Short => "short",
            Layout::SideBySide => "side-by-side",
            Layout::Desk => "desk",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Layout::Devlog => "Devlog",
            Layout::Short => "Short",
            Layout::SideBySide => "Side by side",
            Layout::Desk => "Desk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    #[default]
    Rounded,
    Circle,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraOpts {
    pub enabled: bool,
    pub corner: Corner,
    pub shape: Shape,
    /// The corner camera's width as a fraction of the frame's width.
    pub size: f64,
}

impl Default for CameraOpts {
    fn default() -> Self {
        Self {
            enabled: true,
            corner: Corner::BottomRight,
            shape: Shape::Rounded,
            size: 0.28,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SoundOpts {
    /// Gain in dB; `None` leaves the mic out.
    pub mic_db: Option<f64>,
    /// Gain in dB; `None` leaves system audio out.
    pub system_db: Option<f64>,
    /// "Lower system audio while I talk".
    pub duck: bool,
}

impl Default for SoundOpts {
    fn default() -> Self {
        Self {
            mic_db: Some(0.0),
            system_db: Some(-12.0),
            duck: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Speed {
    #[default]
    #[serde(rename = "1x")]
    X1,
    #[serde(rename = "2x")]
    X2,
    #[serde(rename = "4x")]
    X4,
    /// One frame per second of footage, played at 30 fps.
    #[serde(rename = "timelapse")]
    Timelapse,
}

impl Speed {
    pub fn factor(self) -> u32 {
        match self {
            Speed::X1 => 1,
            Speed::X2 => 2,
            Speed::X4 => 4,
            Speed::Timelapse => OUT_FPS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SizeMode {
    /// The sources' own height (up to 2160).
    #[serde(rename = "original")]
    Original,
    #[default]
    #[serde(rename = "1080p")]
    P1080,
    /// Two-pass to at most 25 MB.
    #[serde(rename = "share")]
    Share,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub session_id: String,
    pub from_ms: i64,
    pub to_ms: i64,
    #[serde(default)]
    pub layout: Layout,
    /// A screen track id, or `None` to follow the mouse.
    #[serde(default)]
    pub screen: Option<String>,
    #[serde(default)]
    pub camera: CameraOpts,
    #[serde(default)]
    pub sound: SoundOpts,
    #[serde(default)]
    pub speed: Speed,
    #[serde(default)]
    pub skip_idle: bool,
    #[serde(default)]
    pub size: SizeMode,
    /// `Some`: export these tracks as they are (`-c copy`) into a folder
    /// instead of composing.
    #[serde(default)]
    pub raw_tracks: Option<Vec<String>>,
    #[serde(default)]
    pub title: Option<String>,
}

// ---------------------------------------------------------------------------
// What the composer shows before and while exporting
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportEstimate {
    /// Length of the finished file.
    pub output_ms: i64,
    /// Footage time it covers (the range minus cuts, minus idle when skipped).
    pub source_ms: i64,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    /// "NVENC", "Quick Sync" or "x264".
    pub encoder: String,
    /// A rough guess at how long the export takes.
    pub encode_seconds: i64,
    /// Things worth saying, e.g. that Follow mouse has no mouse data here.
    pub notes: Vec<String>,
    /// Set when this export can't run as asked (e.g. too long for Share).
    pub error: Option<String>,
}

/// Payload of the `export-progress` event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub job_id: String,
    pub fraction: f64,
    pub eta_seconds: Option<f64>,
    /// "running" | "done" | "failed" | "cancelled"
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

/// One line of `exports/index.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRecord {
    pub id: String,
    pub session_id: String,
    pub from: i64,
    pub to: i64,
    /// A layout key, or "raw" for a folder of raw tracks.
    pub layout: String,
    /// Relative to the recordings folder, `/`-separated (servable at
    /// `http://localhost:9120/files/<path>`). A folder for raw tracks.
    pub path: String,
    pub bytes: u64,
    pub duration_ms: i64,
    pub created_epoch_ms: i64,
    #[serde(default)]
    pub title: String,
    /// The first frame, relative like `path`.
    #[serde(default)]
    pub thumb: Option<String>,
}

// ---------------------------------------------------------------------------
// Spans: half-open [from, to) in epoch ms
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub from: i64,
    pub to: i64,
}

impl Span {
    pub fn new(from: i64, to: i64) -> Self {
        Self { from, to }
    }
    pub fn len(&self) -> i64 {
        self.to - self.from
    }
    pub fn is_empty(&self) -> bool {
        self.to <= self.from
    }
}

/// Sorted, with spans that overlap or sit within `slack` of each other merged.
pub fn merge_spans(mut v: Vec<Span>, slack: i64) -> Vec<Span> {
    v.retain(|s| !s.is_empty());
    v.sort_by_key(|s| (s.from, s.to));
    let mut out: Vec<Span> = Vec::with_capacity(v.len());
    for s in v {
        match out.last_mut() {
            Some(last) if s.from <= last.to + slack => last.to = last.to.max(s.to),
            _ => out.push(s),
        }
    }
    out
}

/// `keep` minus `remove`. Both must be merged (sorted, disjoint).
pub fn subtract_spans(keep: &[Span], remove: &[Span]) -> Vec<Span> {
    let mut out = Vec::new();
    for k in keep {
        let mut cur = k.from;
        for r in remove {
            if r.to <= cur || r.from >= k.to {
                continue;
            }
            if r.from > cur {
                out.push(Span::new(cur, r.from));
            }
            cur = cur.max(r.to);
            if cur >= k.to {
                break;
            }
        }
        if cur < k.to {
            out.push(Span::new(cur, k.to));
        }
    }
    out
}

/// Both must be merged.
pub fn intersect_spans(a: &[Span], b: &[Span]) -> Vec<Span> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let from = a[i].from.max(b[j].from);
        let to = a[i].to.min(b[j].to);
        if from < to {
            out.push(Span::new(from, to));
        }
        if a[i].to < b[j].to {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

pub fn union_spans(a: &[Span], b: &[Span]) -> Vec<Span> {
    merge_spans(a.iter().chain(b).copied().collect(), 0)
}

fn total_ms(v: &[Span]) -> i64 {
    v.iter().map(Span::len).sum()
}

// ---------------------------------------------------------------------------
// Facts from the manifest
// ---------------------------------------------------------------------------

fn live_cut(c: &session::Cut) -> bool {
    c.state != CutState::Undone
}

/// Time deleted from every track: the export skips it.
pub fn session_holes(m: &SessionManifest) -> Vec<Span> {
    merge_spans(
        m.cuts
            .iter()
            .filter(|c| live_cut(c))
            .filter(|c| match &c.tracks {
                None => true,
                Some(ts) => m.tracks.iter().all(|t| ts.iter().any(|x| x == &t.id)),
            })
            .map(|c| Span::new(c.from_epoch_ms, c.to_epoch_ms))
            .collect(),
        0,
    )
}

/// Every hole one track has, whole-session cuts included.
pub fn track_holes(m: &SessionManifest, track_id: &str) -> Vec<Span> {
    merge_spans(
        m.cuts
            .iter()
            .filter(|c| live_cut(c))
            .filter(|c| c.tracks.as_ref().map_or(true, |ts| ts.iter().any(|t| t == track_id)))
            .map(|c| Span::new(c.from_epoch_ms, c.to_epoch_ms))
            .collect(),
        0,
    )
}

/// Where the track has footage that may be shown: its segments (seams within
/// `SEAM_MS` bridged) minus its cuts.
pub fn footage_spans(m: &SessionManifest, t: &Track) -> Vec<Span> {
    let segs = merge_spans(
        t.segments
            .iter()
            .map(|s| Span::new(s.start_epoch_ms, s.start_epoch_ms + s.duration_ms))
            .collect(),
        SEAM_MS,
    );
    subtract_spans(&segs, &track_holes(m, &t.id))
}

pub fn is_skippable_reason(reason: &str) -> bool {
    let r = reason.trim().to_ascii_lowercase();
    r.starts_with("idle") || r == "paused"
}

fn is_video(t: &Track) -> bool {
    t.kind == "screen" || t.kind == "webcam"
}

/// Stretches recorded as idle (or paused) where no video track has footage.
pub fn idle_spans(m: &SessionManifest) -> Vec<Span> {
    let idle = merge_spans(
        m.tracks
            .iter()
            .flat_map(|t| t.gaps.iter())
            .filter(|g| is_skippable_reason(&g.reason))
            .map(|g| Span::new(g.from_epoch_ms, g.to_epoch_ms))
            .collect(),
        0,
    );
    let covered = merge_spans(
        m.tracks
            .iter()
            .filter(|t| is_video(t))
            .flat_map(|t| t.segments.iter())
            .map(|s| Span::new(s.start_epoch_ms, s.start_epoch_ms + s.duration_ms))
            .collect(),
        0,
    );
    subtract_spans(&idle, &covered)
}

pub fn session_span(m: &SessionManifest) -> Span {
    let mut from = m.started_epoch_ms;
    let mut to = m.ended_epoch_ms.unwrap_or(m.started_epoch_ms);
    for t in &m.tracks {
        if let Some(s) = t.segments.first() {
            from = from.min(s.start_epoch_ms);
        }
        if let Some(e) = t.end_epoch_ms() {
            to = to.max(e);
        }
    }
    Span::new(from, to.max(from))
}

/// `screen-2` → 3.
pub fn screen_number(t: &Track) -> u32 {
    let digits: String = t.id.chars().rev().take_while(|c| c.is_ascii_digit()).collect();
    digits.chars().rev().collect::<String>().parse::<u32>().map(|n| n + 1).unwrap_or(0)
}

fn screen_tracks(m: &SessionManifest) -> Vec<&Track> {
    let mut v: Vec<&Track> = m
        .tracks
        .iter()
        .filter(|t| t.kind == "screen" && !t.segments.is_empty())
        .collect();
    v.sort_by_key(|t| screen_number(t));
    v
}

/// The track's picture size: the file's own, else its monitor's, else 1080p.
fn track_dims(t: &Track) -> (u32, u32) {
    if t.width > 0 && t.height > 0 {
        return (t.width, t.height);
    }
    match &t.monitor {
        Some(mo) if mo.w > 0 && mo.h > 0 => (mo.w, mo.h),
        _ => (1920, 1080),
    }
}

fn monitor_area(t: &Track) -> u64 {
    match &t.monitor {
        Some(mo) => mo.w as u64 * mo.h as u64,
        None => {
            let (w, h) = track_dims(t);
            w as u64 * h as u64
        }
    }
}

// ---------------------------------------------------------------------------
// Follow mouse
// ---------------------------------------------------------------------------

/// A screen in virtual-desktop pixels (the cursor's space).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl ScreenRect {
    fn contains(&self, x: i64, y: i64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Which screen (index into `screens`) to show when, covering `[from, to)` end
/// to end. Switches are hard cuts at the moment the cursor entered the new
/// screen; a visit shorter than `FOLLOW_HOLD_MS` doesn't count. A cursor off
/// every screen, or no samples for a while, keeps the current one. With no
/// samples at all it is `fallback` throughout.
pub fn follow_segments(
    samples: &[CursorSample],
    screens: &[ScreenRect],
    from: i64,
    to: i64,
    fallback: usize,
) -> Vec<(Span, usize)> {
    if to <= from {
        return Vec::new();
    }
    // Runs of consecutive samples on one screen: (screen, entered at).
    let mut runs: Vec<(usize, i64)> = Vec::new();
    for s in samples.iter().filter(|s| s.t < to) {
        let Some(i) = screens.iter().position(|r| r.contains(s.x as i64, s.y as i64)) else {
            continue;
        };
        if runs.last().map(|r| r.0) != Some(i) {
            runs.push((i, s.t.max(from)));
        }
    }
    let Some(first) = runs.first() else {
        return vec![(Span::new(from, to), fallback)];
    };
    let mut current = first.0;
    let mut switches: Vec<(i64, usize)> = vec![(from, current)];
    for k in 1..runs.len() {
        let (screen, at) = runs[k];
        let until = runs.get(k + 1).map(|r| r.1).unwrap_or(to);
        if screen != current && until - at >= FOLLOW_HOLD_MS {
            current = screen;
            switches.push((at, screen));
        }
    }
    let mut out: Vec<(Span, usize)> = Vec::with_capacity(switches.len());
    for (k, &(at, screen)) in switches.iter().enumerate() {
        let end = switches.get(k + 1).map(|s| s.0).unwrap_or(to);
        if end > at {
            out.push((Span::new(at, end), screen));
        }
    }
    out
}

fn screen_rect(t: &Track, fallback_x: i64) -> ScreenRect {
    match &t.monitor {
        Some(mo) => ScreenRect {
            x: mo.x as i64,
            y: mo.y as i64,
            w: mo.w as i64,
            h: mo.h as i64,
        },
        None => {
            let (w, h) = track_dims(t);
            ScreenRect {
                x: fallback_x,
                y: 0,
                w: w as i64,
                h: h as i64,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Output time
// ---------------------------------------------------------------------------

/// The kept pieces laid end to end: source epoch ms ↔ output ms (at 1x).
#[derive(Debug, Clone, PartialEq)]
pub struct OutMap {
    pub pieces: Vec<Span>,
    /// Output ms at which each piece starts.
    pub starts: Vec<i64>,
    pub total_ms: i64,
}

impl OutMap {
    pub fn new(pieces: Vec<Span>) -> Self {
        let mut starts = Vec::with_capacity(pieces.len());
        let mut acc = 0;
        for p in &pieces {
            starts.push(acc);
            acc += p.len();
        }
        Self {
            pieces,
            starts,
            total_ms: acc,
        }
    }

    /// Source spans (merged) → output spans, in ms, merged where they touch.
    pub fn map(&self, spans: &[Span]) -> Vec<Span> {
        let mut out = Vec::new();
        for (p, &start) in self.pieces.iter().zip(&self.starts) {
            for s in intersect_spans(std::slice::from_ref(p), spans) {
                out.push(Span::new(start + s.from - p.from, start + s.to - p.from));
            }
        }
        merge_spans(out, 0)
    }
}

/// The kept pieces of `[from, to)`: whole-session cuts out, idle out if asked.
pub fn kept_pieces(m: &SessionManifest, from: i64, to: i64, skip_idle: bool) -> Vec<Span> {
    let mut removed = session_holes(m);
    if skip_idle {
        removed = union_spans(&removed, &idle_spans(m));
    }
    subtract_spans(&[Span::new(from, to)], &removed)
        .into_iter()
        .filter(|p| p.len() >= MIN_PIECE_MS)
        .collect()
}

// ---------------------------------------------------------------------------
// Concat lists
// ---------------------------------------------------------------------------

/// One `file` line of a concat-demuxer list. The file is placed at `start_ms`
/// on the list's clock (relative to the first entry), plays to `outpoint_ms`
/// (where the next segment overlaps it), and the next file begins
/// `duration_ms` later, which leaves a hole in the timestamps over a gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcatEntry {
    /// `<track dir>/<file>`
    pub file: String,
    pub start_epoch_ms: i64,
    pub inpoint_ms: Option<i64>,
    pub outpoint_ms: Option<i64>,
    pub duration_ms: i64,
}

/// The segments of `t` that overlap `keep`, placed at their wall-clock offsets.
/// Overlapping segments are trimmed where the next one starts (the later wins,
/// as in the player); two that start together keep the later one.
pub fn concat_entries(t: &Track, keep: &[Span]) -> Vec<ConcatEntry> {
    let mut segs: Vec<&session::Segment> = t
        .segments
        .iter()
        .filter(|s| s.duration_ms > 0)
        .filter(|s| {
            let sp = Span::new(s.start_epoch_ms, s.start_epoch_ms + s.duration_ms);
            keep.iter().any(|k| k.from < sp.to && k.to > sp.from)
        })
        .collect();
    segs.sort_by_key(|s| s.start_epoch_ms);
    let mut dedup: Vec<&session::Segment> = Vec::with_capacity(segs.len());
    for s in segs {
        match dedup.last_mut() {
            Some(last) if last.start_epoch_ms == s.start_epoch_ms => *last = s,
            _ => dedup.push(s),
        }
    }
    let mut out = Vec::with_capacity(dedup.len());
    for (i, s) in dedup.iter().enumerate() {
        let gap_to_next = dedup.get(i + 1).map(|n| n.start_epoch_ms - s.start_epoch_ms);
        let duration_ms = gap_to_next.unwrap_or(s.duration_ms);
        let outpoint_ms = match gap_to_next {
            Some(g) if g < s.duration_ms => Some(g),
            _ => None,
        };
        out.push(ConcatEntry {
            file: format!("{}/{}", t.dir, s.file),
            start_epoch_ms: s.start_epoch_ms,
            inpoint_ms: None,
            outpoint_ms,
            duration_ms,
        });
    }
    out
}

/// Raw tracks: each piece's footage, trimmed to the piece (`-c copy` snaps the
/// in point back to a keyframe), laid end to end.
pub fn raw_entries(t: &Track, pieces: &[Span]) -> Vec<ConcatEntry> {
    let mut segs: Vec<&session::Segment> = t.segments.iter().filter(|s| s.duration_ms > 0).collect();
    segs.sort_by_key(|s| s.start_epoch_ms);
    let mut out = Vec::new();
    for p in pieces {
        for (i, s) in segs.iter().enumerate() {
            let mut end = s.start_epoch_ms + s.duration_ms;
            if let Some(n) = segs.get(i + 1) {
                end = end.min(n.start_epoch_ms);
            }
            let a = p.from.max(s.start_epoch_ms);
            let b = p.to.min(end);
            if b - a < MIN_PIECE_MS {
                continue;
            }
            let inpoint = a - s.start_epoch_ms;
            let outpoint = b - s.start_epoch_ms;
            out.push(ConcatEntry {
                file: format!("{}/{}", t.dir, s.file),
                start_epoch_ms: a,
                inpoint_ms: (inpoint > 0).then_some(inpoint),
                outpoint_ms: (outpoint < s.duration_ms).then_some(outpoint),
                duration_ms: b - a,
            });
        }
    }
    out
}

fn secs(ms: i64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// The concat list. Paths are absolute with forward slashes, single-quoted.
pub fn concat_list_text(session_dir: &Path, entries: &[ConcatEntry]) -> String {
    let mut s = String::from("ffconcat version 1.0\n");
    for e in entries {
        let p = session_dir.join(&e.file).to_string_lossy().replace('\\', "/");
        s.push_str(&format!("file '{}'\n", p.replace('\'', "'\\''")));
        if let Some(i) = e.inpoint_ms {
            s.push_str(&format!("inpoint {}\n", secs(i)));
        }
        if let Some(o) = e.outpoint_ms {
            s.push_str(&format!("outpoint {}\n", secs(o)));
        }
        s.push_str(&format!("duration {}\n", secs(e.duration_ms)));
    }
    s
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Video,
    Audio,
}

/// One ffmpeg input: a track as a concat list.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanInput {
    pub track_id: String,
    pub kind: InputKind,
    pub entries: Vec<ConcatEntry>,
    /// The frames this input contributes, in source epoch ms (inside pieces).
    pub select: Vec<Span>,
}

impl PlanInput {
    /// Epoch ms at which the list's clock reads 0.
    pub fn e0(&self) -> i64 {
        self.entries.first().map(|e| e.start_epoch_ms).unwrap_or(0)
    }
}

/// A picture laid over the canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub input: usize,
    /// Scaled size before any crop.
    pub scale: (u32, u32),
    /// Center crop to this size after scaling (cover fit).
    pub crop: Option<(u32, u32)>,
    pub x: i64,
    pub y: i64,
    /// Through the camera mask (rounded or circle).
    pub masked: bool,
    /// When it shows, in output ms (at 1x).
    pub enable: Vec<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioUse {
    pub input: usize,
    pub gain_db: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub speed: u32,
    pub out: OutMap,
    pub inputs: Vec<PlanInput>,
    pub layers: Vec<Layer>,
    pub mic: Option<AudioUse>,
    pub system: Option<AudioUse>,
    pub duck: bool,
    /// The camera mask's size and shape, when a layer uses it.
    pub mask: Option<(u32, u32, Shape)>,
    /// Which screen shows when (source epoch), for Devlog/Short/Side by side.
    pub focus: Vec<(Span, String)>,
    pub notes: Vec<String>,
}

impl Plan {
    pub fn composed_ms(&self) -> i64 {
        self.out.total_ms
    }
    pub fn output_ms(&self) -> i64 {
        self.out.total_ms / self.speed as i64
    }
    pub fn has_audio(&self) -> bool {
        self.speed != Speed::Timelapse.factor() && (self.mic.is_some() || self.system.is_some())
    }
    pub fn video_inputs(&self) -> usize {
        self.inputs.iter().filter(|i| i.kind == InputKind::Video).count()
    }
}

/// The nearest even number (video dimensions must be even), at least 2.
fn even(v: f64) -> u32 {
    ((v / 2.0).round().max(1.0) as u32) * 2
}

/// Canvas size for a layout at a reference height `p` (the short side for Short).
pub fn canvas_dims(layout: Layout, p: u32) -> (u32, u32) {
    let long = even(p as f64 * 16.0 / 9.0);
    match layout {
        Layout::Short => (p, long),
        _ => (long, p),
    }
}

/// The reference height for Original: the tallest source, 480..=2160.
fn original_p(sources: &[&Track]) -> u32 {
    let h = sources.iter().map(|t| track_dims(t).1).max().unwrap_or(1080);
    even(h.clamp(480, 2160) as f64)
}

/// Contain `src` in `box`: scaled size and the offset that centers it.
fn contain(src: (u32, u32), bx: (u32, u32)) -> ((u32, u32), (i64, i64)) {
    let s = (bx.0 as f64 / src.0 as f64).min(bx.1 as f64 / src.1 as f64);
    let w = even(src.0 as f64 * s).min(bx.0);
    let h = even(src.1 as f64 * s).min(bx.1);
    ((w, h), ((bx.0 as i64 - w as i64) / 2, (bx.1 as i64 - h as i64) / 2))
}

/// Cover `box` with `src`: scaled size; crop to `box` after.
fn cover(src: (u32, u32), bx: (u32, u32)) -> (u32, u32) {
    let s = (bx.0 as f64 / src.0 as f64).max(bx.1 as f64 / src.1 as f64);
    (even(src.0 as f64 * s).max(bx.0), even(src.1 as f64 * s).max(bx.1))
}

/// The corner camera's box size for a frame `w` wide.
pub fn corner_camera_dims(frame_w: u32, cam: &CameraOpts) -> (u32, u32) {
    let size = cam.size.clamp(0.1, 0.5);
    let w = even(frame_w as f64 * size);
    let h = even(w as f64 * 9.0 / 16.0);
    match cam.shape {
        Shape::Rounded => (w, h),
        Shape::Circle => (h, h),
    }
}

fn corner_xy(frame: (u32, u32), bx: (u32, u32), corner: Corner) -> (i64, i64) {
    let m = (frame.0.min(frame.1) as f64 * 0.04).round() as i64;
    let (fw, fh) = (frame.0 as i64, frame.1 as i64);
    let (bw, bh) = (bx.0 as i64, bx.1 as i64);
    match corner {
        Corner::TopLeft => (m, m),
        Corner::TopRight => (fw - bw - m, m),
        Corner::BottomLeft => (m, fh - bh - m),
        Corner::BottomRight => (fw - bw - m, fh - bh - m),
    }
}

/// Everything about an export short of running it. `cursor` is the mouse
/// track over the range (empty when there is none). `share_p` is the height
/// Share settled on; ignored otherwise.
pub fn build_plan(
    m: &SessionManifest,
    req: &ExportRequest,
    cursor: &[CursorSample],
    share_p: Option<u32>,
) -> Result<Plan, String> {
    let whole = session_span(m);
    let from = req.from_ms.max(whole.from);
    let to = req.to_ms.min(whole.to);
    if to - from < 1_000 {
        return Err("Pick at least a second of this session to export".into());
    }
    let pieces = kept_pieces(m, from, to, req.skip_idle);
    if pieces.is_empty() {
        return Err(if req.skip_idle { NOTHING_LEFT_IDLE } else { NOTHING_LEFT }.into());
    }
    let out = OutMap::new(pieces.clone());
    let mut notes = Vec::new();

    let screens = screen_tracks(m);
    let cam = if req.camera.enabled {
        m.tracks.iter().find(|t| t.kind == "webcam" && !t.segments.is_empty())
    } else {
        None
    };

    // Which screens show, and when (source epoch).
    let mut focus: Vec<(Span, String)> = Vec::new();
    let shown: Vec<(&Track, Vec<Span>)> = if req.layout == Layout::Desk {
        screens.iter().map(|t| (*t, pieces.clone())).collect()
    } else if screens.is_empty() {
        Vec::new()
    } else {
        let schedule: Vec<(Span, usize)> = match &req.screen {
            Some(id) => {
                let i = screens
                    .iter()
                    .position(|t| &t.id == id)
                    .ok_or_else(|| format!("This session has no screen named {}", id))?;
                vec![(Span::new(from, to), i)]
            }
            None => {
                let largest = screens
                    .iter()
                    .enumerate()
                    .max_by_key(|(i, t)| (monitor_area(t), std::cmp::Reverse(*i)))
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                if cursor.is_empty() {
                    if screens.len() > 1 {
                        notes.push(format!(
                            "No mouse data for this range, so it shows Screen {}, the largest",
                            screen_number(screens[largest])
                        ));
                    }
                    vec![(Span::new(from, to), largest)]
                } else {
                    let mut x = 0;
                    let rects: Vec<ScreenRect> = screens
                        .iter()
                        .map(|t| {
                            let r = screen_rect(t, x);
                            x += r.w;
                            r
                        })
                        .collect();
                    follow_segments(cursor, &rects, from, to, largest)
                }
            }
        };
        let mut per: Vec<Vec<Span>> = vec![Vec::new(); screens.len()];
        for (span, i) in &schedule {
            per[*i].push(*span);
            focus.push((*span, screens[*i].id.clone()));
        }
        screens
            .iter()
            .zip(per)
            .filter(|(_, spans)| !spans.is_empty())
            .map(|(t, spans)| (*t, intersect_spans(&merge_spans(spans, 0), &pieces)))
            .collect()
    };

    // What each screen shows, where it has footage.
    let visible: Vec<(&Track, Vec<Span>)> = shown
        .into_iter()
        .map(|(t, spans)| {
            let v = intersect_spans(&spans, &footage_spans(m, t));
            (t, v)
        })
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let screen_visible = merge_spans(visible.iter().flat_map(|(_, v)| v.iter().copied()).collect(), 0);
    let no_screen = subtract_spans(&pieces, &screen_visible);
    let cam_footage = cam.map(|c| intersect_spans(&footage_spans(m, c), &pieces)).unwrap_or_default();

    // Canvas.
    let sources: Vec<&Track> = visible.iter().map(|(t, _)| *t).collect();
    let p = match req.size {
        SizeMode::P1080 => 1080,
        SizeMode::Original => original_p(&sources),
        SizeMode::Share => share_p.unwrap_or(1080),
    };
    let (w, h) = canvas_dims(req.layout, p);

    let mut inputs: Vec<PlanInput> = Vec::new();
    let mut layers: Vec<Layer> = Vec::new();
    fn input_for(t: &Track, kind: InputKind, inputs: &mut Vec<PlanInput>) -> usize {
        if let Some(i) = inputs.iter().position(|x| x.track_id == t.id) {
            return i;
        }
        inputs.push(PlanInput {
            track_id: t.id.clone(),
            kind,
            entries: Vec::new(),
            select: Vec::new(),
        });
        inputs.len() - 1
    }
    let add_layer = |t: &Track,
                         show: &[Span],
                         scale: (u32, u32),
                         crop: Option<(u32, u32)>,
                         xy: (i64, i64),
                         masked: bool,
                         inputs: &mut Vec<PlanInput>,
                         layers: &mut Vec<Layer>| {
        if show.is_empty() {
            return;
        }
        let input = input_for(t, InputKind::Video, inputs);
        inputs[input].select = union_spans(&inputs[input].select, show);
        layers.push(Layer {
            input,
            scale,
            crop,
            x: xy.0,
            y: xy.1,
            masked,
            enable: out.map(show),
        });
    };

    let has_cam = !cam_footage.is_empty();
    // The screens' area, and the camera's own box where the layout has one.
    let (screen_box, screen_cover, cam_box): ((i64, i64, u32, u32), bool, Option<(i64, i64, u32, u32)>) =
        match req.layout {
            Layout::Devlog | Layout::Desk => ((0, 0, w, h), false, None),
            Layout::Short => {
                if has_cam {
                    let top = even(h as f64 * 0.6);
                    ((0, 0, w, top), true, Some((0, top as i64, w, h - top)))
                } else {
                    ((0, 0, w, h), true, None)
                }
            }
            Layout::SideBySide => {
                if has_cam {
                    let half = even(w as f64 / 2.0);
                    ((0, 0, half, h), false, Some((half as i64, 0, w - half, h)))
                } else {
                    ((0, 0, w, h), false, None)
                }
            }
        };

    if req.layout == Layout::Desk {
        // Every screen at its real position, the whole desk contained.
        let mut x = 0;
        let rects: Vec<ScreenRect> = visible
            .iter()
            .map(|(t, _)| {
                let r = screen_rect(t, x);
                x += r.w;
                r
            })
            .collect();
        if let (Some(bx0), Some(by0), Some(bx1), Some(by1)) = (
            rects.iter().map(|r| r.x).min(),
            rects.iter().map(|r| r.y).min(),
            rects.iter().map(|r| r.x + r.w).max(),
            rects.iter().map(|r| r.y + r.h).max(),
        ) {
            let margin = (w.min(h) as f64 * 0.03).round();
            let s = ((w as f64 - 2.0 * margin) / (bx1 - bx0).max(1) as f64)
                .min((h as f64 - 2.0 * margin) / (by1 - by0).max(1) as f64);
            let ox = (w as f64 - (bx1 - bx0) as f64 * s) / 2.0;
            let oy = (h as f64 - (by1 - by0) as f64 * s) / 2.0;
            for ((t, show), r) in visible.iter().zip(&rects) {
                let sw = even(r.w as f64 * s);
                let sh = even(r.h as f64 * s);
                let x = (ox + (r.x - bx0) as f64 * s).round() as i64;
                let y = (oy + (r.y - by0) as f64 * s).round() as i64;
                add_layer(t, show, (sw, sh), None, (x, y), false, &mut inputs, &mut layers);
            }
        }
    } else {
        let (bx, by, bw, bh) = screen_box;
        for (t, show) in &visible {
            let src = track_dims(t);
            if screen_cover {
                let sc = cover(src, (bw, bh));
                add_layer(t, show, sc, Some((bw, bh)), (bx, by), false, &mut inputs, &mut layers);
            } else {
                let (sc, (dx, dy)) = contain(src, (bw, bh));
                add_layer(t, show, sc, None, (bx + dx, by + dy), false, &mut inputs, &mut layers);
            }
        }
    }

    let mut mask = None;
    if let Some(c) = cam.filter(|_| has_cam) {
        let src = track_dims(c);
        // No screen to show: the camera fills the frame.
        let full = intersect_spans(&no_screen, &cam_footage);
        add_layer(c, &full, cover(src, (w, h)), Some((w, h)), (0, 0), false, &mut inputs, &mut layers);
        let beside = intersect_spans(&screen_visible, &cam_footage);
        match cam_box {
            Some((x, y, bw, bh)) => {
                add_layer(c, &beside, cover(src, (bw, bh)), Some((bw, bh)), (x, y), false, &mut inputs, &mut layers);
            }
            None => {
                let dims = corner_camera_dims(w, &req.camera);
                let xy = corner_xy((w, h), dims, req.camera.corner);
                let before = layers.len();
                add_layer(c, &beside, cover(src, dims), Some(dims), xy, true, &mut inputs, &mut layers);
                if layers.len() > before {
                    mask = Some((dims.0, dims.1, req.camera.shape));
                }
            }
        }
    }

    if layers.is_empty() {
        notes.push("Nothing in this range has a picture, so the video is black".into());
    }

    // Sound.
    let speed = req.speed.factor();
    let audio = |kind: &str, db: Option<f64>, inputs: &mut Vec<PlanInput>| -> Option<AudioUse> {
        let db = db?;
        if req.speed == Speed::Timelapse {
            return None;
        }
        let t = m.tracks.iter().find(|t| t.kind == kind && !t.segments.is_empty())?;
        let keep = intersect_spans(&footage_spans(m, t), &pieces);
        if keep.is_empty() {
            return None;
        }
        let input = input_for(t, InputKind::Audio, inputs);
        inputs[input].select = keep;
        Some(AudioUse { input, gain_db: db })
    };
    let mic = audio("mic", req.sound.mic_db, &mut inputs);
    let system = audio("system-audio", req.sound.system_db, &mut inputs);
    if req.speed == Speed::Timelapse && (req.sound.mic_db.is_some() || req.sound.system_db.is_some()) {
        notes.push("Timelapse has no sound".into());
    }

    // Each input's list: only the segments it contributes. Video selects are
    // loosened (the overlays' `enable` stays exact) so Follow mouse's select
    // stays a few terms, never wider than the pieces.
    for inp in &mut inputs {
        let t = m.tracks.iter().find(|t| t.id == inp.track_id).ok_or("track vanished")?;
        if inp.kind == InputKind::Video {
            let loose = merge_spans(inp.select.clone(), SELECT_SLACK_MS);
            inp.select = intersect_spans(&subtract_spans(&loose, &track_holes(m, &t.id)), &pieces);
        }
        inp.entries = concat_entries(t, &inp.select);
        if inp.entries.is_empty() {
            return Err(format!("No footage of {} to export", t.id));
        }
    }

    Ok(Plan {
        width: w,
        height: h,
        speed,
        out,
        inputs,
        layers,
        mic,
        system,
        duck: req.sound.duck,
        mask,
        focus,
        notes,
    })
}

// ---------------------------------------------------------------------------
// The filter graph
// ---------------------------------------------------------------------------

/// `between(t,a,b)+…` over output spans, in seconds.
fn enable_expr(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|s| format!("between(t,{},{})", secs(s.from), secs(s.to)))
        .collect::<Vec<_>>()
        .join("+")
}

/// `select` over an input's own clock (seconds since its first entry).
fn select_expr(spans: &[Span], e0: i64) -> String {
    spans
        .iter()
        .map(|s| format!("between(t,{},{})", secs(s.from - e0), secs(s.to - e0)))
        .collect::<Vec<_>>()
        .join("+")
}

/// `setpts` that lays the kept pieces end to end: within piece k, output time
/// is input time plus a constant shift. Frames outside pieces were dropped by
/// `select`, so a step function of `T` is exact.
fn collapse_expr(out: &OutMap, e0: i64) -> String {
    let shift = |k: usize| out.starts[k] - out.pieces[k].from + e0;
    let s0 = shift(0);
    let mut s = format!("T{}{}", if s0 < 0 { "-" } else { "+" }, secs(s0.abs()));
    for k in 1..out.pieces.len() {
        // Always negative: the gap between piece k-1 and piece k.
        let d = shift(k) - shift(k - 1);
        let sign = if d < 0 { "-" } else { "+" };
        s.push_str(&format!("{}gte(T,{})*{}", sign, secs(out.pieces[k].from - e0), secs(d.abs())));
    }
    s
}

/// Index of the mask image input, after every concat input.
fn mask_input_index(plan: &Plan) -> usize {
    plan.inputs.len()
}

/// The canvas frame rate: 30 / speed, as a rational.
fn canvas_rate(speed: u32) -> String {
    match speed {
        1 => "30".into(),
        s if OUT_FPS % s == 0 => format!("{}", OUT_FPS / s),
        s => format!("{}/{}", OUT_FPS, s),
    }
}

/// The whole `-filter_complex`, ending in `[vout]` (and `[aout]` with audio).
pub fn filter_graph(plan: &Plan, with_audio: bool) -> String {
    let mut g: Vec<String> = Vec::new();
    let d = secs(plan.composed_ms());

    // Each video input, selected, collapsed, split once per layer that uses it.
    for (i, inp) in plan.inputs.iter().enumerate() {
        if inp.kind != InputKind::Video {
            continue;
        }
        let uses: Vec<usize> = plan
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.input == i)
            .map(|(n, _)| n)
            .collect();
        if uses.is_empty() {
            continue;
        }
        let e0 = inp.e0();
        let head = format!(
            "[{}:v]select='{}',setpts='({})/TB'",
            i,
            select_expr(&inp.select, e0),
            collapse_expr(&plan.out, e0)
        );
        if uses.len() == 1 {
            g.push(format!("{}[s{}]", head, uses[0]));
        } else {
            let outs: String = uses.iter().map(|n| format!("[s{}]", n)).collect();
            g.push(format!("{},split={}{}", head, uses.len(), outs));
        }
    }

    // Each layer scaled (and cropped, and masked).
    for (n, l) in plan.layers.iter().enumerate() {
        let mut chain = format!("[s{}]scale={}:{}", n, l.scale.0, l.scale.1);
        if let Some((cw, ch)) = l.crop {
            chain.push_str(&format!(",crop={}:{}", cw, ch));
        }
        if l.masked {
            g.push(format!("{},format=yuva420p[m{}]", chain, n));
            g.push(format!("[{}:v]format=gray[mask{}]", mask_input_index(plan), n));
            g.push(format!("[m{}][mask{}]alphamerge[l{}]", n, n, n));
        } else {
            g.push(format!("{},setsar=1[l{}]", chain, n));
        }
    }

    // The canvas, then every layer over it.
    g.push(format!(
        "color=c=black:s={}x{}:r={}:d={}[c0]",
        plan.width,
        plan.height,
        canvas_rate(plan.speed),
        d
    ));
    for (n, l) in plan.layers.iter().enumerate() {
        let full = l.enable.len() == 1 && l.enable[0].from <= 0 && l.enable[0].to >= plan.composed_ms();
        let enable = if full {
            String::new()
        } else {
            format!(":enable='{}'", enable_expr(&l.enable))
        };
        g.push(format!(
            "[c{}][l{}]overlay=x={}:y={}:eof_action=pass{}[c{}]",
            n,
            n,
            l.x,
            l.y,
            enable,
            n + 1
        ));
    }
    let last = plan.layers.len();
    if plan.speed > 1 {
        g.push(format!(
            "[c{}]setpts=PTS/{},fps={},format=yuv420p[vout]",
            last, plan.speed, OUT_FPS
        ));
    } else {
        g.push(format!("[c{}]format=yuv420p[vout]", last));
    }

    // Sound: each source selected, collapsed, silence in its holes, gained.
    if with_audio && plan.has_audio() {
        let chain_for = |a: &AudioUse, label: &str, g: &mut Vec<String>| {
            let inp = &plan.inputs[a.input];
            let e0 = inp.e0();
            g.push(format!(
                "[{}:a]aselect='{}',asetpts='({})/TB',aresample=async=1:min_hard_comp=0.01:first_pts=0,volume={:.1}dB,apad,atrim=end={}[{}]",
                a.input,
                select_expr(&inp.select, e0),
                collapse_expr(&plan.out, e0),
                a.gain_db,
                d,
                label
            ));
        };
        let mixed = match (&plan.mic, &plan.system) {
            (Some(mic), Some(sys)) => {
                chain_for(mic, "mic", &mut g);
                chain_for(sys, "sys", &mut g);
                if plan.duck {
                    g.push("[mic]asplit=2[mic1][mic2]".into());
                    g.push("[sys][mic2]sidechaincompress=threshold=0.02:ratio=8:attack=20:release=400[sysd]".into());
                    g.push("[mic1][sysd]amix=inputs=2:normalize=0:duration=first[mix]".into());
                } else {
                    g.push("[mic][sys]amix=inputs=2:normalize=0:duration=first[mix]".into());
                }
                "mix"
            }
            (Some(mic), None) => {
                chain_for(mic, "mix", &mut g);
                "mix"
            }
            (None, Some(sys)) => {
                chain_for(sys, "mix", &mut g);
                "mix"
            }
            (None, None) => "",
        };
        if !mixed.is_empty() {
            let tempo = match plan.speed {
                2 => "atempo=2".to_string(),
                4 => "atempo=2,atempo=2".to_string(),
                _ => "anull".to_string(),
            };
            g.push(format!("[{}]{}[aout]", mixed, tempo));
        }
    }
    g.join(";\n")
}

/// An 8-bit PGM (P5) alpha mask for the corner camera: a rounded rectangle or a
/// circle, anti-aliased by pixel coverage.
pub fn mask_pgm(w: u32, h: u32, shape: Shape) -> Vec<u8> {
    let mut out = format!("P5\n{} {}\n255\n", w, h).into_bytes();
    let (fw, fh) = (w as f64, h as f64);
    let r = match shape {
        Shape::Rounded => fh.min(fw) * 0.14,
        Shape::Circle => fh.min(fw) / 2.0,
    };
    for y in 0..h {
        for x in 0..w {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            // Signed distance to the rounded rect (a circle when r = half).
            let qx = (px - fw / 2.0).abs() - (fw / 2.0 - r);
            let qy = (py - fh / 2.0).abs() - (fh / 2.0 - r);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let dist = outside + qx.max(qy).min(0.0) - r;
            let a = (0.5 - dist).clamp(0.0, 1.0);
            out.push((a * 255.0).round() as u8);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Size, encoder, estimate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharePlan {
    /// Reference height: 1080, 720 or 480.
    pub p: u32,
    pub video_kbps: u32,
    pub audio_kbps: u32,
}

/// Bitrates for a two-pass encode that lands under 25 MB, at the tallest
/// height that still looks acceptable. Refused when even 480p won't fit.
pub fn share_plan(output_ms: i64, with_audio: bool) -> Result<SharePlan, String> {
    let secs = (output_ms.max(1) as f64) / 1000.0;
    let budget_kbps = (SHARE_LIMIT_BYTES as f64 * 8.0 * SHARE_HEADROOM / 1000.0 / secs).floor() as u32;
    let audio_kbps = if !with_audio {
        0
    } else if budget_kbps >= 1_000 {
        96
    } else {
        64
    };
    let video_kbps = budget_kbps.saturating_sub(audio_kbps).min(SHARE_MAX_VIDEO_KBPS);
    for (p, min) in SHARE_TIERS {
        if video_kbps >= min {
            return Ok(SharePlan {
                p,
                video_kbps,
                audio_kbps,
            });
        }
    }
    let longest_ms = (SHARE_LIMIT_BYTES as f64 * 8.0 * SHARE_HEADROOM
        / ((SHARE_TIERS[2].1 + audio_kbps) as f64 * 1000.0)
        * 1000.0) as i64;
    Err(format!(
        "Too long for 25 MB: {} would drop below 480p. Share fits about {}; pick a shorter range or a faster speed",
        fmt_minutes(output_ms),
        fmt_minutes(longest_ms)
    ))
}

fn fmt_minutes(ms: i64) -> String {
    let m = (ms as f64 / 60_000.0).round() as i64;
    if m < 1 {
        format!("{} seconds", (ms / 1000).max(1))
    } else if m < 120 {
        format!("{} minute{}", m, if m == 1 { "" } else { "s" })
    } else {
        format!("{}h {:02}m", m / 60, m % 60)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Nvenc,
    Qsv,
    X264,
}

impl Encoder {
    pub fn name(self) -> &'static str {
        match self {
            Encoder::Nvenc => "NVENC",
            Encoder::Qsv => "Quick Sync",
            Encoder::X264 => "x264",
        }
    }
    fn args(self) -> Vec<String> {
        let v: &[&str] = match self {
            Encoder::Nvenc => &["-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", "23", "-b:v", "0"],
            Encoder::Qsv => &["-c:v", "h264_qsv", "-preset", "medium", "-global_quality", "23"],
            Encoder::X264 => &["-c:v", "libx264", "-preset", "veryfast", "-crf", "23"],
        };
        v.iter().map(|s| s.to_string()).collect()
    }
}

/// NVENC, then Quick Sync, then x264. h264_nvenc can't go past 4096 wide.
pub fn pick_encoder(caps: &EncoderCaps, w: u32, h: u32) -> Encoder {
    if caps.h264_nvenc && w <= 4096 && h <= 4096 {
        Encoder::Nvenc
    } else if caps.h264_qsv {
        Encoder::Qsv
    } else {
        Encoder::X264
    }
}

/// Footage bytes inside the pieces for these tracks, prorated at the edges.
pub fn raw_bytes(m: &SessionManifest, tracks: &[String], pieces: &[Span]) -> u64 {
    let mut total = 0.0;
    for t in m.tracks.iter().filter(|t| tracks.contains(&t.id)) {
        for s in &t.segments {
            if s.duration_ms <= 0 {
                continue;
            }
            let sp = Span::new(s.start_epoch_ms, s.start_epoch_ms + s.duration_ms);
            let inside: i64 = intersect_spans(&[sp], pieces).iter().map(Span::len).sum();
            total += s.bytes as f64 * inside as f64 / s.duration_ms as f64;
        }
    }
    total.round() as u64
}

/// What the footer says before the button is pressed. Never fails: a request
/// that can't run comes back with `error` set.
pub fn estimate(m: &SessionManifest, req: &ExportRequest, cursor: &[CursorSample], caps: &EncoderCaps) -> ExportEstimate {
    let mut est = ExportEstimate {
        output_ms: 0,
        source_ms: 0,
        bytes: 0,
        width: 0,
        height: 0,
        encoder: String::new(),
        encode_seconds: 0,
        notes: Vec::new(),
        error: None,
    };
    let whole = session_span(m);
    let from = req.from_ms.max(whole.from);
    let to = req.to_ms.min(whole.to);

    if let Some(tracks) = &req.raw_tracks {
        let pieces = kept_pieces(m, from, to, false);
        est.source_ms = total_ms(&pieces);
        est.output_ms = est.source_ms;
        est.bytes = raw_bytes(m, tracks, &pieces);
        est.encoder = "copy".into();
        // Copying is disk-bound: about 200 MB/s.
        est.encode_seconds = (est.bytes / 200_000_000) as i64 + tracks.len() as i64;
        if tracks.is_empty() {
            est.error = Some("Pick at least one track".into());
        } else if pieces.is_empty() {
            est.error = Some(NOTHING_LEFT.into());
        }
        return est;
    }

    let mut share: Option<SharePlan> = None;
    if req.size == SizeMode::Share {
        match build_plan(m, req, cursor, None) {
            Ok(p0) => match share_plan(p0.output_ms(), p0.has_audio()) {
                Ok(s) => share = Some(s),
                Err(e) => {
                    est.output_ms = p0.output_ms();
                    est.source_ms = p0.composed_ms();
                    est.error = Some(e);
                    est.notes = p0.notes;
                    return est;
                }
            },
            Err(e) => {
                est.error = Some(e);
                return est;
            }
        }
    }
    let plan = match build_plan(m, req, cursor, share.map(|s| s.p)) {
        Ok(p) => p,
        Err(e) => {
            est.error = Some(e);
            return est;
        }
    };
    est.output_ms = plan.output_ms();
    est.source_ms = plan.composed_ms();
    est.width = plan.width;
    est.height = plan.height;
    est.notes = plan.notes.clone();
    let out_s = est.output_ms as f64 / 1000.0;
    let pixels = plan.width as f64 * plan.height as f64;
    let encoder = if share.is_some() { Encoder::X264 } else { pick_encoder(caps, plan.width, plan.height) };
    est.encoder = encoder.name().into();
    est.bytes = match share {
        Some(s) => ((s.video_kbps + s.audio_kbps) as f64 * 1000.0 / 8.0 * out_s) as u64,
        None => {
            // Screen content at CRF/CQ 23: about 0.05 bits per pixel per frame,
            // more when sped up (less redundancy between frames).
            let bpp = match req.speed {
                Speed::X1 => 0.05,
                Speed::X2 | Speed::X4 => 0.08,
                Speed::Timelapse => 0.14,
            };
            let audio = if plan.has_audio() { 160_000.0 } else { 0.0 };
            ((pixels * OUT_FPS as f64 * bpp + audio) / 8.0 * out_s) as u64
        }
    };
    // Decode is the bulk: each video input runs for the footage it shows.
    let decode_s = plan.composed_ms() as f64 / 1000.0 * 0.04 * plan.video_inputs().max(1) as f64;
    let per_frame = match encoder {
        Encoder::Nvenc | Encoder::Qsv => 0.0015,
        Encoder::X264 => 0.008,
    } * (pixels / 2_073_600.0);
    let passes = if share.is_some() { 2.0 } else { 1.0 };
    est.encode_seconds = (decode_s * passes + out_s * OUT_FPS as f64 * per_frame * passes).ceil() as i64;
    est
}

// ---------------------------------------------------------------------------
// Files: names, the index
// ---------------------------------------------------------------------------

/// `2026-08-01 12.10 Devlog` from the range's start and a title, made safe for
/// a Windows file name.
pub fn export_stem(range_start_ms: i64, title: &str) -> String {
    use chrono::TimeZone;
    let when = chrono::Local
        .timestamp_millis_opt(range_start_ms)
        .single()
        .map(|t| t.format("%Y-%m-%d %H.%M").to_string())
        .unwrap_or_else(|| "export".into());
    let clean: String = title
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean.chars().take(80).collect();
    let clean = clean.trim_end_matches(['.', ' ']);
    if clean.is_empty() {
        when
    } else {
        format!("{} {}", when, clean)
    }
}

/// `<stem><ext>`, or `<stem> (2)<ext>`… whichever doesn't exist yet.
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{}{}", stem, ext));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{} ({}){}", stem, n, ext));
        n += 1;
    }
    p
}

fn rel_to(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Serializes index rewrites within this process.
static INDEX_LOCK: Mutex<()> = Mutex::new(());

pub fn read_index(recordings: &Path) -> Vec<ExportRecord> {
    let path = recordings.join(EXPORTS_DIR).join(INDEX_FILE);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<ExportRecord>(l).ok())
        .collect()
}

/// Rewrite the whole index through a temp file and a rename.
fn write_index(recordings: &Path, records: &[ExportRecord]) -> Result<(), String> {
    let dir = recordings.join(EXPORTS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    let mut text = String::new();
    for r in records {
        text.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
        text.push('\n');
    }
    let tmp = dir.join(format!("{}.tmp", INDEX_FILE));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, dir.join(INDEX_FILE)).map_err(|e| format!("index: {}", e))
}

pub fn add_to_index(recordings: &Path, rec: ExportRecord) -> Result<(), String> {
    let _g = INDEX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = read_index(recordings);
    all.retain(|r| r.id != rec.id);
    all.push(rec);
    write_index(recordings, &all)
}

/// Exports whose file is still there, newest first.
pub fn list(recordings: &Path) -> Vec<ExportRecord> {
    let mut v: Vec<ExportRecord> = read_index(recordings)
        .into_iter()
        .filter(|r| recordings.join(&r.path).exists())
        .collect();
    v.sort_by(|a, b| b.created_epoch_ms.cmp(&a.created_epoch_ms));
    v
}

fn record_path(recordings: &Path, rel: &str) -> Result<PathBuf, String> {
    let p = recordings.join(rel);
    let base = recordings.join(EXPORTS_DIR);
    // Only ever touch what lives under exports/.
    if rel.split('/').any(|c| c == ".." || c.is_empty()) || !p.starts_with(&base) {
        return Err("That export is not in the exports folder".into());
    }
    Ok(p)
}

/// Remove an export's file (or folder), its thumbnail and its record.
pub fn delete(recordings: &Path, id: &str) -> Result<(), String> {
    let _g = INDEX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = read_index(recordings);
    let Some(rec) = all.iter().find(|r| r.id == id).cloned() else {
        return Err("That export is already gone".into());
    };
    let p = record_path(recordings, &rec.path)?;
    if p.is_dir() {
        std::fs::remove_dir_all(&p).map_err(|e| format!("Could not delete the folder: {}", e))?;
    } else if p.exists() {
        std::fs::remove_file(&p).map_err(|e| format!("Could not delete the file: {}", e))?;
    }
    if let Some(t) = &rec.thumb {
        if let Ok(tp) = record_path(recordings, t) {
            let _ = std::fs::remove_file(tp);
        }
    }
    all.retain(|r| r.id != id);
    write_index(recordings, &all)
}

// ---------------------------------------------------------------------------
// Running ffmpeg
// ---------------------------------------------------------------------------

/// A running export: cancel kills whatever ffmpeg it is in.
#[derive(Default)]
pub struct JobCtl {
    cancelled: AtomicBool,
    child: Mutex<Option<std::process::Child>>,
}

impl JobCtl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = c.kill();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    Cancelled,
    Failed(String),
}

impl From<String> for RunError {
    fn from(s: String) -> Self {
        RunError::Failed(s)
    }
}

/// `-/filter_complex <file>` arrived in ffmpeg 7; older builds only know
/// `-filter_complex_script`.
pub fn filter_script_flag(version_line: &str) -> &'static str {
    let major = version_line
        .split_whitespace()
        .nth(2)
        .and_then(|v| v.trim_start_matches('n').split(['.', '-']).next().map(str::to_string))
        .and_then(|v| v.parse::<u32>().ok());
    match major {
        Some(m) if m < 7 => "-filter_complex_script",
        _ => "-/filter_complex",
    }
}

fn ffmpeg_version_line(ffmpeg: &str) -> String {
    crate::recorder::silent_command(ffmpeg)
        .arg("-version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string())
        .unwrap_or_default()
}

/// Run one ffmpeg with `-progress pipe:1`, reporting the fraction of
/// `expect_ms` written. Stderr is drained on its own thread and its tail is the
/// error.
fn run_ffmpeg(
    ffmpeg: &str,
    args: &[String],
    expect_ms: i64,
    ctl: &JobCtl,
    progress: &mut dyn FnMut(f64),
) -> Result<(), RunError> {
    if ctl.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    let mut cmd = crate::recorder::silent_command(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-nostats", "-progress", "pipe:1"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Could not start FFmpeg: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    *ctl.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    // A cancel that landed between the check above and storing the child.
    if ctl.is_cancelled() {
        ctl.cancel();
    }

    let err_tail = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut buf);
        }
        session::tail(&buf, 600)
    });
    if let Some(out) = stdout {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(v) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
                if let Ok(us) = v.trim().parse::<i64>() {
                    progress((us as f64 / 1000.0 / expect_ms.max(1) as f64).clamp(0.0, 1.0));
                }
            }
        }
    }
    let status = {
        let mut guard = ctl.child.lock().unwrap_or_else(|e| e.into_inner());
        let r = guard.as_mut().map(|c| c.wait());
        *guard = None;
        r
    };
    let tail = err_tail.join().unwrap_or_default();
    if ctl.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    match status {
        Some(Ok(s)) if s.success() => Ok(()),
        Some(Ok(_)) => Err(RunError::Failed(if tail.trim().is_empty() {
            "FFmpeg stopped with an error".into()
        } else {
            format!("FFmpeg: {}", tail.trim())
        })),
        Some(Err(e)) => Err(RunError::Failed(format!("FFmpeg: {}", e))),
        None => Err(RunError::Failed("FFmpeg did not start".into())),
    }
}

/// Removes the job's work folder however the job ends.
struct WorkDir(PathBuf);
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Removes a partial output unless the job finished.
struct Partial(Option<PathBuf>);
impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

fn path_arg(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// Run an export to the end. `recordings` is the recordings folder (it holds
/// `sessions/` and `exports/`). Blocking: call it from a thread. `progress`
/// gets the overall fraction.
pub fn run_export(
    ffmpeg: &str,
    recordings: &Path,
    job_id: &str,
    req: &ExportRequest,
    cursor: &[CursorSample],
    caps: &EncoderCaps,
    ctl: &JobCtl,
    progress: &mut dyn FnMut(f64),
) -> Result<ExportRecord, RunError> {
    crate::edit::check_id(&req.session_id)?;
    let session_dir = crate::recorder::sessions_root(recordings).join(&req.session_id);
    let m = session::read_manifest(&session_dir)?;
    let exports = recordings.join(EXPORTS_DIR);
    let work = exports.join(WORK_DIR).join(job_id);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {}", work.display(), e))?;
    let _work = WorkDir(work.clone());

    if let Some(tracks) = &req.raw_tracks {
        return run_raw(ffmpeg, recordings, &session_dir, &m, job_id, req, tracks, &work, ctl, progress);
    }

    // Share settles the height from the plan's length first.
    let mut share: Option<SharePlan> = None;
    if req.size == SizeMode::Share {
        let p0 = build_plan(&m, req, cursor, None)?;
        share = Some(share_plan(p0.output_ms(), p0.has_audio())?);
    }
    let plan = build_plan(&m, req, cursor, share.map(|s| s.p))?;

    // Inputs.
    let mut input_args: Vec<String> = Vec::new();
    for (i, inp) in plan.inputs.iter().enumerate() {
        let list = work.join(format!("input-{}.ffconcat", i));
        std::fs::write(&list, concat_list_text(&session_dir, &inp.entries))
            .map_err(|e| format!("{}: {}", list.display(), e))?;
        input_args.extend(["-f", "concat", "-safe", "0", "-i"].iter().map(|s| s.to_string()));
        input_args.push(path_arg(&list));
    }
    if let Some((w, h, shape)) = plan.mask {
        let mask = work.join("mask.pgm");
        std::fs::write(&mask, mask_pgm(w, h, shape)).map_err(|e| format!("{}: {}", mask.display(), e))?;
        input_args.extend(
            ["-loop", "1", "-framerate", "1", "-t"]
                .iter()
                .map(|s| s.to_string())
                .chain([secs(plan.composed_ms() + 1_000), "-i".into(), path_arg(&mask)]),
        );
    }
    let flag = filter_script_flag(&ffmpeg_version_line(ffmpeg));

    let title = req
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| req.layout.name().to_string());
    std::fs::create_dir_all(&exports).map_err(|e| format!("{}: {}", exports.display(), e))?;
    let first_piece = plan.out.pieces.first().map(|p| p.from).unwrap_or(req.from_ms);
    let final_path = unique_path(&exports, &export_stem(first_piece, &title), ".mp4");
    let part = final_path.with_extension("mp4.part");
    let mut partial = Partial(Some(part.clone()));
    let out_ms = plan.output_ms();
    let out_t = secs(out_ms);

    let script = work.join("graph.txt");
    let common_out = |with_audio: bool| -> Vec<String> {
        let mut v: Vec<String> = vec!["-map".into(), "[vout]".into()];
        if with_audio && plan.has_audio() {
            v.extend(["-map".into(), "[aout]".into()]);
        }
        v.extend(["-t".into(), out_t.clone()]);
        v
    };

    match share {
        Some(s) => {
            // Pass 1 has no sound: its graph must not leave [aout] unconnected.
            let g1 = work.join("graph-pass1.txt");
            std::fs::write(&g1, filter_graph(&plan, false)).map_err(|e| format!("{}: {}", g1.display(), e))?;
            std::fs::write(&script, filter_graph(&plan, true)).map_err(|e| format!("{}: {}", script.display(), e))?;
            let log = path_arg(&work.join("pass"));
            let rate = vec!["-c:v".to_string(), "libx264".into(), "-preset".into(), "medium".into(), "-b:v".into(), format!("{}k", s.video_kbps), "-maxrate".into(), format!("{}k", s.video_kbps * 2), "-bufsize".into(), format!("{}k", s.video_kbps * 2), "-passlogfile".into(), log];
            let mut a1 = input_args.clone();
            a1.extend([flag.to_string(), path_arg(&g1)]);
            a1.extend(common_out(false));
            a1.extend(rate.clone());
            a1.extend(["-pass".into(), "1".into(), "-an".into(), "-f".into(), "null".into(), "NUL".into()]);
            run_ffmpeg(ffmpeg, &a1, out_ms, ctl, &mut |f| progress(f * 0.5))?;
            let mut a2 = input_args.clone();
            a2.extend([flag.to_string(), path_arg(&script)]);
            a2.extend(common_out(true));
            a2.extend(rate);
            a2.extend(["-pass".into(), "2".into()]);
            if plan.has_audio() {
                a2.extend(["-c:a".into(), "aac".into(), "-b:a".into(), format!("{}k", s.audio_kbps)]);
            }
            a2.extend(["-movflags".into(), "+faststart".into(), "-f".into(), "mp4".into(), path_arg(&part)]);
            run_ffmpeg(ffmpeg, &a2, out_ms, ctl, &mut |f| progress(0.5 + f * 0.5))?;
        }
        None => {
            std::fs::write(&script, filter_graph(&plan, true)).map_err(|e| format!("{}: {}", script.display(), e))?;
            let encoder = pick_encoder(caps, plan.width, plan.height);
            let attempt = |enc: Encoder, progress: &mut dyn FnMut(f64)| {
                let mut a = input_args.clone();
                a.extend([flag.to_string(), path_arg(&script)]);
                a.extend(common_out(true));
                a.extend(enc.args());
                if plan.has_audio() {
                    a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "160k".into()]);
                }
                a.extend(["-movflags".into(), "+faststart".into(), "-f".into(), "mp4".into(), path_arg(&part)]);
                run_ffmpeg(ffmpeg, &a, out_ms, ctl, progress)
            };
            match attempt(encoder, progress) {
                // A hardware encoder that is listed but won't open (no device,
                // a driver too old): x264 always works.
                Err(RunError::Failed(e)) if encoder != Encoder::X264 => {
                    eprintln!("[chirptime] export: {} failed, retrying with x264: {}", encoder.name(), e);
                    attempt(Encoder::X264, progress)?;
                }
                r => r?,
            }
        }
    }

    std::fs::rename(&part, &final_path).map_err(|e| format!("Could not finish the file: {}", e))?;
    partial.0 = None;
    let thumb = final_path.with_extension("jpg");
    let thumb_ok = crate::recorder::silent_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&final_path)
        .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4"])
        .arg(&thumb)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let rec = ExportRecord {
        id: job_id.to_string(),
        session_id: req.session_id.clone(),
        from: req.from_ms,
        to: req.to_ms,
        layout: req.layout.key().into(),
        path: rel_to(recordings, &final_path),
        bytes: std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0),
        duration_ms: out_ms,
        created_epoch_ms: session::now_epoch_ms(),
        title,
        thumb: thumb_ok.then(|| rel_to(recordings, &thumb)),
    };
    add_to_index(recordings, rec.clone())?;
    progress(1.0);
    Ok(rec)
}

#[allow(clippy::too_many_arguments)]
fn run_raw(
    ffmpeg: &str,
    recordings: &Path,
    session_dir: &Path,
    m: &SessionManifest,
    job_id: &str,
    req: &ExportRequest,
    tracks: &[String],
    work: &Path,
    ctl: &JobCtl,
    progress: &mut dyn FnMut(f64),
) -> Result<ExportRecord, RunError> {
    if tracks.is_empty() {
        return Err(RunError::Failed("Pick at least one track".into()));
    }
    let whole = session_span(m);
    let pieces = kept_pieces(m, req.from_ms.max(whole.from), req.to_ms.min(whole.to), false);
    if pieces.is_empty() {
        return Err(RunError::Failed(NOTHING_LEFT.into()));
    }
    let exports = recordings.join(EXPORTS_DIR);
    let title = req.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Raw tracks".into());
    let folder = unique_path(&exports, &export_stem(pieces[0].from, &title), "");
    let part = PathBuf::from(format!("{}.part", folder.to_string_lossy()));
    std::fs::create_dir_all(&part).map_err(|e| format!("{}: {}", part.display(), e))?;
    let mut partial = Partial(Some(part.clone()));
    let n = tracks.len() as f64;
    for (i, id) in tracks.iter().enumerate() {
        let t = m
            .tracks
            .iter()
            .find(|t| &t.id == id)
            .ok_or_else(|| format!("This session has no track named {}", id))?;
        // Raw cuts only whole-session deletes and this track's own.
        let keep = subtract_spans(&pieces, &track_holes(m, &t.id));
        let entries = raw_entries(t, &keep);
        if entries.is_empty() {
            continue;
        }
        let list = work.join(format!("raw-{}.ffconcat", i));
        std::fs::write(&list, concat_list_text(session_dir, &entries)).map_err(|e| format!("{}: {}", list.display(), e))?;
        let expect: i64 = entries.iter().map(|e| e.duration_ms).sum();
        let args: Vec<String> = vec![
            "-f".into(), "concat".into(), "-safe".into(), "0".into(), "-i".into(), path_arg(&list),
            "-map".into(), "0".into(), "-c".into(), "copy".into(), "-movflags".into(), "+faststart".into(),
            "-f".into(), "mp4".into(), path_arg(&part.join(format!("{}.mp4", t.id))),
        ];
        run_ffmpeg(ffmpeg, &args, expect, ctl, &mut |f| progress((i as f64 + f) / n))?;
    }
    std::fs::rename(&part, &folder).map_err(|e| format!("Could not finish the folder: {}", e))?;
    partial.0 = None;
    let bytes = std::fs::read_dir(&folder)
        .map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0);
    let rec = ExportRecord {
        id: job_id.to_string(),
        session_id: req.session_id.clone(),
        from: req.from_ms,
        to: req.to_ms,
        layout: "raw".into(),
        path: rel_to(recordings, &folder),
        bytes,
        duration_ms: total_ms(&pieces),
        created_epoch_ms: session::now_epoch_ms(),
        title,
        thumb: None,
    };
    add_to_index(recordings, rec.clone())?;
    progress(1.0);
    Ok(rec)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

type SharedState = Arc<tokio::sync::Mutex<crate::state::AppState>>;

static JOBS: Mutex<Option<HashMap<String, Arc<JobCtl>>>> = Mutex::new(None);

fn jobs() -> std::sync::MutexGuard<'static, Option<HashMap<String, Arc<JobCtl>>>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `find_ffmpeg` and `detect_encoders` each spawn a process; remember them.
fn tools() -> Result<(String, EncoderCaps), String> {
    static FOUND: Mutex<Option<(String, EncoderCaps)>> = Mutex::new(None);
    let mut f = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    if f.is_none() {
        let ffmpeg = crate::capture::find_ffmpeg().ok_or("FFmpeg was not found. Set it up in Settings")?;
        let caps = crate::recorder::detect_encoders(&ffmpeg);
        *f = Some((ffmpeg, caps));
    }
    f.clone().ok_or_else(|| "FFmpeg was not found".to_string())
}

async fn recordings_dir(state: &tauri::State<'_, SharedState>) -> PathBuf {
    let s = state.lock().await;
    crate::capture::get_output_dir(&s.config)
}

fn load(recordings: &Path, req: &ExportRequest) -> Result<(SessionManifest, Vec<CursorSample>), String> {
    crate::edit::check_id(&req.session_id)?;
    let dir = crate::recorder::sessions_root(recordings).join(&req.session_id);
    let m = session::read_manifest(&dir).map_err(|_| "This session could not be read".to_string())?;
    let cursor = crate::cursor::read_track(&dir, req.from_ms, req.to_ms);
    Ok((m, cursor))
}

#[tauri::command]
pub async fn estimate_export(
    state: tauri::State<'_, SharedState>,
    req: ExportRequest,
) -> Result<ExportEstimate, String> {
    let recordings = recordings_dir(&state).await;
    tokio::task::spawn_blocking(move || {
        let (m, cursor) = load(&recordings, &req)?;
        let caps = tools().map(|t| t.1).unwrap_or_default();
        Ok(estimate(&m, &req, &cursor, &caps))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Start an export in the background. Resolves to its job id at once; the job
/// reports through `export-progress` events.
#[tauri::command]
pub async fn start_export(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    req: ExportRequest,
) -> Result<String, String> {
    use tauri::Emitter;
    let recordings = recordings_dir(&state).await;
    let (ffmpeg, caps) = tools()?;
    // Refuse up front what can't run (an empty range, too long for Share).
    let (m, cursor) = {
        let r = recordings.clone();
        let q = req.clone();
        tokio::task::spawn_blocking(move || load(&r, &q)).await.map_err(|e| e.to_string())??
    };
    if req.raw_tracks.is_none() {
        let est = estimate(&m, &req, &cursor, &caps);
        if let Some(e) = est.error {
            return Err(e);
        }
    }
    let job_id = format!("exp-{}", session::now_epoch_ms());
    let ctl = Arc::new(JobCtl::default());
    jobs().get_or_insert_with(HashMap::new).insert(job_id.clone(), ctl.clone());

    let id = job_id.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let emit = |p: ExportProgress| {
            let _ = app.emit("export-progress", p);
        };
        let mut on_progress = |f: f64| {
            if last_emit.elapsed() < std::time::Duration::from_millis(250) && f < 1.0 {
                return;
            }
            last_emit = std::time::Instant::now();
            let el = started.elapsed().as_secs_f64();
            let eta = (f > 0.02).then(|| el / f * (1.0 - f));
            emit(ExportProgress {
                job_id: id.clone(),
                fraction: f,
                eta_seconds: eta,
                state: "running",
                error: None,
                output_path: None,
            });
        };
        on_progress(0.0);
        let result = run_export(&ffmpeg, &recordings, &id, &req, &cursor, &caps, &ctl, &mut on_progress);
        let done = match result {
            Ok(rec) => ExportProgress {
                job_id: id.clone(),
                fraction: 1.0,
                eta_seconds: Some(0.0),
                state: "done",
                error: None,
                output_path: Some(recordings.join(&rec.path).to_string_lossy().to_string()),
            },
            Err(RunError::Cancelled) => ExportProgress {
                job_id: id.clone(),
                fraction: 0.0,
                eta_seconds: None,
                state: "cancelled",
                error: Some(CANCELLED.into()),
                output_path: None,
            },
            Err(RunError::Failed(e)) => {
                eprintln!("[chirptime] export {} failed: {}", id, e);
                ExportProgress {
                    job_id: id.clone(),
                    fraction: 0.0,
                    eta_seconds: None,
                    state: "failed",
                    error: Some(e),
                    output_path: None,
                }
            }
        };
        if let Some(j) = jobs().as_mut() {
            j.remove(&id);
        }
        emit(done);
    });
    Ok(job_id)
}

/// Stop a running export; its partial file is deleted.
#[tauri::command]
pub async fn cancel_export(job_id: String) -> Result<(), String> {
    let ctl = jobs().as_ref().and_then(|j| j.get(&job_id).cloned());
    match ctl {
        Some(c) => {
            c.cancel();
            Ok(())
        }
        None => Err("That export is not running".into()),
    }
}

#[tauri::command]
pub async fn list_exports(state: tauri::State<'_, SharedState>) -> Result<Vec<ExportRecord>, String> {
    let recordings = recordings_dir(&state).await;
    tokio::task::spawn_blocking(move || list(&recordings))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_export(state: tauri::State<'_, SharedState>, id: String) -> Result<(), String> {
    let recordings = recordings_dir(&state).await;
    tokio::task::spawn_blocking(move || delete(&recordings, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Show the export in Explorer, selected. With `play`, open it in the default
/// player instead.
#[tauri::command]
pub async fn reveal_export(
    state: tauri::State<'_, SharedState>,
    id: String,
    play: Option<bool>,
) -> Result<(), String> {
    let recordings = recordings_dir(&state).await;
    let rec = read_index(&recordings)
        .into_iter()
        .find(|r| r.id == id)
        .ok_or("That export is gone")?;
    let path = record_path(&recordings, &rec.path)?;
    if !path.exists() {
        return Err("The file is no longer there".into());
    }
    open_in_shell(&path, play.unwrap_or(false))
}

#[cfg(target_os = "windows")]
fn open_in_shell(path: &Path, play: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let shown = crate::nativeui::display_path(path);
    let mut cmd = crate::recorder::silent_command("explorer");
    // explorer parses its own command line: `/select,"<path>"` must reach it verbatim.
    if play {
        cmd.raw_arg(format!("\"{}\"", shown));
    } else {
        cmd.raw_arg(format!("/select,\"{}\"", shown));
    }
    cmd.spawn().map(|_| ()).map_err(|e| format!("Could not open Explorer: {}", e))
}

#[cfg(not(target_os = "windows"))]
fn open_in_shell(_path: &Path, _play: bool) -> Result<(), String> {
    Err("Opening files is only implemented on Windows".into())
}
