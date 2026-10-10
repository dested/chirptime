//! Deleting a block of time from a recorded session, and deleting whole sessions.
//!
//! A delete is instant and undoable for `UNDO_WINDOW_MS`, then permanent:
//!
//! 1. `delete_range` plans the cut (`plan_cut`, pure), parks every segment that
//!    lies wholly inside it in `<session>/.trash/<cut_id>/<track_dir>/` (a rename,
//!    so instant), writes the manifest without them, and appends a `Pending` line
//!    to `cuts.jsonl`.
//! 2. `undo_cut` moves them back while the cut is still `Pending`.
//! 3. `finalize_cuts` trims the segments the cut only partly covers (ffmpeg stream
//!    copy, never a re-encode), deletes everything parked, and appends `Final`.
//!
//! The cut itself is remembered forever. Video edges snap outward to keyframes,
//! so up to one GOP of footage inside the range can physically survive; the cut
//! is what keeps it masked in playback and on the timeline.
//!
//! Ordering rule everywhere: the manifest is written before anything is deleted,
//! and nothing is deleted while the manifest still references it. Every step a
//! crash can interrupt is finished by `recover_sessions` on the next start.

use crate::session::{self, Cut, CutState, Marker, Segment, SessionManifest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// How long a delete can be undone.
pub const UNDO_WINDOW_MS: i64 = 10_000;
/// When the scheduled finalize / purge runs. A little after the undo window so a
/// last-second undo never races it.
pub const FINALIZE_AFTER_MS: u64 = 12_000;
/// A kept piece shorter than this is not worth a file: it is removed instead.
pub const MIN_PIECE_MS: i64 = 1_000;
/// Parking area for pending deletes, both inside a session and under the
/// sessions root (whole sessions).
pub const TRASH_DIR: &str = ".trash";
/// Mono 16kHz WAVs of the mic, not in the manifest (see `recorder::push_mic_sidecar_args`).
pub const SIDECAR_DIR: &str = "mic-16k";

pub const LIVE_SESSION_ERR: &str = "Stop recording before deleting from this session";
pub const UNDO_EXPIRED_ERR: &str = "That delete can no longer be undone";

const PLAN_FILE: &str = "plan.json";
const JOURNAL_FILE: &str = "finalize.json";
/// Suffix of a kept piece while ffmpeg is still writing it. Never `.mp4`, so a
/// crash can't leave something the rebuild would index.
const TMP_SUFFIX: &str = ".cut.tmp";

/// Every edit is a read-modify-write of one session's manifest and files, and
/// the scheduled finalize can fire while the user undoes. One process-wide lock
/// keeps them from interleaving; edits are rare, so contention is not a concern.
static EDIT_LOCK: Mutex<()> = Mutex::new(());

/// Whole sessions waiting to be purged: trash path → the token of the delete
/// that put it there, so a purge scheduled by an earlier delete never removes a
/// later one of the same session.
static TRASHED: Mutex<Option<HashMap<PathBuf, i64>>> = Mutex::new(None);

fn edit_lock() -> MutexGuard<'static, ()> {
    EDIT_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// What a cut does to one track. Each list holds the segment records as they
/// were when planned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackCut {
    /// The manifest track id, or `mic-16k` for the transcription sidecar.
    pub track_id: String,
    pub dir: String,
    /// True for the `mic-16k` WAVs, which the manifest does not list.
    #[serde(default)]
    pub sidecar: bool,
    /// Wholly inside the cut.
    pub remove: Vec<Segment>,
    /// Keep `[seg.start, from)`.
    pub trim_head: Vec<Segment>,
    /// Keep `[to, seg.end)`.
    pub trim_tail: Vec<Segment>,
    /// The cut lies strictly inside: keep both sides.
    pub split: Vec<Segment>,
}

