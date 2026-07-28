//! First-run FFmpeg install.
//!
//! ChirpTime drives FFmpeg for every capture but does not ship it: the app is
//! MIT and every usable Windows FFmpeg build is GPL (`capture.rs` encodes with
//! `libx264` unconditionally, so an LGPL build cannot run this app at all).
//! Invoking a separate process is not linking, so fetching the binary at the
//! user's request — rather than bundling it in the installer — is what keeps
//! ChirpTime's own distribution permissively licensed. See decisions.md.
//!
//! **Nothing here runs on its own.** The user is shown what will be downloaded,
//! how big it is and why, and presses a button. This module is only ever reached
//! from `commands::install_ffmpeg`.
//!
//! ## The integrity rule
//!
//! We are downloading an executable and then running it, so the archive is
//! verified against a **pinned SHA-256** before anything is extracted, and the
//! extracted binaries are executed once as a smoke test before they are moved
//! into place. A hash mismatch deletes the download and fails — there is no
//! path through this module that runs an unverified binary.
//!
//! ## Never a half-installed directory
//!
//! Everything lands in a staging directory. [`BIN_SUBDIR`] only ever comes into
//! existence by renaming a staging directory that has already been hashed,
//! extracted, and smoke-tested. An interrupted install leaves debris with a
//! leading dot, which the next attempt clears — it never leaves something that
//! looks like a working FFmpeg.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// The pin
// ---------------------------------------------------------------------------

/// gyan.dev's **essentials** build, mirrored as an immutable GitHub release
/// asset. Essentials rather than "full" because it is 110 MB instead of 180 MB
/// and — verified against this exact archive on 2026-07-27 — still carries
/// every encoder, filter and input device ChirpTime asks for: `libx264`,
/// `h264_nvenc` / `av1_nvenc` / `hevc_nvenc`, `h264_qsv` (via libvpl), `aac`,
/// the `drawtext` filter (via libfreetype/fontconfig/harfbuzz/fribidi, which is
/// what burns the timestamps in), `overlay`, `scale`, and the `gdigrab`,
/// `ddagrab` and `dshow` inputs.
pub const FFMPEG_VERSION: &str = "8.1.2";

pub const FFMPEG_URL: &str =
    "https://github.com/GyanD/codexffmpeg/releases/download/8.1.2/ffmpeg-8.1.2-essentials_build.zip";

/// SHA-256 of the archive at [`FFMPEG_URL`].
///
/// Cross-checked on 2026-07-27 against **two independent hosts**: computed from
/// the GitHub release asset above, and identical to the value gyan.dev publishes
/// at `https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256`
/// (that URL tracks whatever the current release is, so it will stop matching
/// once gyan ships a newer build — it is a check that was made, not one that can
/// be repeated later against a moving target).
///
/// **To update this pin:** download the new versioned asset, confirm its hash
/// against gyan.dev's published `.sha256` *while it is still the current
/// release*, and change [`FFMPEG_URL`], this constant and [`FFMPEG_ZIP_BYTES`]
/// together.
pub const FFMPEG_SHA256: &str =
    "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec";

