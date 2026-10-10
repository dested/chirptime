//! End-to-end tests for deleting time (`edit.rs`) on synthetic sessions.
//!
//!     cargo test --test edit_cuts
//!
//! Every session here is built in a temp dir from footage ffmpeg generates with
//! `lavfi` (`testsrc2` at `-g 60` for video, `sine` for AAC and PCM WAV), under
//! real `-strftime` filenames. Nothing here reads or writes the real recordings
//! folder, opens a capture device, or records.

use chirptime_lib::edit;
use chirptime_lib::session::{self, CutState, Marker, Rect, Segment, SessionManifest, Track, TrackStatus};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const ID: &str = "2026-01-02_10-00-00";
const VIDEO_SEGS: i64 = 6;
const VIDEO_LEN_MS: i64 = 10_000;
const AUDIO_LEN_MS: i64 = 60_000;

fn ffprobe() -> String {
    session::find_ffprobe().expect("ffprobe is needed for these tests")
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg is needed for these tests");
    assert!(out.status.success(), "ffmpeg {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
}

/// Footage generated once and copied into every test's session.
fn templates() -> &'static PathBuf {
    static T: OnceLock<PathBuf> = OnceLock::new();
    T.get_or_init(|| {
        let dir = std::env::temp_dir()
            .join("chirptime-edit-cuts")
            .join(format!("templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = |n: &str| dir.join(n).to_string_lossy().to_string();
        ffmpeg(&[
            "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30", "-t", "10",
            "-c:v", "libx264", "-g", "60", "-pix_fmt", "yuv420p", "-f", "mp4", &p("video.mp4"),
        ]);
        ffmpeg(&[
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "60",
            "-ac", "1", "-c:a", "aac", "-f", "mp4", &p("mic.mp4"),
        ]);
        ffmpeg(&[
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=16000", "-t", "60",
            "-ac", "1", "-c:a", "pcm_s16le", "-f", "wav", &p("mic16k.wav"),
        ]);
        // A damaged segment: the manifest will claim 10s, the file holds 6s.
        ffmpeg(&[
            "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30", "-t", "6",
            "-c:v", "libx264", "-g", "60", "-pix_fmt", "yuv420p", "-f", "mp4", &p("short.mp4"),
        ]);
        dir
    })
}

fn base_epoch() -> i64 {
    session::epoch_from_session_id(ID).unwrap()
}

fn name_at(epoch_ms: i64, ext: &str) -> String {
    use chrono::TimeZone;
    let t = chrono::Local.timestamp_millis_opt(epoch_ms).single().unwrap();
    format!("{}.{}", t.format(session::SEGMENT_STRFTIME), ext)
}

fn probed(dir: &Path, file: &str) -> Segment {
    let path = dir.join(file);
    let p = session::probe_segment(&ffprobe(), &path).unwrap();
    Segment {
        file: file.to_string(),
        start_epoch_ms: session::epoch_from_segment_name(file).unwrap(),
        duration_ms: p.duration_ms,
        bytes: std::fs::metadata(&path).unwrap().len(),
    }
}

fn track(id: &str, kind: &str, segments: Vec<Segment>) -> Track {
    Track {
        id: id.to_string(),
        kind: kind.to_string(),
        label: id.to_string(),
        monitor: None,
        codec: String::new(),
        width: 0,
        height: 0,
        fps: 0,
        start_epoch_ms: segments[0].start_epoch_ms,
        dir: id.to_string(),
        segments,
        gaps: Vec::new(),
        status: TrackStatus::Stopped,
        error: None,
        restarts: 0,
    }
}

/// `<root>/<ID>` with six 10s `screen-0` segments, one 60s `mic` segment, a 60s
/// `mic-16k` WAV and two markers (t+5s and t+30s). Returns (root, session dir).
fn build_session(test: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir()
        .join("chirptime-edit-cuts")
        .join(format!("{}-{}", test, std::process::id()))
        .join("sessions");
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join(ID);
    let t = templates();
    let base = base_epoch();

    let screen = dir.join("screen-0");
    std::fs::create_dir_all(&screen).unwrap();
    let mut segs = Vec::new();
    for i in 0..VIDEO_SEGS {
        let name = name_at(base + i * VIDEO_LEN_MS, "mp4");
        std::fs::copy(t.join("video.mp4"), screen.join(&name)).unwrap();
        segs.push(probed(&screen, &name));
    }
    let mic = dir.join("mic");
    std::fs::create_dir_all(&mic).unwrap();
    let mic_name = name_at(base, "mp4");
    std::fs::copy(t.join("mic.mp4"), mic.join(&mic_name)).unwrap();
    let side = dir.join("mic-16k");
    std::fs::create_dir_all(&side).unwrap();
    std::fs::copy(t.join("mic16k.wav"), side.join(name_at(base, "wav"))).unwrap();

    let mut m = SessionManifest::new(ID.to_string(), base, 10, Rect::default());
    m.tracks.push(track("screen-0", "screen", segs));
    m.tracks.push(track("mic", "mic", vec![probed(&mic, &mic_name)]));
    m.markers = vec![
        Marker { epoch_ms: base + 5_000, label: "keep".into() },
        Marker { epoch_ms: base + 30_000, label: "inside".into() },
    ];
    m.ended_epoch_ms = Some(base + AUDIO_LEN_MS);
    session::write_manifest(&dir, &m).unwrap();
    (root, dir)
}

fn hashes(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for sub in ["screen-0", "mic", "mic-16k"] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
        for e in rd.flatten() {
            let bytes = std::fs::read(e.path()).unwrap();
            out.insert(
                format!("{}/{}", sub, e.file_name().to_string_lossy()),
                format!("{:x}", Sha256::digest(&bytes)),
            );
        }
    }
    out
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn seg_files(m: &SessionManifest, track: &str) -> Vec<String> {
    m.tracks.iter().find(|t| t.id == track).unwrap().segments.iter().map(|s| s.file.clone()).collect()
}

/// The cut every end-to-end test uses: t+15s → t+43s on every track. On
/// `screen-0` that trims segment 1's head, removes 2 and 3, and trims 4's tail
/// (whose keyframe snap is exercised: t+43s is not on a keyframe);
/// on `mic` and `mic-16k` it splits the one 60s segment.
fn cut_range() -> (i64, i64) {
    (base_epoch() + 15_000, base_epoch() + 43_000)
}

#[test]
fn delete_parks_files_and_undo_restores_them_byte_for_byte() {
    let (_root, dir) = build_session("undo");
    let before = hashes(&dir);
    let before_manifest = session::read_manifest(&dir).unwrap();
    let (from, to) = cut_range();

    let r = edit::delete_range(&dir, from, to, None).unwrap();
    assert_eq!(r.cut.state, CutState::Pending);
    assert!(r.undo_until_epoch_ms - r.cut.created_epoch_ms == edit::UNDO_WINDOW_MS);
    assert!(r.freed_bytes_estimate > 0);

    // Files wholly inside the cut sit in .trash; the rest stay put.
    let trash = dir.join(".trash").join(&r.cut.id);
    let parked = files_in(&trash.join("screen-0"));
    assert_eq!(
        parked,
        vec![name_at(base_epoch() + 20_000, "mp4"), name_at(base_epoch() + 30_000, "mp4")]
    );
    assert!(trash.join("plan.json").is_file());
    assert_eq!(files_in(&dir.join("screen-0")).len(), 4);

    let m = session::read_manifest(&dir).unwrap();
    assert_eq!(seg_files(&m, "screen-0").len(), 4);
    assert!(!seg_files(&m, "screen-0").iter().any(|f| parked.contains(f)));
    assert_eq!(m.markers.len(), 1, "the marker inside the range is dropped");
    assert_eq!(m.cuts.len(), 1);
    assert_eq!(m.cuts[0].state, CutState::Pending);
    let jsonl = session::read_cuts(&dir);
    assert_eq!(jsonl.len(), 1);
    assert_eq!(jsonl[0].state, CutState::Pending);

    let m = edit::undo_cut(&dir, &r.cut.id).unwrap();
    assert_eq!(hashes(&dir), before, "undo must restore every file byte for byte");
    assert_eq!(seg_files(&m, "screen-0"), seg_files(&before_manifest, "screen-0"));
    assert_eq!(m.markers, before_manifest.markers);
    assert!(m.cuts.is_empty(), "an undone cut is not carried");
    assert_eq!(session::read_cuts(&dir)[0].state, CutState::Undone);
    assert!(!trash.exists());

    assert_eq!(edit::undo_cut(&dir, &r.cut.id).unwrap_err(), edit::UNDO_EXPIRED_ERR);
}

/// After finalize: every manifest file exists and probes, its duration matches
/// ffprobe, and nothing in a track dir is unreferenced.
fn assert_consistent(dir: &Path, m: &SessionManifest) {
    let fp = ffprobe();
    for t in &m.tracks {
        let on_disk = files_in(&dir.join(&t.dir));
        let mut referenced: Vec<String> = t.segments.iter().map(|s| s.file.clone()).collect();
        referenced.sort();
        assert_eq!(on_disk, referenced, "{}: track dir must hold exactly the manifest's files", t.id);
        for s in &t.segments {
            let p = session::probe_segment(&fp, &dir.join(&t.dir).join(&s.file))
                .unwrap_or_else(|e| panic!("{}/{} must be playable: {}", t.id, s.file, e));
            assert!((p.duration_ms - s.duration_ms).abs() <= 50, "{}/{}", t.id, s.file);
        }
        let starts: Vec<i64> = t.segments.iter().map(|s| s.start_epoch_ms).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(starts, sorted, "{}: segments sorted by start", t.id);
    }
}

#[test]
fn finalize_trims_by_stream_copy_and_rebuild_agrees() {
    let (_root, dir) = build_session("finalize");
    let base = base_epoch();
    let (from, to) = cut_range();
    let before = session::read_manifest(&dir).unwrap();
    let bytes_before: u64 = before.total_bytes();

    let r = edit::delete_range(&dir, from, to, None).unwrap();
    let m = edit::finalize_cuts(&dir).unwrap();

    assert_eq!(m.cuts.len(), 1);
    assert_eq!(m.cuts[0].state, CutState::Final);
    assert!(m.cuts[0].freed_bytes > 0);
    assert!(!dir.join(".trash").join(&r.cut.id).exists());
    assert_consistent(&dir, &m);

    // screen-0: [0,10) [10,15) | cut | [40+kf, 50) [50,60)
    let screen = &m.tracks.iter().find(|t| t.id == "screen-0").unwrap().segments;
    assert_eq!(screen.len(), 4);
    assert_eq!(screen[1].start_epoch_ms, base + 10_000, "a head piece keeps its start");
    // Stream copy stops on the first packet at or past the cut, so a head piece
    // can run a frame or two into it (masked by the cut), never short of it.
    assert!(
        (5_000..=5_100).contains(&screen[1].duration_ms),
        "head ends at the cut: {}",
        screen[1].duration_ms
    );
    // The tail piece snaps back to the last keyframe at or before t+43s (GOP 2s,
    // so t+42s) and still ends where the original did.
    let tail = &screen[2];
    assert!((tail.start_epoch_ms - (base + 42_000)).abs() <= 50, "{}", tail.start_epoch_ms - base);
    assert!((tail.start_epoch_ms + tail.duration_ms - (base + 50_000)).abs() <= 50);
    assert_eq!(tail.file, name_at(tail.start_epoch_ms, "mp4"));

    // mic: split into [0,15) and [43,60), exactly: every audio packet is a sync point
    let mic = &m.tracks.iter().find(|t| t.id == "mic").unwrap().segments;
    assert_eq!(mic.len(), 2);
    assert_eq!(mic[0].start_epoch_ms, base);
    assert!((mic[0].duration_ms - 15_000).abs() <= 50, "{}", mic[0].duration_ms);
    assert_eq!(mic[1].start_epoch_ms, to);
    assert!((mic[1].start_epoch_ms + mic[1].duration_ms - (base + AUDIO_LEN_MS)).abs() <= 50);

    // mic-16k follows the mic.
    let wavs = files_in(&dir.join("mic-16k"));
    assert_eq!(wavs.len(), 2, "{:?}", wavs);
    let fp = ffprobe();
    let total: i64 = wavs
        .iter()
        .map(|w| session::probe_segment(&fp, &dir.join("mic-16k").join(w)).unwrap().duration_ms)
        .sum();
    assert!((total - (AUDIO_LEN_MS - (to - from))).abs() <= 100, "{}", total);
    assert!(wavs.contains(&name_at(to, "wav")));

    assert!(m.total_bytes() < bytes_before);

    // A rebuild from disk alone lands on the same starts and still carries the cut.
    let rebuilt = session::rebuild_manifest_from_disk(&dir).unwrap();
    assert_eq!(rebuilt.cuts.len(), 1);
    assert_eq!(rebuilt.cuts[0].state, CutState::Final);
    assert_eq!(rebuilt.markers.len(), 1, "the marker inside the cut stays gone");
    for t in &m.tracks {
        let rt = rebuilt.tracks.iter().find(|x| x.id == t.id).unwrap();
        assert_eq!(rt.segments.len(), t.segments.len(), "{}", t.id);
        for (a, b) in t.segments.iter().zip(&rt.segments) {
            assert!((a.start_epoch_ms - b.start_epoch_ms).abs() <= 1_000, "{}/{}", t.id, a.file);
        }
    }
    assert!(!rebuilt.tracks.iter().any(|t| t.id.starts_with('.') || t.id == "logs"));
}

#[test]
fn a_mic_only_cut_leaves_the_screen_alone() {
    let (_root, dir) = build_session("mic-only");
    let (from, to) = cut_range();
    let before = session::read_manifest(&dir).unwrap();
    edit::delete_range(&dir, from, to, Some(vec!["mic".to_string()])).unwrap();
    let m = edit::finalize_cuts(&dir).unwrap();
    assert_eq!(seg_files(&m, "screen-0"), seg_files(&before, "screen-0"));
    assert_eq!(seg_files(&m, "mic").len(), 2);
    assert_eq!(files_in(&dir.join("mic-16k")).len(), 2);
    assert_eq!(m.cuts[0].tracks.as_deref(), Some(&["mic".to_string()][..]));
    assert_eq!(m.markers, before.markers, "a per-track cut keeps every marker");
    assert_eq!(session::rebuild_manifest_from_disk(&dir).unwrap().markers, before.markers);
    assert_consistent(&dir, &m);

    assert!(edit::delete_range(&dir, from, to, Some(vec!["webcam".to_string()])).is_err());
}

#[test]
fn a_crash_before_finalize_is_finished_by_startup_recovery() {
    let (root, dir) = build_session("crash");
    let (from, to) = cut_range();
    let r = edit::delete_range(&dir, from, to, None).unwrap();

    // The app dies here: no finalize ever runs. Recovery only takes cuts whose
    // undo window has passed, so first it must leave this one alone, and hand the
    // session back for a finalize once the window closes.
    assert_eq!(edit::recover_sessions(&root), vec![dir.clone()]);
    assert_eq!(session::read_cuts(&dir)[0].state, CutState::Pending);

    std::thread::sleep(std::time::Duration::from_millis(edit::UNDO_WINDOW_MS as u64 + 300));
    assert!(edit::recover_sessions(&root).is_empty());

    let m = session::read_manifest(&dir).unwrap();
    assert_eq!(m.cuts.len(), 1);
    assert_eq!(m.cuts[0].state, CutState::Final);
    assert!(!dir.join(".trash").join(&r.cut.id).exists());
    assert_consistent(&dir, &m);
    assert_eq!(files_in(&dir.join("mic-16k")).len(), 2);
}

#[test]
fn whole_session_delete_undo_and_purge() {
    let (root, _dir) = build_session("whole");
    let listed = |root: &Path| edit::visible_sessions(root).iter().any(|s| s.id == ID);
    assert!(listed(&root));

    let r = edit::delete_session(&root, ID).unwrap();
    assert!(r.bytes > 0);
    assert!(!listed(&root));
    assert!(edit::visible_sessions(&root).iter().all(|s| !s.id.starts_with('.')));

    edit::undo_delete_session(&root, ID).unwrap();
    assert!(listed(&root));
    // The purge scheduled by the undone delete must not fire on anything.
    assert!(!edit::purge_session(&root, ID, r.undo_until_epoch_ms).unwrap());
    assert!(listed(&root));

    let r = edit::delete_session(&root, ID).unwrap();
    assert!(edit::purge_session(&root, ID, r.undo_until_epoch_ms).unwrap());
    assert!(!listed(&root));
    assert!(!root.join(".trash").join(ID).exists());
    assert_eq!(edit::undo_delete_session(&root, ID).unwrap_err(), edit::UNDO_EXPIRED_ERR);
}

fn tmp_files(dir: &Path) -> Vec<String> {
    ["screen-0", "mic", "mic-16k"]
        .iter()
        .flat_map(|d| files_in(&dir.join(d)).into_iter().map(move |f| format!("{}/{}", d, f)))
        .filter(|f| f.ends_with(".cut.tmp"))
        .collect()
}

#[test]
fn a_crash_mid_finalize_is_replayed_by_recovery() {
    let (root, dir) = build_session("crash-mid");
    let (from, to) = cut_range();
    let before = session::read_manifest(&dir).unwrap();
    let r = edit::delete_range(&dir, from, to, None).unwrap();
    let trash = dir.join(".trash").join(&r.cut.id);
    std::fs::write(trash.join("crash-after-journal"), b"").unwrap();

    // The pieces exist and the journal is written; then the app "dies".
    assert!(edit::finalize_cuts(&dir).unwrap_err().contains("simulated crash"));
    assert!(trash.join("finalize.json").is_file());
    let tmps = tmp_files(&dir);
    assert!(tmps.len() >= 4, "pieces stay *.cut.tmp until the commit: {:?}", tmps);
    let m = session::read_manifest(&dir).unwrap();
    assert_eq!(m.cuts[0].state, CutState::Pending);
    let seg1 = &seg_files(&before, "screen-0")[1];
    assert!(seg_files(&m, "screen-0").contains(seg1), "originals still referenced");
    assert!(dir.join("screen-0").join(seg1).is_file());
    // Undo is off the table once the finalize has committed its pieces.
    assert_eq!(edit::undo_cut(&dir, &r.cut.id).unwrap_err(), edit::FINALIZING_ERR);

    std::fs::remove_file(trash.join("crash-after-journal")).unwrap();
    // The cut is younger than the undo window, but a journaled finalize rolls forward.
    assert!(edit::recover_sessions(&root).is_empty());

    let m = session::read_manifest(&dir).unwrap();
    assert_eq!(m.cuts[0].state, CutState::Final);
    assert!(!trash.exists());
    assert!(tmp_files(&dir).is_empty());
    assert_consistent(&dir, &m);
    assert_eq!(files_in(&dir.join("mic-16k")).len(), 2);
}

#[test]
fn recovery_of_a_final_cut_never_touches_the_manifest() {
    let (root, dir) = build_session("crash-final");
    let (from, to) = cut_range();
    let r = edit::delete_range(&dir, from, to, None).unwrap();
    let trash = dir.join(".trash").join(&r.cut.id);
    std::fs::write(trash.join("crash-before-trash-removal"), b"").unwrap();

    assert!(edit::finalize_cuts(&dir).unwrap_err().contains("simulated crash"));
    assert_eq!(session::read_cuts(&dir)[0].state, CutState::Final);
    assert!(trash.join("finalize.json").is_file());
    let manifest_bytes = std::fs::read(dir.join("session.json")).unwrap();

    edit::recover_sessions(&root);
    assert_eq!(std::fs::read(dir.join("session.json")).unwrap(), manifest_bytes);
    assert!(!trash.exists());
    assert_consistent(&dir, &session::read_manifest(&dir).unwrap());
}

#[test]
fn overlapping_cuts_finalize_into_their_union() {
    let (_root, dir) = build_session("overlap");
    let base = base_epoch();
    // A: t+15 → t+43, then B: t+38 → t+52 while A is still pending.
    edit::delete_range(&dir, base + 15_000, base + 43_000, None).unwrap();
    edit::delete_range(&dir, base + 38_000, base + 52_000, None).unwrap();
    let m = edit::finalize_cuts(&dir).unwrap();

    assert_eq!(m.cuts.len(), 2);
    assert!(m.cuts.iter().all(|c| c.state == CutState::Final));
    assert_consistent(&dir, &m);
    assert!(tmp_files(&dir).is_empty());

    let spans = |track: &str| -> Vec<(i64, i64)> {
        m.tracks
            .iter()
            .find(|t| t.id == track)
            .unwrap()
            .segments
            .iter()
            .map(|s| (s.start_epoch_ms - base, s.start_epoch_ms + s.duration_ms - base))
            .collect()
    };
    // screen-0: [0,10) [10,15) | union cut | [52 snapped back to its keyframe at 52, 60)
    let screen = spans("screen-0");
    assert_eq!(screen.len(), 3, "{:?}", screen);
    assert_eq!(screen[0], (0, 10_000));
    assert!(screen[1].0 == 10_000 && (5_000..=5_100).contains(&(screen[1].1 - 10_000)), "{:?}", screen[1]);
    assert!((screen[2].0 - 52_000).abs() <= 50 && (screen[2].1 - 60_000).abs() <= 50, "{:?}", screen[2]);
    // mic: [0,15) and [52,60); nothing outside the union lost.
    let mic = spans("mic");
    assert_eq!(mic.len(), 2, "{:?}", mic);
    assert!(mic[0].0 == 0 && (mic[0].1 - 15_000).abs() <= 50, "{:?}", mic[0]);
    assert!(mic[1].0 == 52_000 && (mic[1].1 - 60_000).abs() <= 50, "{:?}", mic[1]);
    let fp = ffprobe();
    let wav_total: i64 = files_in(&dir.join("mic-16k"))
        .iter()
        .map(|w| session::probe_segment(&fp, &dir.join("mic-16k").join(w)).unwrap().duration_ms)
        .sum();
    assert!((wav_total - 23_000).abs() <= 100, "{}", wav_total);
}

#[test]
fn a_failed_undo_rolls_back_and_the_timer_leaves_it_alone() {
    let (_root, dir) = build_session("undo-fail");
    let before = hashes(&dir);
    let (from, to) = cut_range();
    let r = edit::delete_range(&dir, from, to, None).unwrap();
    let after_delete = hashes(&dir);
    let manifest_after_delete = std::fs::read(dir.join("session.json")).unwrap();
    let trash = dir.join(".trash").join(&r.cut.id);

    // Something sits where the second parked segment has to go back.
    let blocker = dir.join("screen-0").join(name_at(base_epoch() + 30_000, "mp4"));
    std::fs::write(&blocker, b"in the way").unwrap();
    let err = edit::undo_cut(&dir, &r.cut.id).unwrap_err();
    assert!(err.starts_with("Could not undo that delete"), "{}", err);

    // All or nothing: the first segment went back into the trash.
    std::fs::remove_file(&blocker).unwrap();
    assert_eq!(hashes(&dir), after_delete);
    assert_eq!(files_in(&trash.join("screen-0")).len(), 2);
    assert_eq!(std::fs::read(dir.join("session.json")).unwrap(), manifest_after_delete);
    assert_eq!(session::read_cuts(&dir)[0].state, CutState::Pending);
    assert!(trash.join("undo-failed.json").is_file());
    assert!(std::fs::read_to_string(dir.join("logs").join("edit.log")).unwrap().contains("undo failed"));

    // The timer fires long after the window: it must do nothing to this cut.
    edit::finalize_due_cuts_at(&dir, r.cut.created_epoch_ms + 60_000).unwrap();
    assert_eq!(session::read_cuts(&dir)[0].state, CutState::Pending);
    assert_eq!(hashes(&dir), after_delete, "nothing trimmed");
    assert_eq!(files_in(&trash.join("screen-0")).len(), 2, "nothing deleted");

    // A retried undo, with the way clear, restores everything.
    edit::undo_cut(&dir, &r.cut.id).unwrap();
    assert_eq!(hashes(&dir), before);
    assert!(!trash.exists());
}

#[test]
fn a_damaged_segment_keeps_its_original() {
    let (_root, dir) = build_session("damaged");
    let base = base_epoch();
    // The manifest says segment 4 (t+40s) runs 10s; the file holds 6s, so its
    // tail piece cannot end where the original should.
    let seg4 = name_at(base + 40_000, "mp4");
    std::fs::copy(templates().join("short.mp4"), dir.join("screen-0").join(&seg4)).unwrap();
    let seg4_hash = hashes(&dir)[&format!("screen-0/{}", seg4)].clone();

    let (from, to) = cut_range();
    edit::delete_range(&dir, from, to, None).unwrap();
    let m = edit::finalize_cuts(&dir).unwrap();

    assert_eq!(m.cuts[0].state, CutState::Final);
    assert!(seg_files(&m, "screen-0").contains(&seg4), "the original stays in the manifest");
    assert_eq!(hashes(&dir)[&format!("screen-0/{}", seg4)], seg4_hash, "and on disk, untouched");
    assert!(tmp_files(&dir).is_empty());
    let log = std::fs::read_to_string(dir.join("logs").join("edit.log")).unwrap();
    assert!(log.contains(&seg4) && log.contains("tail piece"), "{}", log);
    // The good trims still happened.
    assert_eq!(seg_files(&m, "mic").len(), 2);
    assert!(seg_files(&m, "screen-0").len() == 4);
}

#[test]
fn a_torn_cuts_line_does_not_swallow_the_next() {
    let dir = std::env::temp_dir()
        .join("chirptime-edit-cuts")
        .join(format!("torn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cuts.jsonl"), b"{\"id\":\"cut-1\",\"from_ep").unwrap();
    let cut = session::Cut {
        id: "cut-2".into(),
        from_epoch_ms: 1,
        to_epoch_ms: 2,
        tracks: None,
        created_epoch_ms: 3,
        state: CutState::Pending,
        freed_bytes: 0,
    };
    session::append_cut(&dir, &cut).unwrap();
    assert_eq!(session::read_cuts(&dir), vec![cut]);
}

#[test]
fn a_mark_note_is_set_cleared_and_survives_a_rebuild() {
    let dir = std::env::temp_dir()
        .join("chirptime-edit-cuts")
        .join(format!("notes-{}", std::process::id()))
        .join(ID);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let base = base_epoch();
    let first = Marker { epoch_ms: base + 5_000, label: String::new() };
    let second = Marker { epoch_ms: base + 30_000, label: "old".into() };
    let mut m = SessionManifest::new(ID.to_string(), base, 10, Rect::default());
    m.markers = vec![first.clone(), second.clone()];
    m.ended_epoch_ms = Some(base + AUDIO_LEN_MS);
    session::write_manifest(&dir, &m).unwrap();
    session::append_marker(&dir, &first).unwrap();
    session::append_marker(&dir, &second).unwrap();

    let set = edit::set_marker_label(&dir, first.epoch_ms, "  Bug repro ").unwrap();
    assert_eq!(set.label, "Bug repro");
    edit::set_marker_label(&dir, second.epoch_ms, "").unwrap();

    let labels = |ms: &[Marker]| ms.iter().map(|mk| (mk.epoch_ms, mk.label.clone())).collect::<Vec<_>>();
    let want = vec![(first.epoch_ms, "Bug repro".to_string()), (second.epoch_ms, String::new())];
    assert_eq!(labels(&session::read_manifest(&dir).unwrap().markers), want);
    // the sidecar a rebuild reads: one entry per instant, last label wins
    assert_eq!(labels(&session::read_markers(&dir)), want);

    assert_eq!(
        edit::set_marker_label(&dir, base + 1, "nope").unwrap_err(),
        edit::NO_MARK_ERR
    );
}
