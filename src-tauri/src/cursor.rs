//! Cursor track (S3): where the mouse was while a session recorded, so the
//! Player and the export composer can follow the screen under it.
//!
//! The recorder samples `GetCursorPos` at 10 Hz while it records (never while
//! paused) and appends to `<session>/cursor/<start_epoch_ms>.bin`, one file per
//! run: a session start and every resume begin a new one.
//!
//! The format is chosen for size. After a 4-byte magic, each record is 8 bytes:
//! a `u32` ms offset from the epoch in the filename, then `x` and `y` as `i16`
//! in virtual-desktop pixels. A record is written only when the cursor moved,
//! or once a second while it holds still, so a reader always has a position
//! less than a second old. A full 8 h day of constant movement is
//! 288,000 records, 2.3 MB; a real day is a fraction of that.
//!
//! A crash can leave a torn last record; readers drop it.

use crate::session::{Cut, CutState};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The sampling interval.
pub const SAMPLE_MS: u64 = 100;
/// Write a record at least this often while the cursor holds still.
pub const KEEPALIVE_MS: i64 = 1_000;
/// Flush the file this often, bounding what a crash loses.
const FLUSH_EVERY: usize = 10;
pub const CURSOR_DIR: &str = "cursor";
const MAGIC: &[u8; 4] = b"CTC1";
const RECORD_BYTES: usize = 8;

/// One cursor position in virtual-desktop pixels (the same space as
/// `manifest.desktop` and each track's `monitor` rect), at wall-clock `t`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct CursorSample {
    pub t: i64,
    pub x: i32,
    pub y: i32,
}

/// Does `c` hide the cursor at `t`? The cursor is screen metadata, so any
/// cut that removed screen footage there removes it too: an all-track cut, or
/// one naming a screen track.
pub fn cut_hides_cursor(c: &Cut, t: i64) -> bool {
    c.state != CutState::Undone
        && t >= c.from_epoch_ms
        && t < c.to_epoch_ms
        && c
            .tracks
            .as_ref()
            .map_or(true, |ts| ts.iter().any(|id| id.starts_with("screen-")))
}

fn clamp16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Decode one run file. Pure, for the tests.
pub fn decode(base_epoch_ms: i64, bytes: &[u8]) -> Vec<CursorSample> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Vec::new();
    }
    bytes[MAGIC.len()..]
        .chunks_exact(RECORD_BYTES)
        .map(|r| CursorSample {
            t: base_epoch_ms + u32::from_le_bytes([r[0], r[1], r[2], r[3]]) as i64,
            x: i16::from_le_bytes([r[4], r[5]]) as i32,
            y: i16::from_le_bytes([r[6], r[7]]) as i32,
        })
        .collect()
}

