//! Thumbnails for the file server's `/thumb` route (`session::thumb_lookup`,
//! `session::render_thumb`) and their purge when time is deleted
//! (`edit::purge_thumbs`), on synthetic sessions.
//!
//!     cargo test --test thumbs
//!
//! Footage comes from ffmpeg's `lavfi` `testsrc2`, written into a temp dir
//! under real `-strftime` filenames. Nothing here reads or writes the real
//! recordings folder, opens a capture device, or records.

use chirptime_lib::edit;
use chirptime_lib::session::{
    self, Cut, CutState, Rect, Segment, SessionManifest, Thumb, ThumbFrame, ThumbLookup, ThumbMiss,
    Track, TrackStatus,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const ID: &str = "2026-01-03_10-00-00";
const SEG_MS: i64 = 10_000;

fn ffmpeg_exe() -> String {
    "ffmpeg".to_string()
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new(ffmpeg_exe())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg is needed for these tests");
    assert!(out.status.success(), "ffmpeg {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
}

fn ffprobe() -> String {
    session::find_ffprobe().expect("ffprobe is needed for these tests")
}

/// One 10s 320x240 clip, generated once and copied into every session.
fn template() -> &'static PathBuf {
    static T: OnceLock<PathBuf> = OnceLock::new();
    T.get_or_init(|| {
        let dir = std::env::temp_dir()
            .join("chirptime-thumbs")
            .join(format!("templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("video.mp4");
        ffmpeg(&[
            "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30", "-t", "10",
            "-c:v", "libx264", "-g", "60", "-pix_fmt", "yuv420p", "-f", "mp4",
            &p.to_string_lossy(),
        ]);
        p
    })
}

fn base() -> i64 {
    session::epoch_from_session_id(ID).unwrap()
}

fn name_at(epoch_ms: i64) -> String {
    use chrono::TimeZone;
    let t = chrono::Local.timestamp_millis_opt(epoch_ms).single().unwrap();
    format!("{}.mp4", t.format(session::SEGMENT_STRFTIME))
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
        start_epoch_ms: segments.first().map(|s| s.start_epoch_ms).unwrap_or(0),
        dir: id.to_string(),
        segments,
        gaps: Vec::new(),
        status: TrackStatus::Stopped,
        error: None,
        restarts: 0,
    }
}

fn seg(file: &str, start: i64, dur: i64) -> Segment {
    Segment {
        file: file.to_string(),
        start_epoch_ms: start,
        duration_ms: dur,
        bytes: 1_000,
    }
}

/// `<root>/<ID>` with `screen-0` (three 10s segments from t), `webcam` (the
/// same) and nothing on disk for audio. Returns (root, session dir).
fn build_session(test: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir()
        .join("chirptime-thumbs")
        .join(format!("{}-{}", test, std::process::id()))
        .join("sessions");
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join(ID);
    let mut m = SessionManifest::new(ID.to_string(), base(), 10, Rect::default());
    for (id, kind) in [("screen-0", "screen"), ("webcam", "webcam")] {
        let tdir = dir.join(id);
        std::fs::create_dir_all(&tdir).unwrap();
        let mut segs = Vec::new();
        for i in 0..3 {
            let name = name_at(base() + i * SEG_MS);
            std::fs::copy(template(), tdir.join(&name)).unwrap();
            let p = session::probe_segment(&ffprobe(), &tdir.join(&name)).unwrap();
            segs.push(Segment {
                bytes: std::fs::metadata(tdir.join(&name)).unwrap().len(),
                ..seg(&name, base() + i * SEG_MS, p.duration_ms)
            });
        }
        m.tracks.push(track(id, kind, segs));
    }
    m.ended_epoch_ms = Some(base() + 3 * SEG_MS);
    session::write_manifest(&dir, &m).unwrap();
    (root, dir)
}

/// `thumb_lookup` + `render_thumb`, the way the route runs them.
fn thumb(dir: &Path, track: &str, epoch_ms: i64, w: u32) -> Thumb {
    match session::thumb_lookup(dir, track, epoch_ms, w) {
        Ok(ThumbLookup::Cached(p)) => Thumb::Ready(p),
        Ok(ThumbLookup::Render(job)) => session::render_thumb(&ffmpeg_exe(), dir, &job).unwrap(),
        Err(m) => Thumb::Missing(m),
    }
}

fn ready(t: Thumb) -> PathBuf {
    match t {
        Thumb::Ready(p) => p,
        Thumb::Missing(m) => panic!("expected a thumbnail, got {:?}", m),
    }
}

/// Every file under `<session>/thumbs`, as `track/width/name`.
fn cached(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let root = dir.join(session::THUMBS_DIR);
    for t in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        for w in std::fs::read_dir(t.path()).into_iter().flatten().flatten() {
            for f in std::fs::read_dir(w.path()).into_iter().flatten().flatten() {
                out.push(format!(
                    "{}/{}/{}",
                    t.file_name().to_string_lossy(),
                    w.file_name().to_string_lossy(),
                    f.file_name().to_string_lossy()
                ));
            }
        }
    }
    out.sort();
    out
}

fn key(track: &str, w: u32, epoch_ms: i64) -> String {
    format!("{}/{}/{}.jpg", track, w, epoch_ms)
}

// ---------------------------------------------------------------------------

#[test]
fn resolution_follows_segments_gaps_and_cuts() {
    let t0 = 1_000_000;
    let mut m = SessionManifest::new("s".to_string(), t0, 10, Rect::default());
    m.tracks.push(track(
        "screen-0",
        "screen",
        vec![
            seg("a.mp4", t0, 4_000),
            seg("b.mp4", t0 + 2_500, 14_000), // overlaps a by 1.5s
            seg("d.mp4", t0 + 40_000, 300),   // shorter than the end margin
        ],
    ));
    m.tracks.push(track("mic", "mic", vec![seg("m.mp4", t0, 60_000)]));
    m.tracks.push(track("webcam", "webcam", vec![seg("w.mp4", t0, 60_000)]));
    let frame = |file: &str, offset_ms: i64| ThumbFrame {
        dir: "screen-0".to_string(),
        file: file.to_string(),
        offset_ms,
    };

    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 + 1_000), Ok(frame("a.mp4", 1_000)));
    // Inside the overlap the later segment wins.
    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 + 3_000), Ok(frame("b.mp4", 500)));
    // The seek stays 500ms clear of a segment's end, and never goes negative.
    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 + 16_400), Ok(frame("b.mp4", 13_500)));
    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 + 40_200), Ok(frame("d.mp4", 0)));
    // A segment's end is exclusive; the hole after it is a gap.
    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 + 16_500), Err(ThumbMiss::Gap));
    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 - 1), Err(ThumbMiss::Gap));
    // Audio has no picture; unknown tracks are not found.
    assert_eq!(session::resolve_thumb(&m, "mic", t0 + 1_000), Err(ThumbMiss::NoTrack));
    assert_eq!(session::resolve_thumb(&m, "screen-9", t0 + 1_000), Err(ThumbMiss::NoTrack));

    let cut = |id: &str, from: i64, to: i64, tracks: Option<Vec<String>>, state: CutState| Cut {
        id: id.to_string(),
        from_epoch_ms: from,
        to_epoch_ms: to,
        tracks,
        created_epoch_ms: 0,
        state,
        freed_bytes: 0,
    };
    m.cuts = vec![
        cut("cut-1", t0 + 1_000, t0 + 2_000, None, CutState::Pending),
        cut("cut-2", t0 + 5_000, t0 + 6_000, Some(vec!["webcam".to_string()]), CutState::Final),
        cut("cut-3", t0 + 7_000, t0 + 8_000, None, CutState::Undone),
    ];
    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 + 1_000), Err(ThumbMiss::Cut));
    assert_eq!(session::resolve_thumb(&m, "webcam", t0 + 1_999), Err(ThumbMiss::Cut));
    // A cut's end is exclusive too.
    assert_eq!(session::resolve_thumb(&m, "screen-0", t0 + 2_000), Ok(frame("a.mp4", 2_000)));
    // A per-track cut hides only its tracks; an undone cut hides nothing.
    assert_eq!(session::resolve_thumb(&m, "webcam", t0 + 5_500), Err(ThumbMiss::Cut));
    assert!(session::resolve_thumb(&m, "screen-0", t0 + 5_500).is_ok());
    assert!(session::resolve_thumb(&m, "screen-0", t0 + 7_500).is_ok());
}

