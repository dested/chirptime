//! Cursor track (S3). Lead pre-wire: `widget` owns this file and replaces the
//! stubs; `read_track` and `get_cursor_track` keep these signatures because the
//! export backend and the Player depend on them.

use serde::Serialize;
use std::path::Path;

/// One cursor position in virtual-desktop pixels (the same space as
/// `manifest.desktop` and each track's `monitor` rect), at wall-clock `t`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct CursorSample {
    pub t: i64,
    pub x: i32,
    pub y: i32,
}

/// Samples in `[from_ms, to_ms]`, ascending by `t`, excluding cut ranges.
/// Empty when the session has no cursor data.
pub fn read_track(session_dir: &Path, from_ms: i64, to_ms: i64) -> Vec<CursorSample> {
    let _ = (session_dir, from_ms, to_ms);
    Vec::new()
}

#[tauri::command]
pub async fn get_cursor_track(
    session_id: String,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<CursorSample>, String> {
    let _ = (session_id, from_ms, to_ms);
    Ok(Vec::new())
}
