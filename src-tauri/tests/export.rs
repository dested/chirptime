//! Export composer (`export.rs`): the filter graph for each layout, cut and
//! idle subtraction, follow-mouse segmentation, concat placement, Share's
//! two-pass bitrates, and one small real-ffmpeg export of a generated session.
//!
//!     cargo test --test export
//!
//! Everything but the last test is pure. The real one writes a 10 s
//! `testsrc2` session into a temp dir; nothing here touches the real
//! recordings folder, opens a device, or records.

use chirptime_lib::cursor::CursorSample;
use chirptime_lib::export::{
    self, CameraOpts, Corner, ExportRequest, InputKind, JobCtl, Layout, OutMap, RunError, ScreenRect,
    Shape, SizeMode, SoundOpts, Span, Speed,
};
use chirptime_lib::recorder::EncoderCaps;
use chirptime_lib::session::{
    self, Cut, CutState, Gap, MonitorRef, Rect, Segment, SessionManifest, Track, TrackStatus,
};
use std::path::PathBuf;
use std::process::Command;

const T0: i64 = 1_785_600_000_000;
const MIN: i64 = 60_000;

fn seg(file: &str, start: i64, dur: i64) -> Segment {
    Segment {
        file: file.to_string(),
        start_epoch_ms: start,
        duration_ms: dur,
        bytes: 6_000_000,
    }
}

fn track(id: &str, kind: &str, w: u32, h: u32, monitor: Option<MonitorRef>, segments: Vec<Segment>) -> Track {
    Track {
        id: id.to_string(),
        kind: kind.to_string(),
        label: id.to_string(),
        monitor,
        codec: "h264".into(),
        width: w,
        height: h,
        fps: 30,
        start_epoch_ms: segments.first().map(|s| s.start_epoch_ms).unwrap_or(T0),
        dir: id.to_string(),
        segments,
        gaps: Vec::new(),
        status: TrackStatus::Stopped,
        error: None,
        restarts: 0,
    }
}

fn mon(x: i32, y: i32, w: u32, h: u32) -> Option<MonitorRef> {
    Some(MonitorRef {
        output_idx: 0,
        adapter: 0,
        x,
        y,
        w,
        h,
    })
}

/// Minutes `[from, to)` as 60 s segments.
fn minutes(range: std::ops::Range<i64>) -> Vec<Segment> {
    range.map(|i| seg(&format!("{:03}.mp4", i), T0 + i * MIN, MIN)).collect()
}

const IDLE: &str = "idle — no keyboard or mouse activity";

/// Ten minutes: two screens side by side (2560×1440 at 0,0 and 1920×1080 to
/// its right), a webcam, mic and system audio. Minute 6–7 is idle on every
/// track; minute 3–4 is cut from every track; screen-1 went dark for minute 8–9.
fn manifest() -> SessionManifest {
    let mut m = SessionManifest::new("2026-08-01_10-00-00".into(), T0, 60, Rect { x: 0, y: 0, w: 4480, h: 1440 });
    let video = |id: &str, kind: &str, w, h, mo| {
        let mut segs = minutes(0..6);
        segs.extend(minutes(7..10));
        let mut t = track(id, kind, w, h, mo, segs);
        t.gaps.push(Gap {
            from_epoch_ms: T0 + 6 * MIN,
            to_epoch_ms: T0 + 7 * MIN,
            reason: IDLE.into(),
        });
        t
    };
    m.tracks.push(video("screen-0", "screen", 2560, 1440, mon(0, 0, 2560, 1440)));
    let mut s1 = video("screen-1", "screen", 1920, 1080, mon(2560, 0, 1920, 1080));
    s1.segments.retain(|s| s.start_epoch_ms != T0 + 8 * MIN);
    s1.gaps.push(Gap {
        from_epoch_ms: T0 + 8 * MIN,
        to_epoch_ms: T0 + 9 * MIN,
        reason: "887a0026 access lost".into(),
    });
    m.tracks.push(s1);
    m.tracks.push(video("webcam", "webcam", 1280, 720, None));
    m.tracks.push(track("mic", "mic", 0, 0, None, vec![seg("mic.mp4", T0, 10 * MIN)]));
    m.tracks.push(track("sysaudio", "system-audio", 0, 0, None, vec![seg("sys.mp4", T0, 10 * MIN)]));
    m.cuts.push(Cut {
        id: "cut-1".into(),
        from_epoch_ms: T0 + 3 * MIN,
        to_epoch_ms: T0 + 4 * MIN,
        tracks: None,
        created_epoch_ms: T0 + 20 * MIN,
        state: CutState::Final,
        freed_bytes: 0,
    });
    m.ended_epoch_ms = Some(T0 + 10 * MIN);
    m
}