impl TrackCut {
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty()
            && self.trim_head.is_empty()
            && self.trim_tail.is_empty()
            && self.split.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutPlan {
    pub from_epoch_ms: i64,
    pub to_epoch_ms: i64,
    /// Only tracks the cut actually touches.
    pub tracks: Vec<TrackCut>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CutReceipt {
    pub cut: Cut,
    pub freed_bytes_estimate: u64,
    pub undo_until_epoch_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionTrashReceipt {
    pub id: String,
    pub bytes: u64,
    pub undo_until_epoch_ms: i64,
}

/// `.trash/<cut_id>/plan.json`: everything undo needs to put the session back.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanFile {
    cut: Cut,
    plan: CutPlan,
    /// Markers that fell inside the range, dropped from the manifest.
    markers: Vec<Marker>,
}

/// `.trash/<cut_id>/finalize.json`, written once every kept piece exists and
/// before the manifest changes. Replaying it is idempotent, which is what makes
/// a crash mid-finalize recoverable by rolling forward.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Journal {
    freed_bytes: u64,
    replace: Vec<Replacement>,
    /// `(dir, file)` deleted outright.
    delete: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Replacement {
    dir: String,
    sidecar: bool,
    original: String,
    pieces: Vec<Segment>,
}

// ---------------------------------------------------------------------------
// Planning (pure)
// ---------------------------------------------------------------------------

/// Sort one track's segments against `[from, to)`. Uses each segment's own
/// start and duration: segments are ragged and can overlap, so nothing here
/// assumes a length or a neighbour.
pub fn classify_segments(
    track_id: &str,
    dir: &str,
    sidecar: bool,
    segments: &[Segment],
    from: i64,
    to: i64,
) -> TrackCut {
    let mut tc = TrackCut {
        track_id: track_id.to_string(),
        dir: dir.to_string(),
        sidecar,
        ..TrackCut::default()
    };
    for seg in segments {
        let s = seg.start_epoch_ms;
        let e = s + seg.duration_ms;
        if e <= from || s >= to {
            continue;
        }
        let keep_head = s < from && from - s >= MIN_PIECE_MS;
        let keep_tail = e > to && e - to >= MIN_PIECE_MS;
        let bucket = match (keep_head, keep_tail) {
            (false, false) => &mut tc.remove,
            (true, false) => &mut tc.trim_head,
            (false, true) => &mut tc.trim_tail,
            (true, true) => &mut tc.split,
        };
        bucket.push(seg.clone());
    }
    tc
}

fn covers(tracks: Option<&[String]>, id: &str) -> bool {
    tracks.map_or(true, |ts| ts.iter().any(|t| t == id))
}

/// Does a cut over `tracks` cover the mic (and so its 16kHz sidecar)?
fn covers_mic(m: &SessionManifest, tracks: Option<&[String]>) -> bool {
    match tracks {
        None => true,
        Some(ts) => m
            .tracks
            .iter()
            .any(|t| t.kind == "mic" && ts.iter().any(|x| *x == t.id)),
    }
}

/// Earliest and latest instant the session holds footage or claims to span.
pub fn session_span(m: &SessionManifest) -> (i64, i64) {
    let starts = m.tracks.iter().flat_map(|t| t.segments.iter().map(|s| s.start_epoch_ms));
    let ends = m
        .tracks
        .iter()
        .flat_map(|t| t.segments.iter().map(|s| s.start_epoch_ms + s.duration_ms));
    let start = starts.fold(m.started_epoch_ms, i64::min);
    let end = ends.fold(m.ended_epoch_ms.unwrap_or(start), i64::max);
    (start, end)
}

fn plan_tracks(m: &SessionManifest, from: i64, to: i64, tracks: Option<&[String]>) -> CutPlan {
    CutPlan {
        from_epoch_ms: from,
        to_epoch_ms: to,
        tracks: m
            .tracks
            .iter()
            .filter(|t| covers(tracks, &t.id))
            .map(|t| classify_segments(&t.id, &t.dir, false, &t.segments, from, to))
            .filter(|tc| !tc.is_empty())
            .collect(),
    }
}

/// Plan a delete of `[from, to)` over `tracks` (None = every track). Pure: reads
/// only the manifest, touches no file.
pub fn plan_cut(
    m: &SessionManifest,
    from: i64,
    to: i64,
    tracks: Option<&[String]>,
) -> Result<CutPlan, String> {
    if from >= to {
        return Err("The end of the range must come after its start".to_string());
    }
    if let Some(ts) = tracks {
        if ts.is_empty() {
            return Err("Pick at least one track to delete from".to_string());
        }
        for t in ts {
            if !m.tracks.iter().any(|x| x.id == *t) {
                return Err(format!("This session has no track named \"{}\"", t));
            }
        }
    }
    let (start, end) = session_span(m);
    if to <= start || from >= end {
        return Err("That range is outside this session".to_string());
    }
    Ok(plan_tracks(m, from, to, tracks))
}

// ---------------------------------------------------------------------------
// Small file helpers
// ---------------------------------------------------------------------------

/// Session ids come from the frontend and become paths.
pub fn check_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.starts_with('.')
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains(':')
    {
        return Err(format!("Not a session id: {}", id));
    }
    Ok(())
}

fn trash_dir(session_dir: &Path, cut_id: &str) -> PathBuf {
    session_dir.join(TRASH_DIR).join(cut_id)
}

fn write_json_atomic<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes()).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename -> {}: {}", path.display(), e))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Append one line to `<session>/logs/edit.log`. Logging must never fail an edit.
fn log_edit(session_dir: &Path, msg: &str) {
    use std::io::Write;
    let dir = session_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("edit.log"))
    {
        let _ = writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    }
    eprintln!("[chirptime] edit: {}", msg);
}

fn load_manifest(session_dir: &Path) -> Result<SessionManifest, String> {
    session::read_manifest(session_dir).or_else(|_| session::rebuild_manifest_from_disk(session_dir))
}

/// The `mic-16k` WAVs that can intersect `[from, to)`, as segment records: start
/// from the filename, duration from ffprobe.
fn sidecar_segments(session_dir: &Path, ffprobe: &str, from: i64, to: i64) -> Vec<Segment> {
    let dir = session_dir.join(SIDECAR_DIR);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_ascii_lowercase().ends_with(".wav"))
        .collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let Some(start) = session::epoch_from_segment_name(&name) else {
            continue;
        };
        // Sidecar segments run 10 minutes; anything starting an hour before the
        // range cannot reach it, and probing it would only cost time.
        if start >= to || start < from - 3_600_000 {
            continue;
        }
        let path = dir.join(&name);
        match session::probe_segment(ffprobe, &path) {
            Ok(p) => out.push(Segment {
                file: name,
                start_epoch_ms: start,
                duration_ms: p.duration_ms,
                bytes: file_len(&path),
            }),
            Err(e) => log_edit(session_dir, &format!("skipping sidecar {}: {}", name, e)),
        }
    }
    out
}

fn sidecar_plan(
    session_dir: &Path,
    m: &SessionManifest,
    from: i64,
    to: i64,
    tracks: Option<&[String]>,
) -> Option<TrackCut> {
    if !covers_mic(m, tracks) || !session_dir.join(SIDECAR_DIR).is_dir() {
        return None;
    }
    let ffprobe = session::find_ffprobe()?;
    let segs = sidecar_segments(session_dir, &ffprobe, from, to);
    let tc = classify_segments(SIDECAR_DIR, SIDECAR_DIR, true, &segs, from, to);
    (!tc.is_empty()).then_some(tc)
}

