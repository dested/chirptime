//! Session manifest: types, atomic (de)serialization, and directory-scan rebuild.
//!
//! The manifest is an *index*, never the only copy of the truth. Every fact in it
//! can be recovered from the files on disk: segment filenames carry wall-clock
//! time (`-strftime 1`) and `ffprobe` yields exact durations and stream params.
//! `rebuild_manifest_from_disk` is that recovery path and is what runs after a
//! crash, when no `session.json` was ever flushed.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MANIFEST_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "session.json";
pub const MARKERS_FILE: &str = "markers.jsonl";
/// Durable record of every time-range delete (see `edit.rs`). One JSON `Cut` per
/// line, appended on every state change; the last line per id wins.
pub const CUTS_FILE: &str = "cuts.jsonl";

/// Filename pattern handed to ffmpeg's `-strftime 1` segment muxer.
pub const SEGMENT_STRFTIME: &str = "%Y%m%d-%H%M%S";
/// Cached thumbnails, `<session>/thumbs/<track_id>/<width>/<epoch_ms>.jpg`. Not
/// footage: the rebuild never indexes it, and `edit.rs` purges it under cuts.
pub const THUMBS_DIR: &str = "thumbs";

// ---------------------------------------------------------------------------
// Types (contract with the frontend — see plans/2026-07-27-continuous-recording.md §3)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MonitorRef {
    pub output_idx: u32,
    pub adapter: u32,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub file: String,
    pub start_epoch_ms: i64,
    pub duration_ms: i64,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub from_epoch_ms: i64,
    pub to_epoch_ms: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub epoch_ms: i64,
    pub label: String,
}

/// A deleted block of time. The session keeps its id and the cut is remembered
/// forever, so playback and the timeline can collapse the hole — and so the up to
/// one GOP of footage that survives at each keyframe-snapped edge stays masked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cut {
    /// `cut-<epoch_ms of creation>`
    pub id: String,
    pub from_epoch_ms: i64,
    pub to_epoch_ms: i64,
    /// None = every track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<String>>,
    pub created_epoch_ms: i64,
    /// Pending = inside the undo window, files parked in .trash. Final = files gone.
    pub state: CutState,
    #[serde(default)]
    pub freed_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CutState {
    Pending,
    Final,
    Undone,
}

/// Lifecycle of one track. Additive to the §3 schema so the UI can distinguish
/// "this track never started" from "this track is fine".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackStatus {
    Starting,
    Recording,
    /// Process died; the watchdog is bringing it back.
    Restarting,
    /// Ended normally with the session.
    Stopped,
    /// Gave up (too many restarts, or it never started at all).
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    /// `screen` | `webcam` | `mic` | `system-audio`
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<MonitorRef>,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Measured — the instant this track's first segment file appeared on disk,
    /// not the instant the process was spawned.
    pub start_epoch_ms: i64,
    pub dir: String,
    pub segments: Vec<Segment>,
    pub gaps: Vec<Gap>,

    // ---- additive fields (not in the §3 sketch, needed to surface failures) ----
    #[serde(default = "default_status")]
    pub status: TrackStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub restarts: u32,
}

fn default_status() -> TrackStatus {
    TrackStatus::Stopped
}

impl Track {
    /// Wall-clock end of the last indexed segment.
    pub fn end_epoch_ms(&self) -> Option<i64> {
        self.segments
            .last()
            .map(|s| s.start_epoch_ms + s.duration_ms)
    }

    pub fn total_bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.bytes).sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    pub version: u32,
    pub id: String,
    pub started_epoch_ms: i64,
    /// `null` while still recording.
    pub ended_epoch_ms: Option<i64>,
    pub segment_seconds: u32,
    /// Bounding box of the virtual desktop in **physical** pixels.
    pub desktop: Rect,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,

    // ---- additive ----
    /// Populated when the session stopped for a reason other than the user asking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// True when this manifest was reconstructed by scanning the directory.
    #[serde(default)]
    pub rebuilt: bool,
    /// Deleted ranges, `Pending` or `Final` (never `Undone`). `cuts.jsonl` is the
    /// durable copy; `read_manifest` and the rebuild repopulate this from it.
    #[serde(default)]
    pub cuts: Vec<Cut>,
}