fn req(layout: Layout) -> ExportRequest {
    ExportRequest {
        session_id: "2026-08-01_10-00-00".into(),
        from_ms: T0,
        to_ms: T0 + 10 * MIN,
        layout,
        screen: None,
        camera: CameraOpts::default(),
        sound: SoundOpts::default(),
        speed: Speed::X1,
        skip_idle: false,
        size: SizeMode::P1080,
        raw_tracks: None,
        title: None,
    }
}

fn sp(a: i64, b: i64) -> Span {
    Span::new(T0 + a, T0 + b)
}

// ---------------------------------------------------------------------------
// Cuts and idle
// ---------------------------------------------------------------------------

#[test]
fn cuts_come_out_and_idle_only_when_asked() {
    let m = manifest();
    let all = export::kept_pieces(&m, T0, T0 + 10 * MIN, false);
    assert_eq!(all, vec![sp(0, 3 * MIN), sp(4 * MIN, 10 * MIN)]);
    let skip = export::kept_pieces(&m, T0, T0 + 10 * MIN, true);
    assert_eq!(skip, vec![sp(0, 3 * MIN), sp(4 * MIN, 6 * MIN), sp(7 * MIN, 10 * MIN)]);

    // A range that starts inside the cut starts where the cut ends.
    assert_eq!(export::kept_pieces(&m, T0 + 3 * MIN + 30_000, T0 + 5 * MIN, false), vec![sp(4 * MIN, 5 * MIN)]);
    // A cut on the mic alone leaves the session's time alone.
    let mut m2 = manifest();
    m2.cuts.push(Cut {
        id: "cut-2".into(),
        from_epoch_ms: T0 + MIN,
        to_epoch_ms: T0 + 2 * MIN,
        tracks: Some(vec!["mic".into()]),
        created_epoch_ms: T0 + 21 * MIN,
        state: CutState::Pending,
        freed_bytes: 0,
    });
    assert_eq!(export::kept_pieces(&m2, T0, T0 + 10 * MIN, false), all);
    assert_eq!(export::track_holes(&m2, "mic"), vec![sp(MIN, 2 * MIN), sp(3 * MIN, 4 * MIN)]);
    // Undone cuts are ignored.
    m2.cuts[1].state = CutState::Undone;
    assert_eq!(export::track_holes(&m2, "mic"), vec![sp(3 * MIN, 4 * MIN)]);

    // Output time: the pieces end to end.
    let out = OutMap::new(skip);
    assert_eq!(out.total_ms, 8 * MIN);
    assert_eq!(out.map(&[sp(2 * MIN, 5 * MIN)]), vec![Span::new(2 * MIN, 4 * MIN)]);
    assert_eq!(out.map(&[sp(6 * MIN + 10, 7 * MIN + MIN / 2)]), vec![Span::new(5 * MIN, 5 * MIN + MIN / 2)]);
}

#[test]
fn idle_is_where_no_picture_was_kept() {
    let mut m = manifest();
    // The webcam kept a frame inside the idle minute: that part is not idle.
    m.tracks[2].segments.push(seg("x.mp4", T0 + 6 * MIN + 20_000, 10_000));
    assert_eq!(
        export::idle_spans(&m),
        vec![sp(6 * MIN, 6 * MIN + 20_000), sp(6 * MIN + 30_000, 7 * MIN)]
    );
    assert!(export::is_skippable_reason("paused"));
    assert!(!export::is_skippable_reason("display went dark"));
}

#[test]
fn span_math() {
    let s = |a, b| Span::new(a, b);
    assert_eq!(export::merge_spans(vec![s(5, 7), s(0, 2), s(2, 3), s(9, 9)], 0), vec![s(0, 3), s(5, 7)]);
    assert_eq!(export::merge_spans(vec![s(0, 2), s(4, 5)], 2), vec![s(0, 5)]);
    assert_eq!(export::subtract_spans(&[s(0, 10)], &[s(2, 3), s(5, 12)]), vec![s(0, 2), s(3, 5)]);
    assert_eq!(export::intersect_spans(&[s(0, 4), s(6, 10)], &[s(3, 7)]), vec![s(3, 4), s(6, 7)]);
}