fn in_range(epoch_ms: i64, from: i64, to: i64) -> bool {
    epoch_ms >= from && epoch_ms < to
}

fn bytes_inside(seg: &Segment, from: i64, to: i64) -> u64 {
    if seg.duration_ms <= 0 {
        return 0;
    }
    let s = seg.start_epoch_ms;
    let e = s + seg.duration_ms;
    let overlap = (e.min(to) - s.max(from)).max(0);
    (seg.bytes as f64 * overlap as f64 / seg.duration_ms as f64).round() as u64
}

// ---------------------------------------------------------------------------
// delete_range / undo_cut
// ---------------------------------------------------------------------------

/// Delete `[from, to)` from the session in `session_dir`. The caller has already
/// refused the live session (`LIVE_SESSION_ERR`). Schedule `finalize_due_cuts`
/// `FINALIZE_AFTER_MS` later.
pub fn delete_range(
    session_dir: &Path,
    from: i64,
    to: i64,
    tracks: Option<Vec<String>>,
) -> Result<CutReceipt, String> {
    let _g = edit_lock();
    let original = load_manifest(session_dir)?;
    let tracks = tracks.as_deref();
    let mut plan = plan_cut(&original, from, to, tracks)?;
    if let Some(tc) = sidecar_plan(session_dir, &original, from, to, tracks) {
        plan.tracks.push(tc);
    }

    let now = session::now_epoch_ms();
    let known = session::read_cuts(session_dir);
    let mut stamp = now;
    let cut_id = loop {
        let id = format!("cut-{}", stamp);
        if !known.iter().any(|c| c.id == id) && !trash_dir(session_dir, &id).exists() {
            break id;
        }
        stamp += 1;
    };
    let cut = Cut {
        id: cut_id.clone(),
        from_epoch_ms: from,
        to_epoch_ms: to,
        tracks: tracks.map(|t| t.to_vec()),
        created_epoch_ms: now,
        state: CutState::Pending,
        freed_bytes: 0,
    };
    let markers: Vec<Marker> = original
        .markers
        .iter()
        .filter(|mk| in_range(mk.epoch_ms, from, to))
        .cloned()
        .collect();

    // Plan first, so any file parked in .trash always has the record that says
    // where it came from.
    let trash = trash_dir(session_dir, &cut_id);
    std::fs::create_dir_all(&trash).map_err(|e| format!("{}: {}", trash.display(), e))?;
    let pf = PlanFile {
        cut: cut.clone(),
        plan: plan.clone(),
        markers,
    };
    if let Err(e) = write_json_atomic(&trash.join(PLAN_FILE), &pf) {
        let _ = std::fs::remove_dir_all(&trash);
        return Err(e);
    }

    // Park everything wholly inside the cut. Same volume, so a rename.
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let rollback = |moved: &[(PathBuf, PathBuf)]| {
        for (src, dst) in moved.iter().rev() {
            let _ = std::fs::rename(dst, src);
        }
        let _ = std::fs::remove_dir_all(&trash);
    };
    for tc in &plan.tracks {
        for seg in &tc.remove {
            let src = session_dir.join(&tc.dir).join(&seg.file);
            if !src.is_file() {
                continue;
            }
            let dst_dir = trash.join(&tc.dir);
            let dst = dst_dir.join(&seg.file);
            let res = std::fs::create_dir_all(&dst_dir).and_then(|_| std::fs::rename(&src, &dst));
            if let Err(e) = res {
                rollback(&moved);
                return Err(format!("Could not move {}: {}", src.display(), e));
            }
            moved.push((src, dst));
        }
    }

    let mut m = original.clone();
    for tc in plan.tracks.iter().filter(|tc| !tc.sidecar) {
        if let Some(t) = m.tracks.iter_mut().find(|t| t.id == tc.track_id) {
            t.segments.retain(|s| !tc.remove.iter().any(|r| r.file == s.file));
        }
    }
    m.markers.retain(|mk| !in_range(mk.epoch_ms, from, to));
    m.cuts.push(cut.clone());
    if let Err(e) = session::write_manifest(session_dir, &m) {
        rollback(&moved);
        return Err(e);
    }
    if let Err(e) = session::append_cut(session_dir, &cut) {
        let _ = session::write_manifest(session_dir, &original);
        rollback(&moved);
        return Err(e);
    }

    let freed_bytes_estimate = plan
        .tracks
        .iter()
        .map(|tc| {
            let removed: u64 = tc.remove.iter().map(|s| s.bytes).sum();
            let trimmed: u64 = tc
                .trim_head
                .iter()
                .chain(&tc.trim_tail)
                .chain(&tc.split)
                .map(|s| bytes_inside(s, from, to))
                .sum();
            removed + trimmed
        })
        .sum();

    Ok(CutReceipt {
        cut,
        freed_bytes_estimate,
        undo_until_epoch_ms: now + UNDO_WINDOW_MS,
    })
}