impl SessionManifest {
    pub fn new(id: String, started_epoch_ms: i64, segment_seconds: u32, desktop: Rect) -> Self {
        Self {
            version: MANIFEST_VERSION,
            id,
            started_epoch_ms,
            ended_epoch_ms: None,
            segment_seconds,
            desktop,
            tracks: Vec::new(),
            markers: Vec::new(),
            error: None,
            rebuilt: false,
            cuts: Vec::new(),
        }
    }

    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn total_bytes(&self) -> u64 {
        self.tracks.iter().map(|t| t.total_bytes()).sum()
    }

    pub fn segment_count(&self) -> usize {
        self.tracks.iter().map(|t| t.segments.len()).sum()
    }
}

// ---------------------------------------------------------------------------
// Session id / time helpers
// ---------------------------------------------------------------------------

pub fn now_epoch_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

/// `2026-07-27_11-56-40` — the session directory name.
pub fn session_id_from(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y-%m-%d_%H-%M-%S").to_string()
}

/// Parse a session directory name back into an epoch. Used by the rebuild path
/// when no manifest exists.
pub fn epoch_from_session_id(id: &str) -> Option<i64> {
    let naive = chrono::NaiveDateTime::parse_from_str(id, "%Y-%m-%d_%H-%M-%S").ok()?;
    local_naive_to_epoch_ms(naive)
}

/// Parse a `-strftime 1` segment filename (`20260727-115640.mp4`) into an epoch.
/// The muxer formats in **local** time, so this resolves through the local zone.
pub fn epoch_from_segment_name(name: &str) -> Option<i64> {
    let stem = Path::new(name).file_stem()?.to_str()?;
    // Tolerate the `-NNN` suffix ffmpeg appends if two segments land in the same second.
    let stem = stem.get(..15).unwrap_or(stem);
    let naive = chrono::NaiveDateTime::parse_from_str(stem, "%Y%m%d-%H%M%S").ok()?;
    local_naive_to_epoch_ms(naive)
}

fn local_naive_to_epoch_ms(naive: chrono::NaiveDateTime) -> Option<i64> {
    use chrono::TimeZone;
    // DST fall-back makes one local hour ambiguous; take the earlier instant.
    match chrono::Local.from_local_datetime(&naive) {
        chrono::offset::LocalResult::Single(dt) => Some(dt.timestamp_millis()),
        chrono::offset::LocalResult::Ambiguous(dt, _) => Some(dt.timestamp_millis()),
        chrono::offset::LocalResult::None => None,
    }
}

// ---------------------------------------------------------------------------
// Atomic persistence
// ---------------------------------------------------------------------------

/// Write `session.json` via tmp + rename so a crash mid-write can never leave a
/// truncated manifest. `std::fs::rename` maps to `MoveFileEx(MOVEFILE_REPLACE_EXISTING)`
/// on Windows, so replacing the existing file is atomic.
pub fn write_manifest(session_dir: &Path, manifest: &SessionManifest) -> Result<(), String> {
    let json = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    let tmp = session_dir.join("session.json.tmp");
    let final_path = session_dir.join(MANIFEST_FILE);
    std::fs::write(&tmp, json.as_bytes()).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, &final_path)
        .map_err(|e| format!("rename -> {}: {}", final_path.display(), e))
}

pub fn read_manifest(session_dir: &Path) -> Result<SessionManifest, String> {
    let path = session_dir.join(MANIFEST_FILE);
    let data = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let mut m: SessionManifest =
        serde_json::from_str(&data).map_err(|e| format!("{}: {}", path.display(), e))?;
    // The sidecar outranks whatever the manifest last carried.
    if session_dir.join(CUTS_FILE).is_file() {
        m.cuts = active_cuts(session_dir);
    }
    Ok(m)
}