// ---------------------------------------------------------------------------
// Concat placement
// ---------------------------------------------------------------------------

#[test]
fn concat_places_segments_at_their_wall_clock() {
    let t = track(
        "screen-0",
        "screen",
        1920,
        1080,
        None,
        vec![
            seg("a.mp4", T0, 60_000),
            seg("b.mp4", T0 + 58_000, 60_000), // overlaps a by 2 s
            seg("c.mp4", T0 + 200_000, 60_000), // after a gap
            seg("c2.mp4", T0 + 200_000, 50_000), // same start: the later one wins
            seg("z.mp4", T0 + 900_000, 60_000), // outside the kept spans
        ],
    );
    let e = export::concat_entries(&t, &[sp(10_000, 300_000)]);
    let files: Vec<&str> = e.iter().map(|x| x.file.as_str()).collect();
    assert_eq!(files, vec!["screen-0/a.mp4", "screen-0/b.mp4", "screen-0/c2.mp4"]);
    // a is trimmed where b starts, and b starts 58 s in.
    assert_eq!((e[0].outpoint_ms, e[0].duration_ms), (Some(58_000), 58_000));
    // b plays whole, then the clock jumps over the gap to c's real start.
    assert_eq!((e[1].outpoint_ms, e[1].duration_ms), (None, 142_000));
    assert_eq!((e[2].outpoint_ms, e[2].duration_ms), (None, 50_000));

    let text = export::concat_list_text(std::path::Path::new("C:\\rec\\s"), &e);
    assert!(text.starts_with("ffconcat version 1.0\n"));
    assert!(text.contains("file 'C:/rec/s/screen-0/a.mp4'\noutpoint 58.000\nduration 58.000\n"), "{}", text);

    // Raw tracks trim to the pieces.
    let r = export::raw_entries(&t, &[sp(30_000, 70_000)]);
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].inpoint_ms, r[0].outpoint_ms, r[0].duration_ms), (Some(30_000), Some(58_000), 28_000));
    assert_eq!((r[1].inpoint_ms, r[1].outpoint_ms, r[1].duration_ms), (None, Some(12_000), 12_000));
}

// ---------------------------------------------------------------------------
// Follow mouse
// ---------------------------------------------------------------------------

fn at(t: i64, x: i32, y: i32) -> CursorSample {
    CursorSample { t: T0 + t, x, y }
}

#[test]
fn follow_switches_on_entry_and_ignores_short_visits() {
    let screens = [
        ScreenRect { x: 0, y: 0, w: 2560, h: 1440 },
        ScreenRect { x: 2560, y: 0, w: 1920, h: 1080 },
    ];
    let samples = vec![
        at(0, 100, 100),
        at(5_000, 3000, 100), // a 1 s visit to screen 2: ignored
        at(6_000, 100, 100),
        at(10_000, 3000, 500), // stays 4 s: switch at 10 s
        at(12_000, 99_999, 99_999), // off every screen: keeps screen 2
        at(14_000, 200, 200), // back to 1 until the end
    ];
    let segs = export::follow_segments(&samples, &screens, T0, T0 + 20_000, 0);
    assert_eq!(segs, vec![(sp(0, 10_000), 0), (sp(10_000, 14_000), 1), (sp(14_000, 20_000), 0)]);

    // A visit of exactly the hold time counts.
    let hold = vec![at(0, 3000, 0), at(1_000, 10, 10), at(1_000 + export::FOLLOW_HOLD_MS, 3000, 0)];
    assert_eq!(
        export::follow_segments(&hold, &screens, T0, T0 + 10_000, 0),
        vec![(sp(0, 1_000), 1), (sp(1_000, 2_500), 0), (sp(2_500, 10_000), 1)]
    );
    // The last run counts to the end of the range.
    let tail = vec![at(0, 10, 10), at(9_000, 3000, 0)];
    assert_eq!(export::follow_segments(&tail, &screens, T0, T0 + 10_000, 0), vec![(sp(0, 10_000), 0)]);
    // No samples: the fallback throughout.
    assert_eq!(export::follow_segments(&[], &screens, T0, T0 + 10_000, 1), vec![(sp(0, 10_000), 1)]);
}