/// Put back everything a plan parked: files, segment records, markers. Does not
/// write the manifest.
fn restore_parked(session_dir: &Path, trash: &Path, pf: &PlanFile, m: &mut SessionManifest) -> Result<(), String> {
    for tc in &pf.plan.tracks {
        for seg in &tc.remove {
            let parked = trash.join(&tc.dir).join(&seg.file);
            let home = session_dir.join(&tc.dir).join(&seg.file);
            if parked.is_file() && !home.exists() {
                std::fs::rename(&parked, &home)
                    .map_err(|e| format!("Could not restore {}: {}", home.display(), e))?;
            }
            if tc.sidecar {
                continue;
            }
            if let Some(t) = m.tracks.iter_mut().find(|t| t.id == tc.track_id) {
                if !t.segments.iter().any(|s| s.file == seg.file) {
                    t.segments.push(seg.clone());
                }
            }
        }
    }
    for t in m.tracks.iter_mut() {
        t.segments.sort_by_key(|s| s.start_epoch_ms);
    }
    for mk in &pf.markers {
        if !m.markers.contains(mk) {
            m.markers.push(mk.clone());
        }
    }
    m.markers.sort_by_key(|mk| mk.epoch_ms);
    Ok(())
}

/// Undo a delete while it is still `Pending`.
pub fn undo_cut(session_dir: &Path, cut_id: &str) -> Result<SessionManifest, String> {
    let _g = edit_lock();
    let cut = session::read_cuts(session_dir)
        .into_iter()
        .find(|c| c.id == cut_id)
        .filter(|c| c.state == CutState::Pending)
        .ok_or(UNDO_EXPIRED_ERR)?;
    let trash = trash_dir(session_dir, cut_id);
    let pf: PlanFile = read_json(&trash.join(PLAN_FILE)).ok_or(UNDO_EXPIRED_ERR)?;

    let mut m = load_manifest(session_dir)?;
    restore_parked(session_dir, &trash, &pf, &mut m)?;
    m.cuts.retain(|c| c.id != cut_id);
    session::write_manifest(session_dir, &m)?;
    session::append_cut(
        session_dir,
        &Cut {
            state: CutState::Undone,
            ..cut
        },
    )?;
    let _ = std::fs::remove_dir_all(&trash);
    session::read_manifest(session_dir)
}

// ---------------------------------------------------------------------------
// finalize
// ---------------------------------------------------------------------------

/// Finalize every `Pending` cut now, undo window or not.
pub fn finalize_cuts(session_dir: &Path) -> Result<SessionManifest, String> {
    let _g = edit_lock();
    finalize_where(session_dir, |_| true)
}

/// Finalize the `Pending` cuts whose undo window has passed. This is what the
/// scheduled task runs, so a timer from one delete never cuts short the undo
/// window of a later one.
pub fn finalize_due_cuts(session_dir: &Path) -> Result<SessionManifest, String> {
    let _g = edit_lock();
    let now = session::now_epoch_ms();
    finalize_where(session_dir, |c| now - c.created_epoch_ms >= UNDO_WINDOW_MS)
}

fn finalize_where(session_dir: &Path, due: impl Fn(&Cut) -> bool) -> Result<SessionManifest, String> {
    let pending: Vec<Cut> = session::read_cuts(session_dir)
        .into_iter()
        .filter(|c| c.state == CutState::Pending && due(c))
        .collect();
    if !pending.is_empty() {
        let ffmpeg = crate::capture::find_ffmpeg().ok_or("FFmpeg not found")?;
        let ffprobe = session::find_ffprobe().ok_or("ffprobe not found")?;
        for cut in &pending {
            finalize_one(session_dir, cut, &ffmpeg, &ffprobe)?;
        }
    }
    load_manifest(session_dir)
}

fn finalize_one(session_dir: &Path, cut: &Cut, ffmpeg: &str, ffprobe: &str) -> Result<(), String> {
    let trash = trash_dir(session_dir, &cut.id);

    // A crash after the journal was written: every kept piece already exists, so
    // roll forward.
    if let Some(j) = read_json::<Journal>(&trash.join(JOURNAL_FILE)) {
        return apply_journal(session_dir, cut, &j);
    }

    let pf: Option<PlanFile> = read_json(&trash.join(PLAN_FILE));
    let m = load_manifest(session_dir)?;

    // A crash in the middle of an undo can leave a parked file back home but
    // unreferenced. The cut stands, so it goes.
    if let Some(pf) = &pf {
        for tc in &pf.plan.tracks {
            for seg in &tc.remove {
                let home = session_dir.join(&tc.dir).join(&seg.file);
                let referenced = !tc.sidecar
                    && m.tracks
                        .iter()
                        .any(|t| t.dir == tc.dir && t.segments.iter().any(|s| s.file == seg.file));
                if home.is_file() && !referenced {
                    let parked_dir = trash.join(&tc.dir);
                    let _ = std::fs::create_dir_all(&parked_dir);
                    let parked = parked_dir.join(&seg.file);
                    if parked.exists() {
                        let _ = std::fs::remove_file(&home);
                    } else {
                        let _ = std::fs::rename(&home, &parked);
                    }
                }
            }
        }
    }

    // Re-plan against the manifest as it is now. Usually that is just the trims
    // planned at delete time, but an overlapping later cut may already have
    // reshaped a segment, and this way neither cut works from a stale record.
    let tracks = cut.tracks.as_deref();
    // A named track that has since vanished from the manifest simply has nothing left.
    let mut plan = plan_tracks(&m, cut.from_epoch_ms, cut.to_epoch_ms, tracks);
    if let Some(tc) = sidecar_plan(session_dir, &m, cut.from_epoch_ms, cut.to_epoch_ms, tracks) {
        plan.tracks.push(tc);
    }

    let mut journal = Journal::default();
    let mut freed = dir_size(&trash).saturating_sub(file_len(&trash.join(PLAN_FILE)));
    for tc in &plan.tracks {
        let track_dir = session_dir.join(&tc.dir);
        for seg in &tc.remove {
            freed += file_len(&track_dir.join(&seg.file));
            journal.delete.push((tc.dir.clone(), seg.file.clone()));
        }
        let trims = tc
            .trim_head
            .iter()
            .map(|s| (s, true, false))
            .chain(tc.trim_tail.iter().map(|s| (s, false, true)))
            .chain(tc.split.iter().map(|s| (s, true, true)));
        for (seg, head, tail) in trims {
            match cut_pieces(&track_dir, seg, cut, head, tail, ffmpeg, ffprobe, &journal) {
                Ok(pieces) => {
                    let kept: u64 = pieces.iter().map(|p| p.bytes).sum();
                    freed += file_len(&track_dir.join(&seg.file)).saturating_sub(kept);
                    journal.replace.push(Replacement {
                        dir: tc.dir.clone(),
                        sidecar: tc.sidecar,
                        original: seg.file.clone(),
                        pieces,
                    });
                }
                // The original stays, untouched and still masked by the cut.
                Err(e) => log_edit(
                    session_dir,
                    &format!("{}: could not trim {}/{}: {}", cut.id, tc.dir, seg.file, e),
                ),
            }
        }
    }
    journal.freed_bytes = freed;

    std::fs::create_dir_all(&trash).map_err(|e| format!("{}: {}", trash.display(), e))?;
    if let Err(e) = write_json_atomic(&trash.join(JOURNAL_FILE), &journal) {
        // Nothing references the new pieces yet; drop them and try again later.
        for r in &journal.replace {
            for p in &r.pieces {
                let _ = std::fs::remove_file(session_dir.join(&r.dir).join(&p.file));
            }
        }
        return Err(e);
    }
    apply_journal(session_dir, cut, &journal)
}