/// Exact byte length of the archive. Only used to render a percentage before
/// the server's `Content-Length` is trusted, and to tell the user the size
/// before they agree to it. The hash, not this, is what decides validity.
pub const FFMPEG_ZIP_BYTES: u64 = 109_728_040;

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Under `%APPDATA%\com.chirptime.app\`, beside `settings.json`. Per-user, so
/// the install needs no elevation — which is the whole reason it is not in
/// Program Files.
const INSTALL_SUBDIR: &str = "ffmpeg";
const BIN_SUBDIR: &str = "bin";
const STAGING_SUBDIR: &str = ".staging";
const PART_FILE: &str = ".download.part";
const RETIRED_SUBDIR: &str = ".retired";

pub fn install_dir() -> PathBuf {
    crate::settings::settings_dir().join(INSTALL_SUBDIR)
}

/// The directory `find_ffmpeg` / `find_ffprobe` add to their candidate list.
pub fn bin_dir() -> PathBuf {
    install_dir().join(BIN_SUBDIR)
}

/// Path to a managed binary, or `None` if this install is not present. Only ever
/// reports a path that exists — callers treat `Some` as "usable".
pub fn managed(exe: &str) -> Option<String> {
    let p = bin_dir().join(exe);
    if p.is_file() {
        Some(p.to_string_lossy().to_string())
    } else {
        None
    }
}

/// Both halves present. `ffprobe` matters as much as `ffmpeg`: the session
/// recorder probes every closed segment to build the manifest, so an install
/// with only `ffmpeg.exe` would record and then fail to index.
pub fn is_installed() -> bool {
    managed("ffmpeg.exe").is_some() && managed("ffprobe.exe").is_some()
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct Progress {
    /// `downloading` | `verifying` | `extracting` | `checking` | `done`
    pub phase: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

/// One progress event per megabyte. Often enough that a stalled download is
/// visibly stalled, rare enough that a 110 MB transfer is ~110 IPC messages.
const PROGRESS_STEP_BYTES: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

/// Download, verify, extract and smoke-test FFmpeg. Blocking — call it from
/// `spawn_blocking`.
///
/// Every failure leaves the previous state intact: either a working install
/// stays working, or nothing was there and nothing is there now. `on_progress`
/// is called from this thread.
pub fn install(mut on_progress: impl FnMut(Progress)) -> Result<PathBuf, String> {
    let root = install_dir();
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("Could not create {}: {e}", root.display()))?;

    // Debris from an interrupted previous attempt. Clearing it up front (rather
    // than only on the way out) means a hard-killed app self-heals.
    let staging = root.join(STAGING_SUBDIR);
    let part = root.join(PART_FILE);
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&part);

    let result = (|| -> Result<PathBuf, String> {
        download_verified(FFMPEG_URL, FFMPEG_SHA256, &part, &mut on_progress)?;

        on_progress(Progress {
            phase: "extracting",
            downloaded_bytes: FFMPEG_ZIP_BYTES,
            total_bytes: FFMPEG_ZIP_BYTES,
        });
        std::fs::create_dir_all(&staging)
            .map_err(|e| format!("Could not create {}: {e}", staging.display()))?;
        extract_binaries(&part, &staging)?;

        // The zip is ~110 MB and we keep ~200 MB of binaries; dropping it here
        // rather than at the end halves the peak disk requirement.
        let _ = std::fs::remove_file(&part);

        on_progress(Progress {
            phase: "checking",
            downloaded_bytes: FFMPEG_ZIP_BYTES,
            total_bytes: FFMPEG_ZIP_BYTES,
        });
        // Running them is the only check that catches an antivirus that let the
        // write succeed and then quarantined or gutted the file.
        for exe in ["ffmpeg.exe", "ffprobe.exe"] {
            smoke_test(&staging.join(exe))?;
        }

        promote(&root, &staging)?;

        // Belt and braces: the swap moved files, so prove the final location
        // still runs before telling anyone it worked.
        for exe in ["ffmpeg.exe", "ffprobe.exe"] {
            smoke_test(&bin_dir().join(exe))?;
        }

        on_progress(Progress {
            phase: "done",
            downloaded_bytes: FFMPEG_ZIP_BYTES,
            total_bytes: FFMPEG_ZIP_BYTES,
        });
        Ok(bin_dir())
    })();

    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Stream `url` to `dest`, hashing as it goes, and delete it unless the hash
/// matches `expected`. The file is hashed *while* being written rather than
/// re-read afterwards, so a 110 MB download is not read twice and there is no
/// window in which an unverified file sits complete on disk under a name
/// anything would use.
///
/// `url` and `expected` are parameters rather than reads of the constants purely
/// so the mismatch path can be tested against something small — [`install`] is
/// the only caller and always passes the pin.
pub(crate) fn download_verified(
    url: &str,
    expected: &str,
    dest: &Path,
    on_progress: &mut impl FnMut(Progress),
) -> Result<(), String> {
    on_progress(Progress {
        phase: "downloading",
        downloaded_bytes: 0,
        total_bytes: FFMPEG_ZIP_BYTES,
    });

    let agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .new_agent();

    let mut response = agent.get(url).call().map_err(|e| {
        format!(
            "Could not reach the download ({e}). Check the internet connection \
             and try again, or install FFmpeg yourself and point ChirpTime at it."
        )
    })?;

    if response.status() != 200 {
        return Err(format!(
            "The download server answered {} instead of 200. Try again later, or \
             install FFmpeg yourself and point ChirpTime at it.",
            response.status()
        ));
    }

    // Advisory only — a proxy may not send one, and it is never trusted over the
    // hash. It exists so the progress bar is honest if the size ever changes.
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(FFMPEG_ZIP_BYTES);

    let mut reader = response.body_mut().as_reader();
    let file = std::fs::File::create(dest)
        .map_err(|e| format!("Could not write to {}: {e}", dest.display()))?;
    let mut writer = std::io::BufWriter::new(file);

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 128 * 1024];
    let mut done: u64 = 0;
    let mut last_reported: u64 = 0;

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("The download was interrupted after {done} bytes: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        // A full disk surfaces here, and says so.
        writer.write_all(&buf[..n]).map_err(|e| {
            format!(
                "Could not write the download to {} — is the drive full? ({e})",
                dest.display()
            )
        })?;
        done += n as u64;
        if done - last_reported >= PROGRESS_STEP_BYTES {
            last_reported = done;
            on_progress(Progress {
                phase: "downloading",
                downloaded_bytes: done,
                total_bytes: total,
            });
        }
    }
    writer
        .flush()
        .map_err(|e| format!("Could not finish writing {}: {e}", dest.display()))?;
    drop(writer);

    on_progress(Progress {
        phase: "verifying",
        downloaded_bytes: done,
        total_bytes: total,
    });

    let got = hex_lower(&hasher.finalize());
    if got != expected {
        let _ = std::fs::remove_file(dest);
        return Err(format!(
            "The downloaded file does not match its expected checksum, so it was \
             deleted and nothing was run. Expected {expected}, got {got}. This \
             usually means the download was corrupted or something on the network \
             altered it."
        ));
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Pull just `bin/ffmpeg.exe` and `bin/ffprobe.exe` out of the archive.
///
/// The archive also holds `ffplay.exe` (another ~100 MB, which this app never
/// runs), docs and presets; taking only the two binaries turns a 304 MB extract
/// into ~200 MB. Destination names are constructed here rather than taken from
/// the archive, so a crafted entry name cannot escape the staging directory.
fn extract_binaries(zip_path: &Path, staging: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("Could not open the download: {e}"))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("The download is not a readable zip archive: {e}"))?;

    for wanted in ["ffmpeg.exe", "ffprobe.exe"] {
        // gyan's archives nest everything under a versioned top-level folder,
        // so the entry is matched by suffix rather than by an exact path that
        // would break on the next version bump.
        let suffix = format!("bin/{wanted}");
        let index = (0..archive.len()).find(|i| {
            archive
                .by_index_raw(*i)
                .map(|e| e.name().replace('\\', "/").ends_with(&suffix))
                .unwrap_or(false)
        });
        let Some(index) = index else {
            return Err(format!(
                "The archive does not contain {wanted}. The pinned download may \
                 have changed layout; nothing was installed."
            ));
        };

        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Could not read {wanted} from the archive: {e}"))?;
        let dest = staging.join(wanted);
        let mut out = std::fs::File::create(&dest)
            .map_err(|e| format!("Could not write {}: {e}", dest.display()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| {
            format!(
                "Could not extract {wanted} — is the drive full, or did antivirus \
                 block it? ({e})"
            )
        })?;
    }
    Ok(())
}

/// Run a binary once and require a clean exit. A file that exists but will not
/// execute is worse than one that is missing, because everything downstream
/// would report a capture failure instead of a setup problem.
fn smoke_test(exe: &Path) -> Result<(), String> {
    if !exe.is_file() {
        return Err(format!("{} is missing after install.", exe.display()));
    }
    let out = crate::recorder::silent_command(&exe.to_string_lossy())
        .arg("-version")
        .output()
        .map_err(|e| {
            format!(
                "{} was downloaded but will not run ({e}). Antivirus quarantining \
                 it is the usual cause.",
                exe.display()
            )
        })?;
    if !out.status.success() {
        return Err(format!(
            "{} was downloaded but exited with {} when asked for its version.",
            exe.display(),
            out.status
        ));
    }
    Ok(())
}

/// Swap a verified staging directory into place as `bin/`.
///
/// The old directory is renamed aside first and only then deleted, so the
/// window in which neither exists is a single rename rather than a recursive
/// delete. If the rename into place fails, the previous install is put back.
fn promote(root: &Path, staging: &Path) -> Result<(), String> {
    let bin = root.join(BIN_SUBDIR);
    let retired = root.join(RETIRED_SUBDIR);
    let _ = std::fs::remove_dir_all(&retired);

    let had_previous = bin.exists();
    if had_previous {
        std::fs::rename(&bin, &retired).map_err(|e| {
            format!(
                "Could not replace the existing FFmpeg at {} ({e}). It is probably \
                 in use — stop any recording and try again.",
                bin.display()
            )
        })?;
    }

    if let Err(e) = std::fs::rename(staging, &bin) {
        if had_previous {
            let _ = std::fs::rename(&retired, &bin);
        }
        return Err(format!(
            "Could not move the verified FFmpeg into {} ({e}).",
            bin.display()
        ));
    }

    let _ = std::fs::remove_dir_all(&retired);
    Ok(())
}

/// Delete the managed install. Exists so a user who suspects a bad install can
/// start over rather than being stuck with one this app insists is fine.
pub fn uninstall() -> Result<(), String> {
    let root = install_dir();
    if !root.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&root)
        .map_err(|e| format!("Could not remove {} ({e}) — is a recording running?", root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pin_is_a_lowercase_sha256_and_the_url_is_https() {
        assert_eq!(FFMPEG_SHA256.len(), 64);
        assert!(FFMPEG_SHA256
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        // A plain-HTTP download would make the hash the only defence against an
        // active attacker; it must be both.
        assert!(FFMPEG_URL.starts_with("https://"));
        assert!(FFMPEG_URL.contains(FFMPEG_VERSION));
    }

    #[test]
    fn hex_is_lowercase_and_zero_padded() {
        assert_eq!(hex_lower(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    /// The integrity gate is the whole reason this module is allowed to exist,
    /// so prove it rejects rather than assuming it does. Points at a 65-byte
    /// file on gyan.dev (the same host the pin is cross-checked against) with a
    /// deliberately wrong hash, so the assertion costs nothing to run.
    ///
    /// `#[ignore]`: it needs the network.
    #[test]
    #[ignore = "needs network"]
    fn a_hash_mismatch_fails_and_deletes_the_download() {
        let dest = std::env::temp_dir().join("chirptime-hash-mismatch-test.part");
        let _ = std::fs::remove_file(&dest);
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";

        let err = download_verified(
            "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256",
            wrong,
            &dest,
            &mut |_| {},
        )
        .expect_err("a wrong hash must not be accepted");

        assert!(err.contains("does not match its expected checksum"), "{err}");
        assert!(
            !dest.exists(),
            "the unverified download must be deleted, not left on disk"
        );
    }

    #[test]
    fn the_binaries_live_beside_the_settings_file() {
        let bin = bin_dir();
        assert!(bin.starts_with(crate::settings::settings_dir()));
        assert!(bin.ends_with("ffmpeg/bin") || bin.ends_with(r"ffmpeg\bin"));
    }
}