#[test]
fn follow_without_mouse_data_uses_the_largest_screen_and_says_so() {
    let m = manifest();
    let plan = export::build_plan(&m, &req(Layout::Devlog), &[], None).unwrap();
    assert!(plan.focus.iter().all(|(_, id)| id == "screen-0"));
    assert!(plan.notes.iter().any(|n| n.contains("No mouse data") && n.contains("Screen 1")), "{:?}", plan.notes);

    // With samples, each screen is an input whose list holds only its minutes.
    let samples = vec![at(0, 100, 100), at(5 * MIN, 3000, 100)];
    let plan = export::build_plan(&m, &req(Layout::Devlog), &samples, None).unwrap();
    assert_eq!(plan.focus, vec![(sp(0, 5 * MIN), "screen-0".into()), (sp(5 * MIN, 10 * MIN), "screen-1".into())]);
    let s1 = plan.inputs.iter().find(|i| i.track_id == "screen-1").unwrap();
    assert!(s1.entries.iter().all(|e| e.start_epoch_ms >= T0 + 4 * MIN), "{:?}", s1.entries);
    assert!(plan.notes.is_empty(), "{:?}", plan.notes);
}

// ---------------------------------------------------------------------------
// Filter graphs, per layout
// ---------------------------------------------------------------------------

fn count(g: &str, needle: &str) -> usize {
    g.matches(needle).count()
}

#[test]
fn devlog_graph() {
    let m = manifest();
    let mut r = req(Layout::Devlog);
    r.screen = Some("screen-1".into());
    let plan = export::build_plan(&m, &r, &[], None).unwrap();
    assert_eq!((plan.width, plan.height), (1920, 1080));
    assert_eq!(plan.composed_ms(), 9 * MIN);
    let g = export::filter_graph(&plan, true);

    // Inputs: screen-1, the webcam, mic, system audio. No screen-0.
    let ids: Vec<&str> = plan.inputs.iter().map(|i| i.track_id.as_str()).collect();
    assert_eq!(ids, vec!["screen-1", "webcam", "mic", "sysaudio"]);
    // The screen contained full frame; the webcam full frame while screen-1
    // was dark (no screen footage) and in the corner the rest of the time.
    assert_eq!(count(&g, "overlay="), 3, "{}", g);
    assert!(g.contains("[s0]scale=1920:1080,setsar=1[l0]"), "{}", g);
    assert!(g.contains("[1:v]select="), "{}", g);
    assert!(g.contains(",split=2[s1][s2]"), "{}", g);
    assert!(g.contains("[s1]scale=1920:1080,crop=1920:1080"), "{}", g);
    // The corner camera: 28% of 1920 = 538 wide, rounded, bottom right.
    assert!(g.contains("[s2]scale=538:302,crop=538:302,format=yuva420p[m2]"), "{}", g);
    assert!(g.contains("[4:v]format=gray[mask2]"), "{}", g);
    assert!(g.contains("[m2][mask2]alphamerge[l2]"), "{}", g);
    assert!(g.contains("[c2][l2]overlay=x=1339:y=735:eof_action=pass:enable='between(t,0.000,300.000)+between(t,360.000,420.000)+between(t,480.000,540.000)'"), "{}", g);
    // Minute 8, the dark one, is minute 7 of the output (after the cut).
    assert!(g.contains("[c1][l1]overlay=x=0:y=0:eof_action=pass:enable='between(t,420.000,480.000)'"), "{}", g);
    assert_eq!(plan.mask, Some((538, 302, Shape::Rounded)));
    // The cut is collapsed: input clock jumps 60 s at minute 4.
    assert!(g.contains("setpts='(T+0.000-gte(T,240.000)*60.000)/TB'"), "{}", g);
    assert!(g.contains("color=c=black:s=1920x1080:r=30:d=540.000[c0]"), "{}", g);
    assert!(g.contains("[c3]format=yuv420p[vout]"), "{}", g);
    // Sound: both, system ducked under the mic.
    assert!(g.contains("[2:a]aselect="), "{}", g);
    assert!(g.contains("volume=0.0dB"), "{}", g);
    assert!(g.contains("volume=-12.0dB"), "{}", g);
    assert!(g.contains("[sys][mic2]sidechaincompress="), "{}", g);
    assert!(g.contains("[mix]anull[aout]"), "{}", g);
    // Without sound (Share's first pass) nothing dangles.
    let g1 = export::filter_graph(&plan, false);
    assert!(!g1.contains("[aout]") && !g1.contains(":a]"), "{}", g1);
}

