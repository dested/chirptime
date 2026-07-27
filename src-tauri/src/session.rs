//! Session manifest: types, atomic (de)serialization, and directory-scan rebuild.
//!
//! The manifest is an *index*, never the only copy of the truth. Every fact in it
//! can be recovered from the files on disk: segment filenames carry wall-clock
//! time (`-strftime 1`) and `ffprobe` yields exact durations and stream params.
//! `rebuild_manifest_from_disk` is that recovery path and is what runs after a
//! crash, when no `session.json` was ever flushed.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MANIFEST_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "session.json";
pub const MARKERS_FILE: &str = "markers.jsonl";

/// Filename pattern handed to ffmpeg's `-strftime 1` segment muxer.
pub const SEGMENT_STRFTIME: &str = "%Y%m%d-%H%M%S";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub epoch_ms: i64,
    pub label: String,
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
    serde_json::from_str(&data).map_err(|e| format!("{}: {}", path.display(), e))
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

pub fn read_markers(session_dir: &Path) -> Vec<Marker> {
    let path = session_dir.join(MARKERS_FILE);
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    data.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Marker>(l).ok())
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

pub fn find_ffprobe() -> Option<String> {
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
    None
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
    manifest.markers = read_markers(session_dir);
    if manifest.markers.is_empty() {
        if let Some(p) = &prior {
            manifest.markers = p.markers.clone();
        }
    }

    let mut track_dirs: Vec<String> = std::fs::read_dir(session_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
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