pub fn append_marker(session_dir: &Path, marker: &Marker) -> Result<(), String> {
    use std::io::Write;
    let path = session_dir.join(MARKERS_FILE);
    let line = serde_json::to_string(marker).map_err(|e| e.to_string())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    writeln!(f, "{}", line).map_err(|e| e.to_string())
}

/// Every mark, in the order first made. A note edit appends the same instant
/// again with its new label; the last line per instant wins.
pub fn read_markers(session_dir: &Path) -> Vec<Marker> {
    let path = session_dir.join(MARKERS_FILE);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut out: Vec<Marker> = Vec::new();
    for mk in data
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Marker>(l).ok())
    {
        match out.iter_mut().find(|o| o.epoch_ms == mk.epoch_ms) {
            Some(o) => o.label = mk.label,
            None => out.push(mk),
        }
    }
    out
}

pub fn append_cut(session_dir: &Path, cut: &Cut) -> Result<(), String> {
    use std::io::Write;
    let path = session_dir.join(CUTS_FILE);
    let mut line = serde_json::to_string(cut).map_err(|e| e.to_string())?;
    // A crash mid-append can leave a torn last line with no newline; start on a
    // fresh one so this line is not glued onto it (and lost with it).
    let torn = std::fs::read(&path)
        .map(|b| b.last().is_some_and(|c| *c != b'\n'))
        .unwrap_or(false);
    if torn {
        line.insert(0, '\n');
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    writeln!(f, "{}", line).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

/// Every cut ever made in this session, in creation order, each in its latest
/// state (the last line per id wins). Includes `Undone` ones.
pub fn read_cuts(session_dir: &Path) -> Vec<Cut> {
    let path = session_dir.join(CUTS_FILE);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut out: Vec<Cut> = Vec::new();
    for c in data
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Cut>(l).ok())
    {
        match out.iter_mut().find(|o| o.id == c.id) {
            Some(o) => *o = c,
            None => out.push(c),
        }
    }
    out.sort_by_key(|c| (c.created_epoch_ms, c.id.clone()));
    out
}

/// The cuts a manifest carries: everything but `Undone`.
pub fn active_cuts(session_dir: &Path) -> Vec<Cut> {
    read_cuts(session_dir)
        .into_iter()
        .filter(|c| c.state != CutState::Undone)
        .collect()
}

// ---------------------------------------------------------------------------
// ffprobe
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ProbeResult {
    pub duration_ms: i64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub has_video: bool,
}

/// Locate `ffprobe.exe`. **Mirrors `capture::find_ffmpeg`'s candidate order** —
/// hand-picked folder, then PATH and the two classic install dirs, then the
/// app-managed install. The two must stay in step: the recorder resolves both,
/// and an ffmpeg without a matching ffprobe records segments it cannot index.
pub fn find_ffprobe() -> Option<String> {
    if let Some(p) = crate::settings::ffmpeg_dir_override() {
        let exe = Path::new(&p).join("ffprobe.exe");
        if exe.is_file() {
            return Some(exe.to_string_lossy().to_string());
        }
    }
    if crate::recorder::silent_command("ffprobe")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("ffprobe".to_string());
    }
    for c in [
        r"C:\ffmpeg\bin\ffprobe.exe",
        r"C:\Program Files\ffmpeg\bin\ffprobe.exe",
    ] {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    crate::ffmpeg_install::managed("ffprobe.exe")
}

/// ffprobe one closed segment. Returns `Err` for a file that is missing,
/// zero-length, or has no decodable duration — the caller treats that as corrupt
/// and records a gap rather than writing a bogus manifest entry.
pub fn probe_segment(ffprobe: &str, path: &Path) -> Result<ProbeResult, String> {
    let out = crate::recorder::silent_command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-show_entries",
            "stream=codec_type,codec_name,width,height,avg_frame_rate,duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("ffprobe spawn failed: {}", e))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("ffprobe failed: {}", tail(&stderr, 200)));
    }

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe json: {}", e))?;

    let mut res = ProbeResult::default();

    // Container duration is the authoritative one; fall back to the stream's.
    let mut duration_s = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    if let Some(streams) = v["streams"].as_array() {
        for s in streams {
            let ctype = s["codec_type"].as_str().unwrap_or("");
            if duration_s <= 0.0 {
                if let Some(d) = s["duration"].as_str().and_then(|d| d.parse::<f64>().ok()) {
                    duration_s = d;
                }
            }
            if ctype == "video" && !res.has_video {
                res.has_video = true;
                res.width = s["width"].as_u64().unwrap_or(0) as u32;
                res.height = s["height"].as_u64().unwrap_or(0) as u32;
                res.codec = s["codec_name"].as_str().unwrap_or("").to_string();
                res.fps = parse_rational_fps(s["avg_frame_rate"].as_str().unwrap_or(""));
            } else if ctype == "audio" && res.codec.is_empty() {
                res.codec = s["codec_name"].as_str().unwrap_or("").to_string();
            }
        }
    }

    if duration_s <= 0.0 {
        return Err("segment has zero/unknown duration (corrupt or truncated)".to_string());
    }

    res.duration_ms = (duration_s * 1000.0).round() as i64;
    Ok(res)
}