fn run_files(session_dir: &Path) -> Vec<(i64, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(session_dir.join(CURSOR_DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<(i64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let base = name.strip_suffix(".bin")?.parse::<i64>().ok()?;
            Some((base, e.path()))
        })
        .collect();
    out.sort_by_key(|(b, _)| *b);
    out
}

/// Samples in `[from_ms, to_ms]`, ascending by `t`, excluding cut ranges.
/// Empty when the session has no cursor data.
pub fn read_track(session_dir: &Path, from_ms: i64, to_ms: i64) -> Vec<CursorSample> {
    if to_ms < from_ms {
        return Vec::new();
    }
    let cuts = crate::session::active_cuts(session_dir);
    let mut out: Vec<CursorSample> = Vec::new();
    for (base, path) in run_files(session_dir) {
        // A run cannot hold samples from before it began.
        if base > to_ms {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        out.extend(
            decode(base, &bytes)
                .into_iter()
                .filter(|s| s.t >= from_ms && s.t <= to_ms)
                .filter(|s| !cuts.iter().any(|c| cut_hides_cursor(c, s.t))),
        );
    }
    out.sort_by_key(|s| s.t);
    out
}

/// Appends one run's samples. Owned by the recorder's sampler thread.
pub struct CursorWriter {
    file: std::io::BufWriter<std::fs::File>,
    base: i64,
    last: Option<(i16, i16, i64)>,
    unflushed: usize,
}

impl CursorWriter {
    /// Start a run file at `<session>/cursor/<base_epoch_ms>.bin`.
    pub fn create(session_dir: &Path, base_epoch_ms: i64) -> Result<Self, String> {
        let dir = session_dir.join(CURSOR_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
        let path = dir.join(format!("{}.bin", base_epoch_ms));
        let mut file = std::io::BufWriter::new(
            std::fs::File::create(&path).map_err(|e| format!("{}: {}", path.display(), e))?,
        );
        file.write_all(MAGIC).map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            base: base_epoch_ms,
            last: None,
            unflushed: 0,
        })
    }

    /// Record the cursor at `t`, if it moved or has held still for a second.
    /// Returns whether a record was written.
    pub fn sample(&mut self, t: i64, x: i32, y: i32) -> Result<bool, String> {
        let (x, y) = (clamp16(x), clamp16(y));
        let due = match self.last {
            None => true,
            Some((lx, ly, lt)) => lx != x || ly != y || t - lt >= KEEPALIVE_MS,
        };
        let offset = t - self.base;
        if !due || offset < 0 || offset > u32::MAX as i64 {
            return Ok(false);
        }
        let mut rec = [0u8; RECORD_BYTES];
        rec[..4].copy_from_slice(&(offset as u32).to_le_bytes());
        rec[4..6].copy_from_slice(&x.to_le_bytes());
        rec[6..].copy_from_slice(&y.to_le_bytes());
        self.file.write_all(&rec).map_err(|e| e.to_string())?;
        self.last = Some((x, y, t));
        self.unflushed += 1;
        if self.unflushed >= FLUSH_EVERY {
            self.flush()?;
        }
        Ok(true)
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.unflushed = 0;
        self.file.flush().map_err(|e| e.to_string())
    }
}

impl Drop for CursorWriter {
    fn drop(&mut self) {
        let _ = self.file.flush();
    }
}

/// The cursor in virtual-desktop pixels. `None` when Windows will not say,
/// which happens on the secure desktop (locked, UAC prompt).
///
/// The process is per-monitor DPI aware (`recorder::ensure_process_dpi_aware`),
/// so these are physical pixels, the same space as the DXGI monitor rects.
#[cfg(target_os = "windows")]
pub fn cursor_pos() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.ok()?;
    Some((p.x, p.y))
}

#[cfg(not(target_os = "windows"))]
pub fn cursor_pos() -> Option<(i32, i32)> {
    None
}