/// Commit a finalize: manifest first, then deletes, then the `Final` line, then
/// the trash dir. Idempotent, so recovery can replay it any number of times.
fn apply_journal(session_dir: &Path, cut: &Cut, j: &Journal) -> Result<(), String> {
    let mut m = load_manifest(session_dir)?;
    for r in j.replace.iter().filter(|r| !r.sidecar) {
        if let Some(t) = m.tracks.iter_mut().find(|t| t.dir == r.dir) {
            t.segments
                .retain(|s| s.file != r.original && !r.pieces.iter().any(|p| p.file == s.file));
            t.segments.extend(r.pieces.iter().cloned());
        }
    }
    for (dir, file) in &j.delete {
        if let Some(t) = m.tracks.iter_mut().find(|t| t.dir == *dir) {
            t.segments.retain(|s| s.file != *file);
        }
    }
    for t in m.tracks.iter_mut() {
        t.segments.sort_by_key(|s| s.start_epoch_ms);
    }
    let fin = Cut {
        state: CutState::Final,
        freed_bytes: j.freed_bytes,
        ..cut.clone()
    };
    match m.cuts.iter_mut().find(|c| c.id == cut.id) {
        Some(c) => *c = fin.clone(),
        None => m.cuts.push(fin.clone()),
    }
    session::write_manifest(session_dir, &m)?;

    for r in &j.replace {
        remove_if_present(session_dir, &session_dir.join(&r.dir).join(&r.original));
    }
    for (dir, file) in &j.delete {
        remove_if_present(session_dir, &session_dir.join(dir).join(file));
    }
    // Final before the trash goes: a crash in between leaves a Final cut with a
    // trash dir, which recovery just removes.
    session::append_cut(session_dir, &fin)?;
    let trash = trash_dir(session_dir, &cut.id);
    if trash.exists() {
        std::fs::remove_dir_all(&trash).map_err(|e| format!("{}: {}", trash.display(), e))?;
    }
    Ok(())
}

fn remove_if_present(session_dir: &Path, p: &Path) {
    if p.exists() {
        if let Err(e) = std::fs::remove_file(p) {
            log_edit(session_dir, &format!("could not delete {}: {}", p.display(), e));
        }
    }
}

/// A free filename for a kept piece starting at `epoch_ms`, named the way the
/// recorder names segments (`-strftime` local time, to the second) so a rebuild
/// reproduces the start. A collision gets `-1`, `-2`, …, which
/// `epoch_from_segment_name` reads past.
fn piece_name(track_dir: &Path, epoch_ms: i64, ext: &str, taken: &[String]) -> String {
    use chrono::TimeZone;
    let base = chrono::Local
        .timestamp_millis_opt(epoch_ms.div_euclid(1000) * 1000)
        .single()
        .map(|t| t.format(session::SEGMENT_STRFTIME).to_string())
        .unwrap_or_else(|| epoch_ms.to_string());
    let free = |n: &str| !taken.iter().any(|t| t == n) && !track_dir.join(n).exists();
    let first = format!("{}.{}", base, ext);
    if free(&first) {
        return first;
    }
    (1..)
        .map(|i| format!("{}-{}.{}", base, i, ext))
        .find(|n| free(n))
        .unwrap_or(first)
}