fn parse_rational_fps(s: &str) -> u32 {
    let mut parts = s.split('/');
    let n: f64 = parts.next().unwrap_or("0").parse().unwrap_or(0.0);
    let d: f64 = parts.next().unwrap_or("1").parse().unwrap_or(1.0);
    if d == 0.0 {
        0
    } else {
        (n / d).round() as u32
    }
}

pub fn tail(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        // Slice on a char boundary so a multi-byte tail can't panic.
        let start = s.len() - n;
        let start = (start..s.len())
            .find(|i| s.is_char_boundary(*i))
            .unwrap_or(s.len());
        s[start..].to_string()
    }
}

// ---------------------------------------------------------------------------
// Directory-scan rebuild (crash recovery)
// ---------------------------------------------------------------------------

/// Kind + label inferred from a track directory name alone.
fn kind_for_dir(dir: &str) -> (&'static str, String) {
    if let Some(n) = dir.strip_prefix("screen-") {
        ("screen", format!("Screen {}", n))
    } else {
        match dir {
            "webcam" => ("webcam", "Webcam".to_string()),
            "mic" => ("mic", "Microphone".to_string()),
            "sysaudio" => ("system-audio", "System audio".to_string()),
            other => ("screen", other.to_string()),
        }
    }
}

/// Reconstruct a session purely from what is on disk. Does not read `session.json`
/// at all — that is the point. Any manifest that *is* present is used only to
/// recover things the files cannot carry (labels, monitor rects, markers).
pub fn rebuild_manifest_from_disk(session_dir: &Path) -> Result<SessionManifest, String> {
    if !session_dir.is_dir() {
        return Err(format!("not a directory: {}", session_dir.display()));
    }
    let ffprobe = find_ffprobe().ok_or("ffprobe not found")?;

    let id = session_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Anything the filenames can't carry, borrow from an existing manifest if one
    // survived. Never trust its segment list.
    let prior = read_manifest(session_dir).ok();

    let started = epoch_from_session_id(&id)
        .or_else(|| prior.as_ref().map(|p| p.started_epoch_ms))
        .unwrap_or(0);

    let mut manifest = SessionManifest::new(
        id,
        started,
        prior.as_ref().map(|p| p.segment_seconds).unwrap_or(60),
        prior.as_ref().map(|p| p.desktop).unwrap_or_default(),
    );
    manifest.rebuilt = true;
    manifest.cuts = if session_dir.join(CUTS_FILE).is_file() {
        active_cuts(session_dir)
    } else {
        prior.as_ref().map(|p| p.cuts.clone()).unwrap_or_default()
    };
    manifest.markers = read_markers(session_dir);
    if manifest.markers.is_empty() {
        if let Some(p) = &prior {
            manifest.markers = p.markers.clone();
        }
    }
    // markers.jsonl is append-only, so a marker inside a deleted range is still
    // in it. An all-track cut is what says it is gone; a per-track cut keeps it.
    let cuts = manifest.cuts.clone();
    manifest.markers.retain(|mk| {
        !cuts.iter().any(|c| {
            c.tracks.is_none() && mk.epoch_ms >= c.from_epoch_ms && mk.epoch_ms < c.to_epoch_ms
        })
    });

    let mut track_dirs: Vec<String> = std::fs::read_dir(session_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        // `.trash` holds files parked by a pending delete, `thumbs` cached
        // JPEGs; neither is footage.
        .filter(|n| !n.starts_with('.') && n != THUMBS_DIR)
        .collect();
    track_dirs.sort();

    let mut latest_end = started;

    for dir_name in track_dirs {
        let dir = session_dir.join(&dir_name);
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.ends_with(".mp4") || l.ends_with(".m4a") || l.ends_with(".mka")
            })
            .collect();
        // The `-strftime` names sort lexicographically into chronological order.
        files.sort();
        if files.is_empty() {
            continue;
        }

        let (kind, default_label) = kind_for_dir(&dir_name);
        let prior_track = prior.as_ref().and_then(|p| p.tracks.iter().find(|t| t.id == dir_name));

        let mut segments: Vec<Segment> = Vec::new();
        let mut gaps: Vec<Gap> = Vec::new();
        let mut probe_meta: Option<ProbeResult> = None;
        let mut chain: Option<i64> = None;

        for name in &files {
            let path = dir.join(name);
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let wall = epoch_from_segment_name(name);

            match probe_segment(&ffprobe, &path) {
                Ok(p) => {
                    if probe_meta.is_none() && p.has_video {
                        probe_meta = Some(p.clone());
                    } else if probe_meta.is_none() {
                        probe_meta = Some(p.clone());
                    }
                    // Chain from the previous segment's end (segments are gapless by
                    // construction), but re-anchor whenever the filename's wall clock
                    // disagrees by more than a second — that means a real gap.
                    let wall = wall.unwrap_or_else(|| chain.unwrap_or(started));
                    let start = match chain {
                        Some(c) if (c - wall).abs() <= 1500 => c,
                        Some(c) => {
                            if wall > c {
                                gaps.push(Gap {
                                    from_epoch_ms: c,
                                    to_epoch_ms: wall,
                                    reason: "missing segments (rebuilt from disk)".to_string(),
                                });
                            }
                            wall
                        }
                        None => wall,
                    };
                    segments.push(Segment {
                        file: name.clone(),
                        start_epoch_ms: start,
                        duration_ms: p.duration_ms,
                        bytes,
                    });
                    chain = Some(start + p.duration_ms);
                }
                Err(e) => {
                    // Corrupt / truncated / still-open file. Record it as a gap of
                    // one segment length rather than inventing a duration.
                    let from = wall.or(chain).unwrap_or(started);
                    gaps.push(Gap {
                        from_epoch_ms: from,
                        to_epoch_ms: from + manifest.segment_seconds as i64 * 1000,
                        reason: format!("unreadable segment {}: {}", name, e),
                    });
                }
            }
        }

        let meta = probe_meta.unwrap_or_default();
        let start_epoch_ms = segments
            .first()
            .map(|s| s.start_epoch_ms)
            .unwrap_or(started);
        if let Some(end) = segments.last().map(|s| s.start_epoch_ms + s.duration_ms) {
            latest_end = latest_end.max(end);
        }

        manifest.tracks.push(Track {
            id: dir_name.clone(),
            kind: prior_track.map(|t| t.kind.clone()).unwrap_or_else(|| kind.to_string()),
            label: prior_track
                .map(|t| t.label.clone())
                .unwrap_or(default_label),
            monitor: prior_track.and_then(|t| t.monitor),
            codec: if meta.codec.is_empty() {
                prior_track.map(|t| t.codec.clone()).unwrap_or_default()
            } else {
                meta.codec.clone()
            },
            width: if meta.width > 0 { meta.width } else { prior_track.map(|t| t.width).unwrap_or(0) },
            height: if meta.height > 0 { meta.height } else { prior_track.map(|t| t.height).unwrap_or(0) },
            fps: if meta.fps > 0 { meta.fps } else { prior_track.map(|t| t.fps).unwrap_or(0) },
            start_epoch_ms,
            dir: dir_name,
            segments,
            gaps,
            status: TrackStatus::Stopped,
            error: None,
            restarts: 0,
        });
    }

    manifest.ended_epoch_ms = Some(latest_end);
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Thumbnails (the file server's `/thumb` route)
// ---------------------------------------------------------------------------

pub const THUMB_MIN_W: u32 = 64;
pub const THUMB_MAX_W: u32 = 960;
/// Width when the request names none.
pub const THUMB_DEFAULT_W: u32 = 320;
/// The seek never lands closer than this to a segment's end: past the last
/// frame ffmpeg writes nothing and fails.
const THUMB_END_MARGIN_MS: i64 = 500;

static THUMB_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Why there is no thumbnail. Every one of these is a 404.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbMiss {
    /// No readable manifest.
    NoSession,
    /// No such track, or one with no picture (mic, system audio).
    NoTrack,
    /// The instant was deleted.
    Cut,
    /// No segment of the track covers the instant.
    Gap,
    /// The segment is still being written (no moov yet) or has gone.
    NotReady,
    /// ffmpeg decoded no frame at that offset.
    NoFrame,
}

/// The segment frame that shows one instant of one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbFrame {
    pub dir: String,
    pub file: String,
    /// Seek offset into `file`.
    pub offset_ms: i64,
}

