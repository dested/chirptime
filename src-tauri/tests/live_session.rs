//! Live, hardware-touching verification for the session recorder.
//!
//! These are NOT unit tests — they drive real ffmpeg processes against the real
//! displays and write real files. They are `#[ignore]`d so `cargo test` stays
//! clean, and are run explicitly:
//!
//!   cargo test --release --test live_session -- --ignored --nocapture --test-threads=1
//!
//! Individually:
//!   ... -- --ignored --nocapture outputs
//!   ... -- --ignored --nocapture three_minute_session
//!   ... -- --ignored --nocapture kill_test
//!   ... -- --ignored --nocapture rebuild_from_disk

use chirptime_lib::recorder::{self, Recorder, RecorderEnv, SessionOptions};
use chirptime_lib::session;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn scratch() -> PathBuf {
    let p = PathBuf::from(r"G:\code\screener\output\sessions");
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
#[ignore]
fn outputs() {
    let outs = recorder::enumerate_dxgi_outputs().expect("dxgi");
    println!("\n=== DXGI outputs ===");
    for o in &outs {
        println!(
            "adapter {} ({}) output_idx {} : {} {}x{} @ ({},{}) attached={} capturable={}",
            o.adapter,
            o.adapter_name,
            o.output_idx,
            o.device_name,
            o.w,
            o.h,
            o.x,
            o.y,
            o.attached,
            o.capturable
        );
    }
    let env = RecorderEnv::from_dxgi();
    println!("\n=== resolved screen targets ===");
    for t in &env.screens {
        println!(
            "output_idx {} adapter {} -> {} rect {}x{} @ ({},{})",
            t.output_idx, t.adapter, t.label, t.rect.w, t.rect.h, t.rect.x, t.rect.y
        );
    }
    println!("desktop = {:?}", env.desktop);
    println!("\n=== warnings ===");
    for w in &env.warnings {
        println!("- {}", w);
    }
    let caps = recorder::detect_encoders("ffmpeg");
    println!("\nencoders: {:?}", caps);
    for t in &env.screens {
        let (enc, codec, warn) = recorder::pick_video_encoder(t.rect.w, &caps);
        println!(
            "  output_idx {} ({}px) -> {} / {} {:?}",
            t.output_idx, t.rect.w, enc, codec, warn
        );
    }
}

/// Memory of every ffmpeg the recorder owns, plus the count, via tasklist.
fn ffmpeg_mem_kb() -> (usize, u64) {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq ffmpeg.exe", "/FO", "CSV", "/NH"])
        .output();
    let Ok(out) = out else { return (0, 0) };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut n = 0usize;
    let mut total = 0u64;
    for line in text.lines() {
        let cols: Vec<&str> = line.split("\",\"").collect();
        if cols.len() < 5 {
            continue;
        }
        n += 1;
        let mem = cols[4].trim_matches(['"', ' ']).replace(" K", "").replace(',', "");
        total += mem.trim().parse::<u64>().unwrap_or(0);
    }
    (n, total)
}

/// RSS of this test process — i.e. the recorder itself, its supervisor threads,
/// and the manifest. This is the number that must be flat; ffmpeg's own RSS
/// varies with scene complexity.
fn own_rss_kb() -> u64 {
    let pid = std::process::id();
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output();
    let Ok(out) = out else { return 0 };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let cols: Vec<&str> = line.split("\",\"").collect();
        if cols.len() >= 5 {
            return cols[4]
                .trim_matches(['"', ' '])
                .replace(" K", "")
                .replace(',', "")
                .trim()
                .parse()
                .unwrap_or(0);
        }
    }
    0
}

fn opts(segment_seconds: u32) -> SessionOptions {
    SessionOptions {
        segment_seconds,
        record_webcam: false, // the BRIO is frequently held by another app; see report
        record_mic: true,
        record_system_audio: true,
        ..Default::default()
    }
}

