//! End-to-end exercise of the first-run FFmpeg install.
//!
//! **`#[ignore]` on purpose.** It pulls ~105 MB over the network and writes
//! ~200 MB into `%APPDATA%\com.chirptime.app\ffmpeg`, so it must never run as
//! part of `cargo test --lib` or in CI. Run it deliberately:
//!
//! ```text
//! cargo test --test ffmpeg_install -- --ignored --nocapture
//! ```
//!
//! It does NOT record anything — it downloads, hashes, extracts and asks each
//! binary for its version.

use chirptime_lib::ffmpeg_install;

#[test]
#[ignore = "downloads ~105 MB and installs into %APPDATA%"]
fn download_verify_extract_and_run() {
    let mut phases: Vec<&'static str> = Vec::new();
    let mut last_pct = 0u64;

    let bin = ffmpeg_install::install(|p| {
        if phases.last() != Some(&p.phase) {
            phases.push(p.phase);
            eprintln!("[phase] {}", p.phase);
        }
        if p.phase == "downloading" && p.total_bytes > 0 {
            let pct = p.downloaded_bytes * 100 / p.total_bytes;
            if pct >= last_pct + 10 {
                last_pct = pct;
                eprintln!("  {pct}% ({} bytes)", p.downloaded_bytes);
            }
        }
    })
    .expect("install should succeed");

    eprintln!("installed to {}", bin.display());

    // The whole point: both halves, both runnable. `is_installed` only checks
    // that the files exist, so the versions are read back separately.
    assert!(ffmpeg_install::is_installed(), "both binaries should be present");

    for exe in ["ffmpeg.exe", "ffprobe.exe"] {
        let path = ffmpeg_install::managed(exe).unwrap_or_else(|| panic!("{exe} missing"));
        let out = std::process::Command::new(&path)
            .arg("-version")
            .output()
            .unwrap_or_else(|e| panic!("{exe} would not run: {e}"));
        assert!(out.status.success(), "{exe} -version exited {}", out.status);
        let first = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        eprintln!("{exe}: {first}");
        assert!(
            first.contains(ffmpeg_install::FFMPEG_VERSION),
            "{exe} reported {first}, expected version {}",
            ffmpeg_install::FFMPEG_VERSION
        );
    }

    // Nothing may be left behind that a later run would trip over.
    let root = ffmpeg_install::install_dir();
    for debris in [".staging", ".download.part", ".retired"] {
        assert!(
            !root.join(debris).exists(),
            "{debris} should have been cleaned up"
        );
    }

    // The encoders, filters and inputs the app actually asks for. If a future
    // pin quietly moves to a build without one of these, recording breaks in a
    // way that looks like a hardware problem.
    let ffmpeg = ffmpeg_install::managed("ffmpeg.exe").unwrap();
    for (flag, needles) in [
        (
            "-encoders",
            vec!["libx264", "h264_nvenc", "av1_nvenc", "hevc_nvenc", "h264_qsv", "aac"],
        ),
        ("-filters", vec!["drawtext", "overlay", "scale", "ddagrab"]),
        ("-devices", vec!["gdigrab", "dshow"]),
    ] {
        let out = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", flag])
            .output()
            .expect("ffmpeg should run");
        let text = String::from_utf8_lossy(&out.stdout);
        for n in needles {
            assert!(text.contains(n), "{flag} does not list {n}");
        }
    }
}

/// Cheap and safe to run always: proves the pin is well-formed without touching
/// the network.
#[test]
fn the_pin_is_https_and_a_full_sha256() {
    assert!(ffmpeg_install::FFMPEG_URL.starts_with("https://"));
    assert_eq!(ffmpeg_install::FFMPEG_SHA256.len(), 64);
    assert!(ffmpeg_install::FFMPEG_ZIP_BYTES > 0);
}