/// A thumbnail that has to be extracted: `render_thumb`'s input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbJob {
    pub track_id: String,
    pub epoch_ms: i64,
    pub width: u32,
    pub src: PathBuf,
    pub offset_ms: i64,
    pub dest: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThumbLookup {
    /// Already in the cache.
    Cached(PathBuf),
    Render(ThumbJob),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Thumb {
    Ready(PathBuf),
    Missing(ThumbMiss),
}

pub fn clamp_thumb_width(w: Option<u32>) -> u32 {
    w.unwrap_or(THUMB_DEFAULT_W).clamp(THUMB_MIN_W, THUMB_MAX_W)
}

/// `<session>/thumbs/<track_id>/<width>/<epoch_ms>.jpg`
pub fn thumb_path(session_dir: &Path, track_id: &str, width: u32, epoch_ms: i64) -> PathBuf {
    session_dir
        .join(THUMBS_DIR)
        .join(track_id)
        .join(width.to_string())
        .join(format!("{}.jpg", epoch_ms))
}

/// Does `c` hide `epoch_ms` on `track_id`?
pub fn cut_hides(c: &Cut, track_id: &str, epoch_ms: i64) -> bool {
    c.state != CutState::Undone
        && c.tracks.as_ref().map_or(true, |ts| ts.iter().any(|t| t == track_id))
        && epoch_ms >= c.from_epoch_ms
        && epoch_ms < c.to_epoch_ms
}

/// Which segment frame shows `epoch_ms` on `track_id`. Pure. Segments are
/// ragged and can overlap; where two cover the instant, the later one wins.
pub fn resolve_thumb(
    m: &SessionManifest,
    track_id: &str,
    epoch_ms: i64,
) -> Result<ThumbFrame, ThumbMiss> {
    let track = m
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .filter(|t| t.kind == "screen" || t.kind == "webcam")
        .ok_or(ThumbMiss::NoTrack)?;
    if m.cuts.iter().any(|c| cut_hides(c, track_id, epoch_ms)) {
        return Err(ThumbMiss::Cut);
    }
    let seg = track
        .segments
        .iter()
        .filter(|s| s.start_epoch_ms <= epoch_ms && epoch_ms < s.start_epoch_ms + s.duration_ms)
        .max_by_key(|s| s.start_epoch_ms)
        .ok_or(ThumbMiss::Gap)?;
    let offset_ms = (epoch_ms - seg.start_epoch_ms)
        .min(seg.duration_ms - THUMB_END_MARGIN_MS)
        .max(0);
    Ok(ThumbFrame {
        dir: track.dir.clone(),
        file: seg.file.clone(),
        offset_ms,
    })
}

/// Resolve a `/thumb` request against the manifest on disk (for the live
/// session too: its last flush is recent enough) with the cuts sidecar applied.
/// The cache is only consulted once the instant is known not to be cut.
pub fn thumb_lookup(
    session_dir: &Path,
    track_id: &str,
    epoch_ms: i64,
    width: u32,
) -> Result<ThumbLookup, ThumbMiss> {
    let m = read_manifest(session_dir).map_err(|_| ThumbMiss::NoSession)?;
    let frame = resolve_thumb(&m, track_id, epoch_ms)?;
    let dest = thumb_path(session_dir, track_id, width, epoch_ms);
    if dest.is_file() {
        return Ok(ThumbLookup::Cached(dest));
    }
    let src = session_dir.join(&frame.dir).join(&frame.file);
    if !src.is_file() {
        return Err(ThumbMiss::NotReady);
    }
    Ok(ThumbLookup::Render(ThumbJob {
        track_id: track_id.to_string(),
        epoch_ms,
        width,
        src,
        offset_ms: frame.offset_ms,
        dest,
    }))
}

/// Extract one frame into the cache: a temp name, then a rename, so a reader
/// never sees half a JPEG. A cut made while ffmpeg ran wins: the thumbnail is
/// checked against `cuts.jsonl` after it lands, and `edit.rs` purges after it
/// appends the cut, so one of the two always removes it.
pub fn render_thumb(ffmpeg: &str, session_dir: &Path, job: &ThumbJob) -> Result<Thumb, String> {
    // Another request may have rendered it while this one waited for a slot.
    if job.dest.is_file() {
        return Ok(Thumb::Ready(job.dest.clone()));
    }
    let dir = job.dest.parent().ok_or("thumbnail path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    let seq = THUMB_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!("{}.jpg.{}-{}.tmp", job.epoch_ms, std::process::id(), seq));

    let out = crate::recorder::silent_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
        .arg(format!("{:.3}", job.offset_ms as f64 / 1000.0))
        .arg("-i")
        .arg(&job.src)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale={}:-2", job.width))
        .args(["-q:v", "6", "-f", "mjpeg"])
        .arg(&tmp)
        .output()
        .map_err(|e| format!("ffmpeg spawn failed: {}", e))?;
    let written = std::fs::metadata(&tmp).map(|m| m.len() > 0).unwrap_or(false);
    if !out.status.success() || !written {
        let _ = std::fs::remove_file(&tmp);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("moov atom not found") || !job.src.is_file() {
            return Ok(Thumb::Missing(ThumbMiss::NotReady));
        }
        if out.status.success() || stderr.contains("Nothing was written") {
            return Ok(Thumb::Missing(ThumbMiss::NoFrame));
        }
        return Err(format!("ffmpeg failed: {}", tail(&stderr, 300)));
    }
    if let Err(e) = std::fs::rename(&tmp, &job.dest) {
        let _ = std::fs::remove_file(&tmp);
        // A reader holding the one a racing request wrote blocks the replace;
        // that one is just as good.
        if !job.dest.is_file() {
            return Err(format!("rename -> {}: {}", job.dest.display(), e));
        }
    }
    if active_cuts(session_dir)
        .iter()
        .any(|c| cut_hides(c, &job.track_id, job.epoch_ms))
    {
        let _ = std::fs::remove_file(&job.dest);
        return Ok(Thumb::Missing(ThumbMiss::Cut));
    }
    Ok(Thumb::Ready(job.dest.clone()))
}