fn audit(dir: &Path, m: &session::SessionManifest) -> bool {
    let ffprobe = session::find_ffprobe().unwrap();
    let mut ok = true;
    println!("\n=== manifest audit: {} ===", m.id);
    println!(
        "started={} ended={:?} segment_seconds={} desktop={:?}",
        m.started_epoch_ms, m.ended_epoch_ms, m.segment_seconds, m.desktop
    );
    for t in &m.tracks {
        let bytes = t.total_bytes();
        println!(
            "\n-- {} [{}] {} {}x{}@{} {:?} restarts={} segments={} gaps={} bytes={:.1}MB",
            t.id,
            t.kind,
            t.codec,
            t.width,
            t.height,
            t.fps,
            t.status,
            t.restarts,
            t.segments.len(),
            t.gaps.len(),
            bytes as f64 / 1e6
        );
        if let Some(e) = &t.error {
            println!("   error: {}", e);
        }
        for g in &t.gaps {
            println!(
                "   GAP {}ms: {}",
                g.to_epoch_ms - g.from_epoch_ms,
                g.reason
            );
        }
        if t.segments.is_empty() && t.status != session::TrackStatus::Failed {
            println!("   !! no segments");
            ok = false;
        }
        // Independently re-probe EVERY segment on disk. The manifest claiming a
        // duration is not evidence the file is playable.
        let tdir = dir.join(&t.dir);
        let mut prev_end: Option<i64> = None;
        for (i, s) in t.segments.iter().enumerate() {
            let p = tdir.join(&s.file);
            match session::probe_segment(&ffprobe, &p) {
                Ok(pr) => {
                    let delta = (pr.duration_ms - s.duration_ms).abs();
                    let contiguous = prev_end.map(|e| (s.start_epoch_ms - e).abs()).unwrap_or(0);
                    let flag = if delta > 50 || contiguous > 100 { " <<<" } else { "" };
                    println!(
                        "   [{:>3}] {} dur={}ms (probe {}ms) bytes={} joint={}ms{}",
                        i, s.file, s.duration_ms, pr.duration_ms, s.bytes, contiguous, flag
                    );
                    if delta > 50 {
                        ok = false;
                    }
                }
                Err(e) => {
                    println!("   [{:>3}] {} CORRUPT: {}", i, s.file, e);
                    ok = false;
                }
            }
            if s.bytes == 0 {
                println!("   [{:>3}] {} ZERO BYTES", i, s.file);
                ok = false;
            }
            prev_end = Some(s.start_epoch_ms + s.duration_ms);
        }
        // Any file on disk the manifest never indexed is a hole in the index.
        if let Ok(rd) = std::fs::read_dir(&tdir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".mp4") && !t.segments.iter().any(|s| s.file == n) {
                    println!("   !! on disk but NOT in manifest: {}", n);
                    ok = false;
                }
            }
        }
    }
    // Cross-track alignment at t=0.
    let starts: Vec<(String, i64)> = m
        .tracks
        .iter()
        .filter(|t| !t.segments.is_empty())
        .map(|t| (t.id.clone(), t.start_epoch_ms))
        .collect();
    if let (Some(min), Some(max)) = (
        starts.iter().map(|s| s.1).min(),
        starts.iter().map(|s| s.1).max(),
    ) {
        println!("\ncross-track start spread: {}ms", max - min);
        for (id, s) in &starts {
            println!("   {:<12} +{}ms", id, s - min);
        }
    }
    ok
}

#[test]
#[ignore]
fn three_minute_session() {
    let root = scratch();
    let env = RecorderEnv::from_dxgi();
    println!("screens: {}", env.screens.len());
    assert!(!env.screens.is_empty(), "no capturable screens");

    let (n0, m0) = ffmpeg_mem_kb();
    println!("ffmpeg before: {} procs, {} KB", n0, m0);

    let secs: u64 = std::env::var("RUN_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let seg: u32 = std::env::var("SEG_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let rec = Recorder::start(env, &root, opts(seg)).expect("start");
    let dir = rec.session_dir();
    println!("session dir: {}  (run {}s, {}s segments)", dir.display(), secs, seg);

    let start = Instant::now();
    let mut samples: Vec<(u64, usize, u64, u64)> = Vec::new();
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_secs(15));
        let (n, mem) = ffmpeg_mem_kb();
        let own = own_rss_kb();
        let m = rec.manifest();
        samples.push((start.elapsed().as_secs(), n, mem, own));
        println!(
            "t={:>4}s  ffmpeg={} procs {:.0}MB  recorder={:.1}MB  segments={} bytes={:.1}MB",
            start.elapsed().as_secs(),
            n,
            mem as f64 / 1024.0,
            own as f64 / 1024.0,
            m.segment_count(),
            m.total_bytes() as f64 / 1e6
        );
        if let Some(f) = rec.fatal() {
            panic!("fatal during run: {}", f);
        }
    }

    println!("\nstopping...");
    let t = Instant::now();
    let m = rec.stop();
    println!("stop took {:?}", t.elapsed());

    println!("\nmemory samples (elapsed_s, ffmpeg_procs, ffmpeg_rss_KB, recorder_rss_KB):");
    for s in &samples {
        println!("  {:>4}s  {}  {}  {}", s.0, s.1, s.2, s.3);
    }
    if let (Some(first), Some(last)) = (samples.first(), samples.last()) {
        let growth = last.3 as i64 - first.3 as i64;
        println!(
            "recorder RSS: {} KB -> {} KB over {}s ({:+} KB)",
            first.3, last.3, last.0, growth
        );
    }
    println!("\nwarnings:");
    for w in rec.warnings() {
        println!("  - {}", w);
    }

    let ok = audit(&dir, &m);
    // The manifest on disk must equal the in-memory one.
    let on_disk = session::read_manifest(&dir).expect("session.json");
    assert_eq!(on_disk.segment_count(), m.segment_count());
    assert!(on_disk.ended_epoch_ms.is_some(), "session not finalized");
    assert!(ok, "audit found corrupt/zero/unindexed segments");
}

