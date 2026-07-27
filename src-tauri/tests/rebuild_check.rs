//! Read-only audit of the crash-recovery path against whatever sessions are
//! already on disk.
//!
//! Runs `rebuild_manifest_from_disk` over every session directory under
//! `output/sessions` and checks the reconstruction against the manifest that was
//! written live. It **records nothing, opens no capture device, and writes no
//! file** — it only reads directories and ffprobes existing segments.
//!
//!     cargo test --test rebuild_check -- --ignored --nocapture
//!
//! `#[ignore]`d because it depends on this machine's capture archive.

use chirptime_lib::session::{self, SessionManifest};
use std::path::{Path, PathBuf};

fn sessions_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("output")
        .join("sessions")
}

/// Segments the rebuild found, as `(track, file)` pairs.
fn segment_keys(m: &SessionManifest) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = m
        .tracks
        .iter()
        .flat_map(|t| t.segments.iter().map(|s| (t.id.clone(), s.file.clone())))
        .collect();
    v.sort();
    v
}

#[test]
#[ignore]
fn rebuild_matches_the_live_manifest_for_every_session_on_disk() {
    let root = sessions_root();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{}: {}", root.display(), e))
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no sessions on disk to check");

    let mut checked = 0;
    for dir in &dirs {
        let id = dir.file_name().unwrap().to_string_lossy().to_string();
        let live = session::read_manifest(dir).ok();
        let rebuilt = session::rebuild_manifest_from_disk(dir)
            .unwrap_or_else(|e| panic!("{}: rebuild failed: {}", id, e));

        assert!(rebuilt.rebuilt, "{}: must be flagged as reconstructed", id);
        assert_eq!(rebuilt.id, id);
        assert_eq!(
            rebuilt.started_epoch_ms,
            session::epoch_from_session_id(&id).unwrap(),
            "{}: start must come from the directory name",
            id
        );

        // Every segment in the rebuild must be a file that really exists, and
        // every duration must be one ffprobe actually returned.
        for t in &rebuilt.tracks {
            for s in &t.segments {
                let p = dir.join(&t.dir).join(&s.file);
                assert!(p.is_file(), "{}: {} does not exist", id, p.display());
                assert!(s.duration_ms > 0, "{}: {} has no duration", id, s.file);
                assert!(s.bytes > 0, "{}: {} is empty", id, s.file);
            }
            // Segments must be in order and must not overlap.
            for w in t.segments.windows(2) {
                assert!(
                    w[1].start_epoch_ms >= w[0].start_epoch_ms + w[0].duration_ms - 1,
                    "{}/{}: {} overlaps {}",
                    id,
                    t.id,
                    w[1].file,
                    w[0].file
                );
            }
        }

        println!(
            "{:<22} tracks {} segments {:>3} bytes {:>10}  {}",
            id,
            rebuilt.tracks.len(),
            rebuilt.segment_count(),
            rebuilt.total_bytes(),
            match &live {
                Some(l) if l.ended_epoch_ms.is_none() => "(live manifest unfinalized)",
                Some(_) => "",
                None => "(no session.json)",
            }
        );

        if let Some(live) = live {
            let (a, b) = (segment_keys(&live), segment_keys(&rebuilt));
            // The rebuild may legitimately find MORE than the live manifest: a
            // force-killed session never got to index its final in-flight
            // segment. It must never find fewer.
            for k in &a {
                assert!(
                    b.contains(k),
                    "{}: rebuild lost {}/{} that the live manifest had",
                    id,
                    k.0,
                    k.1
                );
            }
            let extra: Vec<_> = b.iter().filter(|k| !a.contains(k)).collect();
            if !extra.is_empty() {
                println!("    recovered {} segment(s) the live manifest never indexed: {:?}", extra.len(), extra);
            }

            // Start times must agree closely for the segments both know about.
            for lt in &live.tracks {
                let Some(rt) = rebuilt.tracks.iter().find(|t| t.id == lt.id) else {
                    panic!("{}: rebuild dropped track {}", id, lt.id);
                };
                for ls in &lt.segments {
                    let Some(rs) = rt.segments.iter().find(|s| s.file == ls.file) else {
                        continue;
                    };
                    let d = (rs.start_epoch_ms - ls.start_epoch_ms).abs();
                    assert!(
                        d <= 5_000,
                        "{}/{}/{}: rebuilt start differs by {}ms",
                        id,
                        lt.id,
                        ls.file,
                        d
                    );
                    assert_eq!(rs.duration_ms, ls.duration_ms, "{}/{}/{}", id, lt.id, ls.file);
                }
            }
        }
        checked += 1;
    }
    println!("checked {} sessions", checked);
}