#[tauri::command]
pub async fn get_cursor_track(
    state: tauri::State<'_, std::sync::Arc<tokio::sync::Mutex<crate::state::AppState>>>,
    session_id: String,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<CursorSample>, String> {
    crate::edit::check_id(&session_id)?;
    let dir = {
        let s = state.lock().await;
        crate::recorder::sessions_root(&crate::capture::get_output_dir(&s.config)).join(&session_id)
    };
    tokio::task::spawn_blocking(move || read_track(&dir, from_ms, to_ms))
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "chirptime-cursor-{}-{}-{}",
            tag,
            std::process::id(),
            crate::session::now_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cut(from: i64, to: i64, tracks: Option<Vec<&str>>, state: CutState) -> Cut {
        Cut {
            id: format!("cut-{}", from),
            from_epoch_ms: from,
            to_epoch_ms: to,
            tracks: tracks.map(|v| v.into_iter().map(String::from).collect()),
            created_epoch_ms: from,
            state,
            freed_bytes: 0,
        }
    }

    #[test]
    fn a_still_cursor_writes_once_a_second_and_a_moving_one_every_sample() {
        let dir = tmp("keepalive");
        let base = 1_000_000;
        let mut w = CursorWriter::create(&dir, base).unwrap();
        let mut written = 0;
        // 3 s still, then 1 s moving.
        for i in 0..30 {
            written += w.sample(base + i * 100, 50, 60).unwrap() as i32;
        }
        assert_eq!(written, 3);
        for i in 30..40 {
            written += w.sample(base + i * 100, 50 + i as i32, 60).unwrap() as i32;
        }
        assert_eq!(written, 13);
        drop(w);
        let s = read_track(&dir, 0, i64::MAX);
        assert_eq!(s.len(), 13);
        assert_eq!(s[0], CursorSample { t: base, x: 50, y: 60 });
        assert!(s.windows(2).all(|p| p[0].t < p[1].t));
    }

    #[test]
    fn an_eight_hour_day_of_constant_movement_is_a_few_mb() {
        let records = 8 * 3600 * (1000 / SAMPLE_MS as usize);
        assert!(MAGIC.len() + records * RECORD_BYTES < 3 * 1024 * 1024);
    }

    #[test]
    fn negative_and_far_coordinates_round_trip_or_clamp() {
        let dir = tmp("coords");
        let mut w = CursorWriter::create(&dir, 0).unwrap();
        w.sample(10, -2560, -400).unwrap();
        w.sample(20, 7679, 2159).unwrap();
        w.sample(30, 100_000, -100_000).unwrap();
        drop(w);
        let s = read_track(&dir, 0, 100);
        assert_eq!(
            s.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(),
            vec![(-2560, -400), (7679, 2159), (32767, -32768)]
        );
    }

    #[test]
    fn the_range_is_inclusive_and_runs_merge_in_order() {
        let dir = tmp("runs");
        // Two runs either side of a pause, written out of order on purpose.
        let mut b = CursorWriter::create(&dir, 5_000).unwrap();
        b.sample(5_000, 3, 3).unwrap();
        b.sample(5_100, 4, 4).unwrap();
        drop(b);
        let mut a = CursorWriter::create(&dir, 1_000).unwrap();
        a.sample(1_000, 1, 1).unwrap();
        a.sample(1_100, 2, 2).unwrap();
        drop(a);
        let ts: Vec<i64> = read_track(&dir, 1_100, 5_000).iter().map(|s| s.t).collect();
        assert_eq!(ts, vec![1_100, 5_000]);
        assert!(read_track(&dir, 6_000, 5_000).is_empty());
    }

    #[test]
    fn cuts_over_the_screens_hide_the_cursor_and_others_do_not() {
        let dir = tmp("cuts");
        let mut w = CursorWriter::create(&dir, 0).unwrap();
        for i in 0..10 {
            w.sample(i * 100, i as i32, 0).unwrap();
        }
        drop(w);
        let cuts = [
            cut(100, 300, None, CutState::Final),
            cut(400, 500, Some(vec!["screen-2"]), CutState::Pending),
            cut(600, 700, Some(vec!["webcam", "mic"]), CutState::Final),
            cut(800, 900, None, CutState::Undone),
        ];
        for c in &cuts {
            crate::session::append_cut(&dir, c).unwrap();
        }
        let ts: Vec<i64> = read_track(&dir, 0, 1_000).iter().map(|s| s.t).collect();
        assert_eq!(ts, vec![0, 300, 500, 600, 700, 800, 900]);
    }

    #[test]
    fn a_torn_last_record_and_a_foreign_file_are_ignored() {
        let dir = tmp("torn");
        let mut w = CursorWriter::create(&dir, 0).unwrap();
        w.sample(0, 1, 1).unwrap();
        w.sample(100, 2, 2).unwrap();
        drop(w);
        let path = dir.join(CURSOR_DIR).join("0.bin");
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&[1, 2, 3]).unwrap();
        drop(f);
        std::fs::write(dir.join(CURSOR_DIR).join("50.bin"), b"nope, not cursor data").unwrap();
        std::fs::write(dir.join(CURSOR_DIR).join("notes.txt"), b"x").unwrap();
        assert_eq!(read_track(&dir, 0, 1_000).len(), 2);
    }

    #[test]
    fn no_cursor_folder_is_no_data_not_an_error() {
        let dir = tmp("none");
        assert!(read_track(&dir, 0, i64::MAX).is_empty());
    }
}