#[test]
fn short_graph() {
    let m = manifest();
    let mut r = req(Layout::Short);
    r.screen = Some("screen-0".into());
    r.sound = SoundOpts { mic_db: Some(3.0), system_db: None, duck: true };
    let plan = export::build_plan(&m, &r, &[], None).unwrap();
    assert_eq!((plan.width, plan.height), (1080, 1920));
    let g = export::filter_graph(&plan, true);
    // The screen cropped into the top 60%, the camera below it.
    assert!(g.contains("[s0]scale=2048:1152,crop=1080:1152,setsar=1[l0]"), "{}", g);
    assert!(g.contains("crop=1080:768,setsar=1"), "{}", g);
    assert!(g.contains("overlay=x=0:y=1152"), "{}", g);
    assert!(plan.mask.is_none());
    // Mic only, no ducking needed.
    assert!(g.contains("volume=3.0dB"), "{}", g);
    assert!(!g.contains("sidechaincompress"), "{}", g);
    assert!(!plan.inputs.iter().any(|i| i.track_id == "sysaudio"));
}

#[test]
fn side_by_side_graph() {
    let m = manifest();
    let mut r = req(Layout::SideBySide);
    r.screen = Some("screen-0".into());
    r.sound.duck = false;
    let plan = export::build_plan(&m, &r, &[], None).unwrap();
    let g = export::filter_graph(&plan, true);
    // Left half: the 16:9 screen contained in 960×1080, centered vertically.
    assert!(g.contains("[s0]scale=960:540,setsar=1[l0]"), "{}", g);
    assert!(g.contains("overlay=x=0:y=270"), "{}", g);
    // Right half: the camera covering 960×1080.
    assert!(g.contains("crop=960:1080,setsar=1"), "{}", g);
    assert!(g.contains("overlay=x=960:y=0"), "{}", g);
    assert!(g.contains("[mic][sys]amix=inputs=2:normalize=0:duration=first[mix]"), "{}", g);

    // With the camera off the screen takes the whole frame.
    r.camera.enabled = false;
    let plan = export::build_plan(&m, &r, &[], None).unwrap();
    let g = export::filter_graph(&plan, true);
    assert!(g.contains("[s0]scale=1920:1080,setsar=1[l0]"), "{}", g);
    assert_eq!(count(&g, "overlay="), 1);
}

#[test]
fn desk_graph() {
    let m = manifest();
    let mut r = req(Layout::Desk);
    r.camera = CameraOpts { enabled: true, corner: Corner::TopLeft, shape: Shape::Circle, size: 0.2 };
    let plan = export::build_plan(&m, &r, &[], None).unwrap();
    let g = export::filter_graph(&plan, true);
    // Both screens, at their real positions: the 4480×1440 desk scaled by
    // (1920-2·32)/4480 into the frame, centered.
    let ids: Vec<&str> = plan.inputs.iter().filter(|i| i.kind == InputKind::Video).map(|i| i.track_id.as_str()).collect();
    assert_eq!(ids, vec!["screen-0", "screen-1", "webcam"]);
    assert!(g.contains("[s0]scale=1060:596,setsar=1[l0]"), "{}", g);
    assert!(g.contains("[s1]scale=796:448,setsar=1[l1]"), "{}", g);
    assert!(g.contains("[c0][l0]overlay=x=32:y=242"), "{}", g);
    assert!(g.contains("[c1][l1]overlay=x=1093:y=242"), "{}", g);
    // A circle in the top-left corner: 20% of 1920 = 384, its 16:9 height 216 square.
    assert_eq!(plan.mask, Some((216, 216, Shape::Circle)));
    assert!(g.contains("crop=216:216,format=yuva420p"), "{}", g);
    assert!(g.contains("overlay=x=43:y=43"), "{}", g);
}

#[test]
fn speed_and_timelapse() {
    let m = manifest();
    let mut r = req(Layout::Devlog);
    r.screen = Some("screen-0".into());
    r.speed = Speed::X4;
    r.skip_idle = true;
    let plan = export::build_plan(&m, &r, &[], None).unwrap();
    assert_eq!(plan.composed_ms(), 8 * MIN);
    assert_eq!(plan.output_ms(), 2 * MIN);
    let g = export::filter_graph(&plan, true);
    assert!(g.contains("r=30/4:d=480.000"), "{}", g);
    assert!(g.contains("setpts=PTS/4,fps=30,format=yuv420p[vout]"), "{}", g);
    assert!(g.contains("[mix]atempo=2,atempo=2[aout]"), "{}", g);
    // Screen-0 never went dark: no camera-only stretch.
    assert_eq!(count(&g, "overlay="), 2, "{}", g);

    r.speed = Speed::Timelapse;
    let plan = export::build_plan(&m, &r, &[], None).unwrap();
    assert_eq!(plan.output_ms(), 16_000);
    assert!(!plan.has_audio());
    assert!(plan.notes.iter().any(|n| n == "Timelapse has no sound"));
    let g = export::filter_graph(&plan, true);
    assert!(g.contains(":r=1:d=480.000"), "{}", g);
    assert!(g.contains("setpts=PTS/30,fps=30"), "{}", g);
    assert!(!g.contains("[aout]"), "{}", g);
}