#[test]
fn cache_path_and_width_clamp() {
    let dir = Path::new("s");
    assert_eq!(
        session::thumb_path(dir, "screen-0", 320, 1_767_430_800_000),
        dir.join("thumbs").join("screen-0").join("320").join("1767430800000.jpg")
    );
    assert_eq!(session::clamp_thumb_width(None), session::THUMB_DEFAULT_W);
    assert_eq!(session::clamp_thumb_width(Some(1)), 64);
    assert_eq!(session::clamp_thumb_width(Some(200)), 200);
    assert_eq!(session::clamp_thumb_width(Some(5_000)), 960);
}

#[test]
fn renders_once_then_serves_from_the_cache() {
    let (_root, dir) = build_session("render");
    let at = base() + 12_000;

    let job = match session::thumb_lookup(&dir, "screen-0", at, 200).unwrap() {
        ThumbLookup::Render(job) => job,
        other => panic!("expected a render, got {:?}", other),
    };
    assert_eq!(job.offset_ms, 2_000);
    assert_eq!(job.src, dir.join("screen-0").join(name_at(base() + SEG_MS)));
    let path = ready(session::render_thumb(&ffmpeg_exe(), &dir, &job).unwrap());
    assert_eq!(path, session::thumb_path(&dir, "screen-0", 200, at));

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..2], b"\xFF\xD8", "a JPEG");
    let out = Command::new(ffprobe())
        .args(["-v", "error", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "200,150");

    assert_eq!(session::thumb_lookup(&dir, "screen-0", at, 200), Ok(ThumbLookup::Cached(path)));
    // Nothing half-written is left behind.
    assert_eq!(cached(&dir), vec![key("screen-0", 200, at)]);
    // The webcam renders the same way; the seek near a segment's end still finds a frame.
    ready(thumb(&dir, "webcam", base() + 29_990, 64));
}