/// Keyframe times (seconds from the start of the file) of the first video
/// stream, or None when the file has no video — then every packet is a sync
/// point and the cut can land exactly.
fn keyframes(ffprobe: &str, path: &Path) -> Result<Option<Vec<f64>>, String> {
    let out = crate::recorder::silent_command(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,flags",
            "-show_entries",
            "format=start_time",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("ffprobe spawn failed: {}", e))?;
    if !out.status.success() {
        return Err(format!("ffprobe failed: {}", session::tail(&String::from_utf8_lossy(&out.stderr), 200)));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe json: {}", e))?;
    let start = v["format"]["start_time"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let Some(packets) = v["packets"].as_array().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let mut kfs: Vec<f64> = packets
        .iter()
        .filter(|p| p["flags"].as_str().map_or(false, |f| f.starts_with('K')))
        .filter_map(|p| p["pts_time"].as_str().and_then(|s| s.parse::<f64>().ok()))
        .map(|t| (t - start).max(0.0))
        .collect();
    kfs.sort_by(|a, b| a.total_cmp(b));
    Ok(Some(kfs))
}

fn run_ffmpeg(ffmpeg: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let out = crate::recorder::silent_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .map_err(|e| format!("ffmpeg spawn failed: {}", e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("ffmpeg failed: {}", session::tail(&String::from_utf8_lossy(&out.stderr), 300)))
    }
}

fn secs(ms: i64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// Write the kept piece(s) of one partly-covered segment next to it, by stream
/// copy. On any failure every piece made so far is removed and the original is
/// left exactly as it was.
#[allow(clippy::too_many_arguments)]
fn cut_pieces(
    track_dir: &Path,
    seg: &Segment,
    cut: &Cut,
    head: bool,
    tail: bool,
    ffmpeg: &str,
    ffprobe: &str,
    journal: &Journal,
) -> Result<Vec<Segment>, String> {
    let src = track_dir.join(&seg.file);
    if !src.is_file() {
        return Err("file is missing".to_string());
    }
    let ext = Path::new(&seg.file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let muxer = match ext.as_str() {
        "wav" => "wav",
        "mka" => "matroska",
        _ => "mp4",
    };
    // Names already promised to other pieces in this finalize.
    let mut taken: Vec<String> = journal
        .replace
        .iter()
        .flat_map(|r| r.pieces.iter().map(|p| p.file.clone()))
        .collect();

    let mut made: Vec<Segment> = Vec::new();
    let mut make = |start: i64, args_before: Vec<String>, args_after: Vec<String>| -> Result<(), String> {
        let name = piece_name(track_dir, start, &ext, &taken);
        let tmp = track_dir.join(format!("{}{}", name, TMP_SUFFIX));
        let mut args: Vec<std::ffi::OsString> = args_before.into_iter().map(Into::into).collect();
        args.push("-i".into());
        args.push(src.clone().into_os_string());
        args.extend(["-map", "0", "-c", "copy"].map(Into::into));
        args.extend(args_after.into_iter().map(Into::into));
        args.extend(["-f".into(), muxer.into()]);
        args.push(tmp.clone().into_os_string());
        let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
        let res = run_ffmpeg(ffmpeg, &refs)
            .and_then(|_| session::probe_segment(ffprobe, &tmp))
            .and_then(|p| {
                std::fs::rename(&tmp, track_dir.join(&name))
                    .map(|_| p)
                    .map_err(|e| format!("rename {}: {}", name, e))
            });
        match res {
            Ok(p) => {
                taken.push(name.clone());
                made.push(Segment {
                    bytes: file_len(&track_dir.join(&name)),
                    file: name,
                    start_epoch_ms: start,
                    duration_ms: p.duration_ms,
                });
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    };

    let result = (|| -> Result<(), String> {
        if head {
            let len = cut.from_epoch_ms - seg.start_epoch_ms;
            make(seg.start_epoch_ms, vec![], vec!["-t".into(), secs(len)])?;
        }
        if tail {
            let off_ms = cut.to_epoch_ms - seg.start_epoch_ms;
            // Video can only start on a keyframe: the last one at or before the
            // cut's end. The footage between it and the end survives on disk,
            // masked by the cut.
            let kf_ms = match keyframes(ffprobe, &src)? {
                Some(kfs) => kfs
                    .iter()
                    .rev()
                    .map(|k| (k * 1000.0).round() as i64)
                    .find(|k| *k <= off_ms)
                    .unwrap_or(0),
                None => off_ms,
            };
            // Half a millisecond past the keyframe so float rounding cannot send
            // the input seek back to the previous one.
            make(
                seg.start_epoch_ms + kf_ms,
                vec!["-ss".into(), format!("{:.4}", kf_ms as f64 / 1000.0 + 0.0005)],
                vec!["-avoid_negative_ts".into(), "make_zero".into()],
            )?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => Ok(made),
        Err(e) => {
            for p in &made {
                let _ = std::fs::remove_file(track_dir.join(&p.file));
            }
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Whole sessions
// ---------------------------------------------------------------------------

/// `list_sessions` without the `.trash` parking area.
pub fn visible_sessions(root: &Path) -> Vec<crate::recorder::SessionSummary> {
    crate::recorder::list_sessions(root)
        .into_iter()
        .filter(|s| !s.id.starts_with('.'))
        .collect()
}

/// Move `<root>/<id>` into `<root>/.trash/<id>`. The caller has refused the live
/// session, and schedules `purge_session` `FINALIZE_AFTER_MS` later with the
/// receipt's `undo_until_epoch_ms` as the token.
pub fn delete_session(root: &Path, id: &str) -> Result<SessionTrashReceipt, String> {
    check_id(id)?;
    let _g = edit_lock();
    let src = root.join(id);
    if !src.is_dir() {
        return Err(format!("No session named {}", id));
    }
    let bytes = dir_size(&src);
    let trash_root = root.join(TRASH_DIR);
    std::fs::create_dir_all(&trash_root).map_err(|e| format!("{}: {}", trash_root.display(), e))?;
    let dst = trash_root.join(id);
    if dst.exists() {
        // A stale copy whose purge never ran.
        std::fs::remove_dir_all(&dst).map_err(|e| format!("{}: {}", dst.display(), e))?;
    }
    std::fs::rename(&src, &dst).map_err(|e| format!("Could not move {}: {}", src.display(), e))?;
    let token = session::now_epoch_ms() + UNDO_WINDOW_MS;
    TRASHED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(dst, token);
    Ok(SessionTrashReceipt {
        id: id.to_string(),
        bytes,
        undo_until_epoch_ms: token,
    })
}

pub fn undo_delete_session(root: &Path, id: &str) -> Result<(), String> {
    check_id(id)?;
    let _g = edit_lock();
    let parked = root.join(TRASH_DIR).join(id);
    if !parked.is_dir() {
        return Err(UNDO_EXPIRED_ERR.to_string());
    }
    let home = root.join(id);
    if home.exists() {
        return Err(format!("A session named {} already exists", id));
    }
    std::fs::rename(&parked, &home).map_err(|e| format!("Could not restore {}: {}", home.display(), e))?;
    if let Some(map) = TRASHED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        map.remove(&parked);
    }
    Ok(())
}

/// Permanently remove a trashed session, if it is still the one `token`'s delete
/// parked. Returns whether anything was removed.
pub fn purge_session(root: &Path, id: &str, token: i64) -> Result<bool, String> {
    check_id(id)?;
    let _g = edit_lock();
    let parked = root.join(TRASH_DIR).join(id);
    {
        let mut guard = TRASHED.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if map.get(&parked) != Some(&token) {
            return Ok(false);
        }
        map.remove(&parked);
    }
    if !parked.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&parked).map_err(|e| format!("{}: {}", parked.display(), e))?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Startup recovery
// ---------------------------------------------------------------------------

/// Finish whatever a previous run left half done: purge trashed sessions, and in
/// every session finish or roll back interrupted cuts and finalize `Pending`
/// ones whose undo window has passed. Undo is only promised while the app stays
/// open. Run on a background thread at start, before anything records.
pub fn recover_sessions(root: &Path) {
    let _g = edit_lock();
    if let Ok(rd) = std::fs::read_dir(root.join(TRASH_DIR)) {
        for e in rd.flatten() {
            if let Err(err) = std::fs::remove_dir_all(e.path()) {
                eprintln!("[chirptime] edit: purge {}: {}", e.path().display(), err);
            }
        }
    }
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = e.path();
        if let Err(err) = recover_session(&dir) {
            log_edit(&dir, &format!("recovery failed: {}", err));
        }
    }
}

fn recover_session(session_dir: &Path) -> Result<(), String> {
    let trash_root = session_dir.join(TRASH_DIR);
    if !trash_root.is_dir() && !session_dir.join(session::CUTS_FILE).is_file() {
        return Ok(());
    }
    let cuts = session::read_cuts(session_dir);

    if let Ok(rd) = std::fs::read_dir(&trash_root) {
        for e in rd.flatten() {
            let cut_id = e.file_name().to_string_lossy().to_string();
            let trash = e.path();
            match cuts.iter().find(|c| c.id == cut_id) {
                // The delete never committed its cuts.jsonl line: roll it back.
                None => {
                    if let Some(pf) = read_json::<PlanFile>(&trash.join(PLAN_FILE)) {
                        let mut m = load_manifest(session_dir)?;
                        restore_parked(session_dir, &trash, &pf, &mut m)?;
                        m.cuts.retain(|c| c.id != cut_id);
                        session::write_manifest(session_dir, &m)?;
                    }
                    let _ = std::fs::remove_dir_all(&trash);
                    log_edit(session_dir, &format!("{}: rolled back an uncommitted delete", cut_id));
                }
                Some(c) if c.state == CutState::Final => {
                    if let Some(j) = read_json::<Journal>(&trash.join(JOURNAL_FILE)) {
                        apply_journal(session_dir, c, &j)?;
                    }
                    let _ = std::fs::remove_dir_all(&trash);
                }
                // Undo moves everything back before it appends Undone.
                Some(c) if c.state == CutState::Undone => {
                    let _ = std::fs::remove_dir_all(&trash);
                }
                Some(_) => {} // Pending: finalized below.
            }
        }
    }

    let now = session::now_epoch_ms();
    finalize_where(session_dir, |c| now - c.created_epoch_ms >= UNDO_WINDOW_MS).map(|_| ())
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Rect, Track, TrackStatus};

    fn seg(file: &str, start: i64, dur: i64) -> Segment {
        Segment {
            file: file.to_string(),
            start_epoch_ms: start,
            duration_ms: dur,
            bytes: dur as u64 * 10,
        }
    }

    fn track(id: &str, segments: Vec<Segment>) -> Track {
        Track {
            id: id.to_string(),
            kind: if id == "mic" { "mic" } else { "screen" }.to_string(),
            label: id.to_string(),
            monitor: None,
            codec: "h264".to_string(),
            width: 0,
            height: 0,
            fps: 0,
            start_epoch_ms: segments.first().map(|s| s.start_epoch_ms).unwrap_or(0),
            dir: id.to_string(),
            segments,
            gaps: Vec::new(),
            status: TrackStatus::Stopped,
            error: None,
            restarts: 0,
        }
    }

    fn manifest(tracks: Vec<Track>) -> SessionManifest {
        let mut m = SessionManifest::new("s".to_string(), 0, 60, Rect::default());
        m.ended_epoch_ms = tracks
            .iter()
            .filter_map(|t| t.end_epoch_ms())
            .max();
        m.tracks = tracks;
        m
    }

    /// 60s video segments, back to back, from t=0.
    fn uniform(id: &str, n: i64, len: i64) -> Track {
        track(id, (0..n).map(|i| seg(&format!("{}-{}.mp4", id, i), i * len, len)).collect())
    }

    fn files(v: &[Segment]) -> Vec<&str> {
        v.iter().map(|s| s.file.as_str()).collect()
    }

    #[test]
    fn cut_inside_one_segment_splits_it() {
        let m = manifest(vec![uniform("mic", 2, 600_000)]);
        let p = plan_cut(&m, 100_000, 200_000, None).unwrap();
        assert_eq!(p.tracks.len(), 1);
        let t = &p.tracks[0];
        assert_eq!(files(&t.split), vec!["mic-0.mp4"]);
        assert!(t.remove.is_empty() && t.trim_head.is_empty() && t.trim_tail.is_empty());
    }

    #[test]
    fn cut_spanning_many_segments() {
        let m = manifest(vec![uniform("screen-0", 10, 60_000)]);
        // 90s → 330s: trims 1, removes 2,3,4, trims 5.
        let p = plan_cut(&m, 90_000, 330_000, None).unwrap();
        let t = &p.tracks[0];
        assert_eq!(files(&t.trim_head), vec!["screen-0-1.mp4"]);
        assert_eq!(files(&t.remove), vec!["screen-0-2.mp4", "screen-0-3.mp4", "screen-0-4.mp4"]);
        assert_eq!(files(&t.trim_tail), vec!["screen-0-5.mp4"]);
        assert!(t.split.is_empty());
    }

    #[test]
    fn ragged_and_overlapping_segments_use_their_own_spans() {
        // 4s / 14s / 16s, the second overlapping the first by 1.5s, then a hole.
        let t = track(
            "screen-0",
            vec![
                seg("a.mp4", 0, 4_000),
                seg("b.mp4", 2_500, 14_000),  // 2.5 → 16.5
                seg("c.mp4", 16_520, 16_000), // 16.52 → 32.52
                seg("d.mp4", 40_000, 15_000), // after a hole
            ],
        );
        let m = manifest(vec![t]);
        let p = plan_cut(&m, 3_000, 20_000, None).unwrap();
        let tc = &p.tracks[0];
        assert_eq!(files(&tc.trim_head), vec!["a.mp4"]); // keeps 0 → 3s
        assert_eq!(files(&tc.remove), vec!["b.mp4"]); // 2.5 → 16.5: head 0.5s < 1s
        assert_eq!(files(&tc.trim_tail), vec!["c.mp4"]);
        // A cut entirely inside the hole touches nothing but is still a valid cut.
        let p = plan_cut(&m, 33_000, 39_000, None).unwrap();
        assert!(p.tracks.is_empty());
    }

    #[test]
    fn cut_touching_session_start_and_end() {
        let m = manifest(vec![uniform("screen-0", 3, 60_000)]);
        let p = plan_cut(&m, -5_000, 30_000, None).unwrap();
        assert_eq!(files(&p.tracks[0].trim_tail), vec!["screen-0-0.mp4"]);
        let p = plan_cut(&m, 150_000, 999_000, None).unwrap();
        assert_eq!(files(&p.tracks[0].trim_head), vec!["screen-0-2.mp4"]);
        let p = plan_cut(&m, 0, 180_000, None).unwrap();
        assert_eq!(p.tracks[0].remove.len(), 3);
    }

    #[test]
    fn per_track_cut_leaves_other_tracks_alone() {
        let m = manifest(vec![uniform("screen-0", 3, 60_000), uniform("mic", 1, 600_000)]);
        let only_mic = vec!["mic".to_string()];
        let p = plan_cut(&m, 70_000, 100_000, Some(&only_mic)).unwrap();
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.tracks[0].track_id, "mic");
        assert!(covers_mic(&m, Some(&only_mic)));
        assert!(!covers_mic(&m, Some(&["screen-0".to_string()])));
        assert!(covers_mic(&m, None));
        let err = plan_cut(&m, 70_000, 100_000, Some(&["webcam".to_string()])).unwrap_err();
        assert!(err.contains("webcam"), "{}", err);
        assert!(plan_cut(&m, 70_000, 100_000, Some(&[])).is_err());
    }

    #[test]
    fn kept_piece_under_a_second_becomes_a_remove() {
        let m = manifest(vec![uniform("screen-0", 3, 60_000)]);
        // keeps 0.5s of segment 0 and 0.4s of segment 2
        let p = plan_cut(&m, 500, 179_600, None).unwrap();
        assert_eq!(p.tracks[0].remove.len(), 3);
        // a split whose tail is under a second is a head trim
        let p = plan_cut(&m, 10_000, 59_500, None).unwrap();
        assert_eq!(files(&p.tracks[0].trim_head), vec!["screen-0-0.mp4"]);
        assert!(p.tracks[0].split.is_empty());
    }

    #[test]
    fn range_outside_the_session_is_an_error() {
        let m = manifest(vec![uniform("screen-0", 3, 60_000)]);
        assert!(plan_cut(&m, 200_000, 300_000, None).unwrap_err().contains("outside"));
        assert!(plan_cut(&m, -50_000, -1_000, None).unwrap_err().contains("outside"));
        assert!(plan_cut(&m, 5_000, 5_000, None).is_err());
        assert!(plan_cut(&m, 6_000, 5_000, None).is_err());
    }

    #[test]
    fn estimate_counts_the_covered_fraction() {
        let s = seg("x.mp4", 0, 10_000); // 100_000 bytes
        assert_eq!(bytes_inside(&s, 2_500, 5_000), 25_000);
        assert_eq!(bytes_inside(&s, -5_000, 20_000), 100_000);
    }

    #[test]
    fn ids_that_escape_the_root_are_refused() {
        assert!(check_id("2026-07-27_11-56-40").is_ok());
        for bad in ["", ".trash", "../x", "a/b", "a\\b", "C:x"] {
            assert!(check_id(bad).is_err(), "{}", bad);
        }
    }
}