#[test]
fn bad_requests_are_refused_in_words() {
    let m = manifest();
    let mut r = req(Layout::Devlog);
    r.from_ms = T0 + 3 * MIN + 1_000;
    r.to_ms = T0 + 3 * MIN + 50_000;
    let e = export::build_plan(&m, &r, &[], None).unwrap_err();
    assert!(e.contains("Nothing is left"), "{}", e);
    r = req(Layout::Devlog);
    r.screen = Some("screen-9".into());
    assert!(export::build_plan(&m, &r, &[], None).unwrap_err().contains("no screen named screen-9"));
}

// ---------------------------------------------------------------------------
// Size
// ---------------------------------------------------------------------------

#[test]
fn share_two_pass_bitrates() {
    // One minute: the whole budget is far more than 1080p needs.
    let p = export::share_plan(MIN, true).unwrap();
    assert_eq!((p.p, p.video_kbps, p.audio_kbps), (1080, 3070, 96));
    // Three minutes: 25 MB · 0.95 · 8 / 180 s = 1055 kbps → 720p.
    let p = export::share_plan(3 * MIN, true).unwrap();
    assert_eq!((p.p, p.video_kbps, p.audio_kbps), (720, 959, 96));
    // Eight minutes, silent: 395 kbps → 480p.
    let p = export::share_plan(8 * MIN, false).unwrap();
    assert_eq!((p.p, p.video_kbps, p.audio_kbps), (480, 395, 0));
    // Twenty-four minutes can't fit.
    let e = export::share_plan(24 * MIN, true).unwrap_err();
    assert!(e.starts_with("Too long for 25 MB: 24 minutes would drop below 480p"), "{}", e);
    // Whatever it picks, the file fits.
    for mins in [1, 2, 5, 9] {
        let p = export::share_plan(mins * MIN, true).unwrap();
        let bytes = (p.video_kbps + p.audio_kbps) as f64 * 1000.0 / 8.0 * (mins * 60) as f64;
        assert!(bytes <= 25_000_000.0 * 0.95, "{} min: {} bytes", mins, bytes);
    }

    // The plan takes the height Share settled on.
    let mut r = req(Layout::Devlog);
    r.size = SizeMode::Share;
    let plan = export::build_plan(&manifest(), &r, &[], Some(720)).unwrap();
    assert_eq!((plan.width, plan.height), (1280, 720));
    r.layout = Layout::Short;
    let plan = export::build_plan(&manifest(), &r, &[], Some(480)).unwrap();
    assert_eq!((plan.width, plan.height), (480, 854));
}

#[test]
fn original_size_follows_the_tallest_source() {
    let mut r = req(Layout::Devlog);
    r.size = SizeMode::Original;
    r.screen = Some("screen-0".into());
    let plan = export::build_plan(&manifest(), &r, &[], None).unwrap();
    assert_eq!((plan.width, plan.height), (2560, 1440));
    assert_eq!(export::pick_encoder(&EncoderCaps { h264_nvenc: true, ..Default::default() }, 2560, 1440), export::Encoder::Nvenc);
    assert_eq!(export::pick_encoder(&EncoderCaps { h264_nvenc: true, h264_qsv: true, ..Default::default() }, 7680, 4320), export::Encoder::Qsv);
    assert_eq!(export::pick_encoder(&EncoderCaps::default(), 1920, 1080), export::Encoder::X264);
}