#[test]
#[ignore]
fn kill_test() {
    let root = scratch();
    let rec = Recorder::start(RecorderEnv::from_dxgi(), &root, opts(15)).expect("start");
    let dir = rec.session_dir();
    println!("session dir: {}", dir.display());

    std::thread::sleep(Duration::from_secs(40));
    println!("segments before kill: {}", rec.manifest().segment_count());

    println!("--- hard-killing every ffmpeg ---");
    let out = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "ffmpeg.exe"])
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&out.stdout));

    // Watchdog needs to notice the exit, record a gap, and relaunch.
    std::thread::sleep(Duration::from_secs(50));
    let m = rec.manifest();
    let restarted: Vec<_> = m.tracks.iter().filter(|t| t.restarts > 0).collect();
    println!("tracks restarted: {}", restarted.len());
    for t in &restarted {
        println!("  {} restarts={} gaps={}", t.id, t.restarts, t.gaps.len());
        for g in &t.gaps {
            println!("    gap {}ms: {}", g.to_epoch_ms - g.from_epoch_ms, g.reason);
        }
    }

    let m = rec.stop();
    audit(&dir, &m);

    let killable: Vec<_> = m.tracks.iter().filter(|t| t.kind == "screen").collect();
    assert!(
        killable.iter().all(|t| t.restarts > 0),
        "screen tracks did not restart after being killed"
    );
    assert!(
        killable.iter().all(|t| !t.gaps.is_empty()),
        "no gap was recorded for the kill"
    );
    // The whole point: the session survived and kept producing after the kill.
    for t in &killable {
        let after = t
            .segments
            .iter()
            .filter(|s| s.start_epoch_ms > t.gaps[0].to_epoch_ms)
            .count();
        println!("{}: {} segments after the gap", t.id, after);
        assert!(after > 0, "{} produced nothing after restart", t.id);
    }
}

/// Rebuild a manifest with `session.json` deliberately removed — the crash path.
#[test]
#[ignore]
fn rebuild_from_disk() {
    let root = scratch();
    // Use the most recent session that has segments.
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let dir = dirs.last().expect("no sessions to rebuild").clone();
    println!("rebuilding {}", dir.display());

    let original = session::read_manifest(&dir).ok();

    // Simulate the crash: move session.json out of the way entirely.
    let mani = dir.join("session.json");
    let stashed = dir.join("session.json.stashed");
    if mani.exists() {
        std::fs::rename(&mani, &stashed).unwrap();
    }
    assert!(!mani.exists());

    let rebuilt = session::rebuild_manifest_from_disk(&dir).expect("rebuild");
    println!(
        "rebuilt: id={} tracks={} segments={} rebuilt_flag={}",
        rebuilt.id,
        rebuilt.tracks.len(),
        rebuilt.segment_count(),
        rebuilt.rebuilt
    );
    for t in &rebuilt.tracks {
        println!(
            "  {} [{}] {} {}x{}@{} segments={} start={}",
            t.id, t.kind, t.codec, t.width, t.height, t.fps, t.segments.len(), t.start_epoch_ms
        );
    }

    if let Some(o) = &original {
        println!("\noriginal segments={} rebuilt segments={}", o.segment_count(), rebuilt.segment_count());
        assert_eq!(
            o.segment_count(),
            rebuilt.segment_count(),
            "rebuild lost segments"
        );
        for ot in &o.tracks {
            let Some(rt) = rebuilt.tracks.iter().find(|t| t.id == ot.id) else {
                continue;
            };
            for (a, b) in ot.segments.iter().zip(rt.segments.iter()) {
                assert_eq!(a.file, b.file);
                assert_eq!(a.duration_ms, b.duration_ms, "{} duration mismatch", a.file);
                let skew = (a.start_epoch_ms - b.start_epoch_ms).abs();
                if skew > 1000 {
                    println!("  {} start skew {}ms", a.file, skew);
                }
            }
        }
    }
    std::fs::rename(&stashed, &mani).ok();
}