#[test]
fn a_cut_purges_its_thumbnails_and_undo_lets_them_regenerate() {
    let (root, dir) = build_session("purge");
    let b = base();
    let (from, to) = (b + 12_000, b + 25_000);
    let inside = [b + 12_000, b + 20_000, b + 24_990];
    let outside = [b + 5_000, b + 11_990, b + 25_000];
    for at in inside.iter().chain(&outside) {
        ready(thumb(&dir, "screen-0", *at, 320));
        ready(thumb(&dir, "webcam", *at, 320));
    }
    ready(thumb(&dir, "screen-0", b + 20_000, 96));
    // A half-written one from a crash inside the range goes too.
    let torn = session::thumb_path(&dir, "screen-0", 320, b + 21_000).with_extension("jpg.1-1.tmp");
    std::fs::write(&torn, b"x").unwrap();
    let all = cached(&dir);

    // Screen only: its thumbnails inside the range go at delete time, the
    // webcam's and everything outside stay.
    let r = edit::delete_range(&dir, from, to, Some(vec!["screen-0".to_string()])).unwrap();
    let gone: Vec<String> = inside
        .iter()
        .map(|at| key("screen-0", 320, *at))
        .chain([key("screen-0", 96, b + 20_000)])
        .collect();
    let mut left: Vec<String> = all.iter().filter(|k| !gone.contains(k)).cloned().collect();
    left.retain(|k| !k.ends_with(".tmp"));
    assert_eq!(cached(&dir), left);
    for at in inside {
        assert_eq!(session::thumb_lookup(&dir, "screen-0", at, 320), Err(ThumbMiss::Cut));
        assert!(matches!(session::thumb_lookup(&dir, "webcam", at, 320), Ok(ThumbLookup::Cached(_))));
    }

    // Undo: the footage is back, and so are its thumbnails, on request.
    edit::undo_cut(&dir, &r.cut.id).unwrap();
    for at in inside {
        assert!(matches!(
            session::thumb_lookup(&dir, "screen-0", at, 320),
            Ok(ThumbLookup::Render(_))
        ));
        ready(thumb(&dir, "screen-0", at, 320));
    }

    // Every track, finalized: nothing inside the range survives as a JPEG.
    edit::delete_range(&dir, from, to, None).unwrap();
    let m = edit::finalize_cuts(&dir).unwrap();
    assert_eq!(m.cuts.len(), 1);
    assert_eq!(m.cuts[0].state, CutState::Final);
    let mut want: Vec<String> = outside
        .iter()
        .flat_map(|at| [key("screen-0", 320, *at), key("webcam", 320, *at)])
        .collect();
    want.sort();
    assert_eq!(cached(&dir), want);
    for at in inside {
        assert_eq!(session::thumb_lookup(&dir, "webcam", at, 320), Err(ThumbMiss::Cut));
    }
    // Outside the range, the trimmed segments still render.
    ready(thumb(&dir, "screen-0", b + 11_000, 320));
    ready(thumb(&dir, "webcam", b + 26_000, 320));

    // The cache is not footage: the rebuild indexes no `thumbs` track, and the
    // session's listed size is its manifest's.
    let rebuilt = session::rebuild_manifest_from_disk(&dir).unwrap();
    let ids: Vec<&str> = rebuilt.tracks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["screen-0", "webcam"]);
    let listed = edit::visible_sessions(&root);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].bytes, session::read_manifest(&dir).unwrap().total_bytes());
}