#[test]
fn estimate_reports_follow_fallback_and_share_refusal() {
    let m = manifest();
    let caps = EncoderCaps { h264_nvenc: true, ..Default::default() };
    let est = export::estimate(&m, &req(Layout::Devlog), &[], &caps);
    assert_eq!(est.error, None);
    assert_eq!((est.output_ms, est.width, est.encoder.as_str()), (9 * MIN, 1920, "NVENC"));
    assert!(est.bytes > 50_000_000 && est.bytes < 500_000_000, "{}", est.bytes);
    assert!(est.notes.iter().any(|n| n.contains("No mouse data")));

    let mut r = req(Layout::Devlog);
    r.size = SizeMode::Share;
    // Nine minutes squeeze into 25 MB at 480p.
    let est = export::estimate(&m, &r, &[], &caps);
    assert_eq!((est.error.as_deref(), est.height), (None, 480));
    // Half an hour doesn't.
    let mut long = manifest();
    long.tracks[0].segments.push(seg("long.mp4", T0 + 10 * MIN, 20 * MIN));
    long.ended_epoch_ms = Some(T0 + 30 * MIN);
    let est = export::estimate(&long, &ExportRequest { to_ms: T0 + 30 * MIN, ..r.clone() }, &[], &caps);
    assert!(est.error.as_deref().is_some_and(|e| e.starts_with("Too long for 25 MB")), "{:?}", est.error);
    r.to_ms = T0 + 2 * MIN;
    let est = export::estimate(&m, &r, &[], &caps);
    assert_eq!((est.error, est.encoder.as_str(), est.height), (None, "x264", 720));
    assert!(est.bytes <= 25_000_000);

    r.raw_tracks = Some(vec!["screen-0".into(), "mic".into()]);
    r.to_ms = T0 + 10 * MIN;
    let est = export::estimate(&m, &r, &[], &caps);
    // screen-0: 8 of its 9 minutes survive the cut; the mic 9 of 10 minutes.
    assert_eq!(est.bytes, 8 * 6_000_000 + 5_400_000);
}

#[test]
fn camera_mask_and_names() {
    let pgm = export::mask_pgm(100, 56, Shape::Rounded);
    let header = b"P5\n100 56\n255\n";
    assert_eq!(&pgm[..header.len()], header);
    let px = |x: usize, y: usize| pgm[header.len() + y * 100 + x];
    assert_eq!(px(0, 0), 0);
    assert_eq!(px(50, 28), 255);
    assert_eq!(px(50, 0), 255);
    let circle = export::mask_pgm(64, 64, Shape::Circle);
    let h = b"P5\n64 64\n255\n".len();
    assert_eq!(circle[h + 2 * 64 + 2], 0);
    assert!(circle[h + 32 * 64] > 200);
    assert_eq!(circle[h + 32 * 64 + 32], 255);

    let stem = export::export_stem(T0, "Bug: the <thing>?  again.");
    assert!(stem.ends_with(" Bug the thing again"), "{}", stem);
    assert_eq!(stem.len(), "2026-08-01 12.00 ".len() + "Bug the thing again".len());
    assert_eq!(export::filter_script_flag("ffmpeg version 7.1.1-full_build-www.gyan.dev"), "-/filter_complex");
    assert_eq!(export::filter_script_flag("ffmpeg version n6.1 Copyright"), "-filter_complex_script");
    assert_eq!(export::filter_script_flag("ffmpeg version N-112233-gabc"), "-/filter_complex");
}

// ---------------------------------------------------------------------------
// One real export
// ---------------------------------------------------------------------------

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg is needed for this test");
    assert!(out.status.success(), "ffmpeg {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
}

fn probe_ms(p: &std::path::Path) -> i64 {
    let ffprobe = session::find_ffprobe().expect("ffprobe is needed for this test");
    session::probe_segment(&ffprobe, p).unwrap().duration_ms
}

/// A 10 s session: screen-0 as two 5 s segments with a 1 s hole between them
/// (4–5 s), a webcam and a mic over the whole span, and 2 s (6–8 s) cut from
/// every track.
fn real_session() -> (PathBuf, String, i64) {
    let id = "2026-01-04_09-00-00".to_string();
    let base = session::epoch_from_session_id(&id).unwrap();
    let rec = std::env::temp_dir().join(format!("chirptime-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&rec);
    let dir = rec.join("sessions").join(&id);
    let mk = |sub: &str, name: &str, src: &str, secs: &str, codec: &[&str]| -> PathBuf {
        let d = dir.join(sub);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(name);
        let mut a = vec!["-f", "lavfi", "-i", src, "-t", secs];
        a.extend_from_slice(codec);
        let ps = p.to_string_lossy().to_string();
        a.push(&ps);
        ffmpeg(&a);
        p
    };
    let x264 = ["-c:v", "libx264", "-preset", "ultrafast", "-g", "30", "-pix_fmt", "yuv420p"];
    mk("screen-0", "a.mp4", "testsrc2=size=320x180:rate=30", "4", &x264);
    mk("screen-0", "b.mp4", "testsrc2=size=320x180:rate=30", "5", &x264);
    mk("webcam", "w.mp4", "testsrc=size=160x90:rate=30", "10", &x264);
    mk("mic", "m.mp4", "sine=frequency=440:sample_rate=48000", "10", &["-c:a", "aac"]);

    let mut m = SessionManifest::new(id.clone(), base, 5, Rect { x: 0, y: 0, w: 320, h: 180 });
    let mut s0 = track("screen-0", "screen", 320, 180, mon(0, 0, 320, 180), vec![seg("a.mp4", base, 4_000), seg("b.mp4", base + 5_000, 5_000)]);
    s0.gaps.push(Gap { from_epoch_ms: base + 4_000, to_epoch_ms: base + 5_000, reason: "display went dark".into() });
    m.tracks.push(s0);
    m.tracks.push(track("webcam", "webcam", 160, 90, None, vec![seg("w.mp4", base, 10_000)]));
    m.tracks.push(track("mic", "mic", 0, 0, None, vec![seg("m.mp4", base, 10_000)]));
    m.cuts.push(Cut {
        id: "cut-1".into(),
        from_epoch_ms: base + 6_000,
        to_epoch_ms: base + 8_000,
        tracks: None,
        created_epoch_ms: base + 60_000,
        state: CutState::Final,
        freed_bytes: 0,
    });
    m.ended_epoch_ms = Some(base + 10_000);
    session::write_manifest(&dir, &m).unwrap();
    (rec, id, base)
}

#[test]
fn a_real_export_composes_lists_and_deletes() {
    let (rec, id, base) = real_session();
    let r = ExportRequest {
        session_id: id.clone(),
        from_ms: base,
        to_ms: base + 10_000,
        size: SizeMode::Original,
        title: Some("Real test".into()),
        ..req(Layout::Devlog)
    };
    let ctl = JobCtl::default();
    let mut last = 0.0;
    let out = export::run_export("ffmpeg", &rec, "exp-1", &r, &[], &EncoderCaps::default(), &ctl, &mut |f| last = f)
        .unwrap_or_else(|e| panic!("export failed: {:?}", e));
    assert_eq!(last, 1.0);
    let file = rec.join(&out.path);
    assert!(out.path.starts_with("exports/") && out.path.ends_with(" Real test.mp4"), "{}", out.path);
    // 10 s minus the 2 s cut.
    let ms = probe_ms(&file);
    assert!((ms - 8_000).abs() <= 150, "duration {}", ms);
    assert_eq!(out.duration_ms, 8_000);
    let thumb = out.thumb.clone().expect("a first-frame thumbnail");
    assert!(rec.join(&thumb).is_file());
    // The work folder is gone, no partial is left, and the index has it.
    assert!(!rec.join("exports").join(".work").join("exp-1").exists());
    assert!(std::fs::read_dir(rec.join("exports")).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
    assert_eq!(export::list(&rec), vec![out.clone()]);

    // Raw tracks: a folder with one copy per track.
    let raw = ExportRequest { raw_tracks: Some(vec!["screen-0".into(), "mic".into()]), title: None, ..r.clone() };
    let rr = export::run_export("ffmpeg", &rec, "exp-2", &raw, &[], &EncoderCaps::default(), &ctl, &mut |_| {})
        .unwrap_or_else(|e| panic!("raw export failed: {:?}", e));
    let folder = rec.join(&rr.path);
    assert!(folder.join("screen-0.mp4").is_file() && folder.join("mic.mp4").is_file());
    assert_eq!(export::list(&rec).len(), 2);

    // A cancelled job stops and leaves nothing behind.
    let cancelled = JobCtl::default();
    cancelled.cancel();
    let e = export::run_export("ffmpeg", &rec, "exp-3", &r, &[], &EncoderCaps::default(), &cancelled, &mut |_| {});
    assert_eq!(e, Err(RunError::Cancelled));
    assert_eq!(export::list(&rec).len(), 2);

    export::delete(&rec, &out.id).unwrap();
    export::delete(&rec, &rr.id).unwrap();
    assert!(!file.exists() && !rec.join(&thumb).exists() && !folder.exists());
    assert!(export::list(&rec).is_empty());
    assert!(export::delete(&rec, &out.id).is_err());
    let _ = std::fs::remove_dir_all(&rec);
}