#[test]
fn a_render_that_lands_inside_a_new_cut_removes_itself() {
    let (_root, dir) = build_session("race");
    let at = base() + 15_000;
    let job = match session::thumb_lookup(&dir, "screen-0", at, 320).unwrap() {
        ThumbLookup::Render(job) => job,
        other => panic!("expected a render, got {:?}", other),
    };
    // The delete commits (and purges) while ffmpeg would be running.
    edit::delete_range(&dir, base() + 14_000, base() + 16_000, None).unwrap();
    assert_eq!(
        session::render_thumb(&ffmpeg_exe(), &dir, &job).unwrap(),
        Thumb::Missing(ThumbMiss::Cut)
    );
    assert!(!job.dest.exists());
    assert!(cached(&dir).is_empty());
}

#[test]
fn unfinished_or_missing_footage_is_a_miss_not_an_error() {
    let (root, dir) = build_session("unfinished");
    let b = base();

    // The segment still being written: no moov atom yet.
    let last = dir.join("webcam").join(name_at(b + 2 * SEG_MS));
    let bytes = std::fs::read(&last).unwrap();
    std::fs::write(&last, &bytes[..bytes.len() / 2]).unwrap();
    assert_eq!(thumb(&dir, "webcam", b + 25_000, 320), Thumb::Missing(ThumbMiss::NotReady));

    // Listed in the manifest but gone from disk.
    std::fs::remove_file(dir.join("screen-0").join(name_at(b))).unwrap();
    assert_eq!(
        session::thumb_lookup(&dir, "screen-0", b + 1_000, 320),
        Err(ThumbMiss::NotReady)
    );

    // Past the end of the session, and a session with no manifest.
    assert_eq!(session::thumb_lookup(&dir, "screen-0", b + 31_000, 320), Err(ThumbMiss::Gap));
    let bare = root.join("2026-01-03_11-00-00");
    std::fs::create_dir_all(&bare).unwrap();
    assert_eq!(
        session::thumb_lookup(&bare, "screen-0", b, 320),
        Err(ThumbMiss::NoSession)
    );
    assert!(cached(&dir).is_empty());
}
