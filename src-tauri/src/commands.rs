use crate::capture;
use crate::state::{AppState, CaptureConfig, CaptureStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

type State = Arc<Mutex<AppState>>;

#[derive(Serialize)]
pub struct CaptureEntry {
    timestamp: String,
    display_time: String,
    screen_video: Option<String>,
    webcam_video: Option<String>,
    combined_video: Option<String>,
    screenshot: Option<String>,
}

#[derive(Serialize)]
pub struct DateEntries {
    date: String,
    entries: Vec<CaptureEntry>,
    full_day_video: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct MonitorRegion {
    id: u32,
    label: String,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Serialize, Deserialize)]
pub struct MonitorConfig {
    #[serde(rename = "totalWidth")]
    total_width: u32,
    #[serde(rename = "totalHeight")]
    total_height: u32,
    monitors: Vec<MonitorRegion>,
}

fn get_output_dir_from_state(config: &CaptureConfig) -> PathBuf {
    capture::get_output_dir(config)
}

#[tauri::command]
pub async fn get_output_dir(state: tauri::State<'_, State>) -> Result<String, String> {
    let s = state.lock().await;
    Ok(capture::get_output_dir(&s.config).to_string_lossy().to_string())
}

#[tauri::command]
pub async fn set_output_dir(state: tauri::State<'_, State>, dir: String) -> Result<String, String> {
    let dir = dir.trim().to_string();
    if dir.is_empty() {
        return Err("Pick a folder first".to_string());
    }
    let mut s = state.lock().await;
    s.config.output_dir = dir;
    persist(&s);
    Ok(capture::get_output_dir(&s.config).to_string_lossy().to_string())
}

/// Write the current settings to disk, reporting a failure to stderr rather
/// than to the caller: a settings file that cannot be written is a real problem
/// but never a reason to reject the change the user just made in the UI.
fn persist(s: &AppState) {
    if let Err(e) = crate::settings::save(&s.settings()) {
        eprintln!("[chirptime] could not save settings: {e}");
    }
}

#[tauri::command]
pub async fn get_dates(state: tauri::State<'_, State>) -> Result<Vec<String>, String> {
    let s = state.lock().await;
    let output_dir = get_output_dir_from_state(&s.config);

    let mut dates = Vec::new();
    let entries = match std::fs::read_dir(&output_dir) {
        Ok(e) => e,
        Err(_) => return Ok(dates),
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.len() == 10 && name.chars().nth(4) == Some('-') && name.chars().nth(7) == Some('-') {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                dates.push(name);
            }
        }
    }

    dates.sort();
    dates.reverse();
    Ok(dates)
}

#[tauri::command]
pub async fn get_date_entries(state: tauri::State<'_, State>, date: String) -> Result<DateEntries, String> {
    let s = state.lock().await;
    let output_dir = get_output_dir_from_state(&s.config);
    let day_dir = output_dir.join(&date);

    let mut map: std::collections::BTreeMap<String, CaptureEntry> = std::collections::BTreeMap::new();

    let get_or_create = |map: &mut std::collections::BTreeMap<String, CaptureEntry>, ts: &str| {
        if !map.contains_key(ts) {
            map.insert(ts.to_string(), CaptureEntry {
                timestamp: ts.to_string(),
                display_time: ts.replace('-', ":"),
                screen_video: None,
                webcam_video: None,
                combined_video: None,
                screenshot: None,
            });
        }
    };

    // Scan screenshots
    let screenshots_dir = day_dir.join("screenshots");
    if let Ok(files) = std::fs::read_dir(&screenshots_dir) {
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            // Match HH-MM-SS.png or HH-MM-SS.jpg
            if let Some(ts) = name.strip_suffix(".png").or_else(|| name.strip_suffix(".jpg")) {
                if ts.len() == 8 && ts.chars().nth(2) == Some('-') && ts.chars().nth(5) == Some('-') {
                    get_or_create(&mut map, ts);
                    if let Some(entry) = map.get_mut(ts) {
                        entry.screenshot = Some(format!("{}/screenshots/{}", date, name));
                    }
                }
            }
        }
    }

    // Scan videos
    let video_dir = day_dir.join("video");
    if let Ok(files) = std::fs::read_dir(&video_dir) {
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if !name.ends_with(".mp4") { continue; }
            // Match HH-MM-SS_screen.mp4, HH-MM-SS_webcam.mp4, HH-MM-SS_combined.mp4
            if name.len() >= 15 {
                let ts = &name[..8];
                if ts.chars().nth(2) == Some('-') && ts.chars().nth(5) == Some('-') {
                    let type_part = &name[9..name.len()-4]; // between _ and .mp4
                    get_or_create(&mut map, ts);
                    if let Some(entry) = map.get_mut(ts) {
                        let rel = format!("{}/video/{}", date, name);
                        match type_part {
                            "screen" => entry.screen_video = Some(rel),
                            "webcam" => entry.webcam_video = Some(rel),
                            "combined" => entry.combined_video = Some(rel),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let entries: Vec<CaptureEntry> = map.into_values().collect();

    // Check for full day video
    let mut full_day_video = None;
    if let Ok(files) = std::fs::read_dir(&day_dir) {
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.ends_with("_full.mp4") {
                full_day_video = Some(format!("{}/{}", date, name));
                break;
            }
        }
    }

    Ok(DateEntries { date, entries, full_day_video })
}

#[tauri::command]
pub async fn get_monitor_config(state: tauri::State<'_, State>) -> Result<MonitorConfig, String> {
    // Auto-detect monitors from the system (always preferred)
    if let Some((total_w, total_h, detected)) = capture::detect_monitors() {
        let monitors = detected.into_iter().map(|m| MonitorRegion {
            id: m.id,
            label: m.label,
            x: m.x as u32,
            y: m.y as u32,
            w: m.w,
            h: m.h,
        }).collect();
        return Ok(MonitorConfig {
            total_width: total_w,
            total_height: total_h,
            monitors,
        });
    }

    // Fallback: try config file
    let s = state.lock().await;
    let output_dir = get_output_dir_from_state(&s.config);
    let config_path = output_dir.parent().unwrap_or(&output_dir).join("monitor-config.json");

    if let Ok(data) = std::fs::read_to_string(&config_path) {
        if let Ok(config) = serde_json::from_str::<MonitorConfig>(&data) {
            return Ok(config);
        }
    }

    // Fallback: single monitor
    Ok(MonitorConfig {
        total_width: 3840,
        total_height: 2160,
        monitors: vec![
            MonitorRegion { id: 1, label: "Display".to_string(), x: 0, y: 0, w: 3840, h: 2160 },
        ],
    })
}

#[tauri::command]
pub async fn save_monitor_config(state: tauri::State<'_, State>, config: MonitorConfig) -> Result<(), String> {
    let s = state.lock().await;
    let output_dir = get_output_dir_from_state(&s.config);
    let config_path = output_dir.parent().unwrap_or(&output_dir).join("monitor-config.json");
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(config_path, json).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_entry(state: tauri::State<'_, State>, date: String, timestamp: String) -> Result<(), String> {
    let s = state.lock().await;
    let output_dir = get_output_dir_from_state(&s.config);
    let day_dir = output_dir.join(&date);

    // Delete screenshot
    for ext in ["png", "jpg"] {
        let p = day_dir.join("screenshots").join(format!("{}.{}", timestamp, ext));
        let _ = std::fs::remove_file(&p);
    }

    // Delete videos
    for suffix in ["screen", "webcam", "combined"] {
        let p = day_dir.join("video").join(format!("{}_{}.mp4", timestamp, suffix));
        let _ = std::fs::remove_file(&p);
    }

    // Delete webcam photos
    for ext in ["png", "jpg"] {
        let p = day_dir.join("webcam").join(format!("{}.{}", timestamp, ext));
        let _ = std::fs::remove_file(&p);
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_file(state: tauri::State<'_, State>, rel_path: String) -> Result<(), String> {
    let s = state.lock().await;
    let output_dir = get_output_dir_from_state(&s.config);
    let full_path = output_dir.join(&rel_path);

    // Security: ensure path is within output dir
    let canonical_output = output_dir.canonicalize().map_err(|e| e.to_string())?;
    let canonical_file = full_path.canonicalize().map_err(|e| e.to_string())?;
    if !canonical_file.starts_with(&canonical_output) {
        return Err("Path traversal attempt".to_string());
    }

    std::fs::remove_file(&canonical_file).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_capture_config(state: tauri::State<'_, State>) -> Result<CaptureConfig, String> {
    let s = state.lock().await;
    Ok(s.config.clone())
}

#[tauri::command]
pub async fn set_capture_config(state: tauri::State<'_, State>, config: CaptureConfig) -> Result<(), String> {
    let mut s = state.lock().await;
    s.config = config;
    if s.config.output_dir.trim().is_empty() {
        s.config.output_dir = crate::state::default_output_dir();
    }
    persist(&s);
    Ok(())
}

// ===========================================================================
// Settings, onboarding, and the output folder
// ===========================================================================

/// Everything about *this machine* the first-run panel has to state plainly:
/// where recordings land, whether that volume has room, and whether the tools
/// the recorder needs are actually installed.
#[derive(Serialize)]
pub struct EnvironmentInfo {
    /// Resolved and absolute — never `%USERPROFILE%\...`. This is the path the
    /// user is being asked to accept, so it is the path they get shown.
    pub output_dir: String,
    pub output_dir_exists: bool,
    pub disk_free_bytes: Option<u64>,
    pub disk_total_bytes: Option<u64>,
    /// `None` when ffmpeg is not on PATH or in the two usual install dirs —
    /// nothing records without it.
    pub ffmpeg_path: Option<String>,
    /// An NVENC encoder is present, i.e. the fast path is available.
    pub nvenc: bool,
    /// A non-empty `output/` beside the exe or repo that is *not* the configured
    /// directory. Reported, never touched.
    pub legacy_output: Option<crate::settings::LegacyOutput>,
    pub settings_path: String,
}

#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, State>) -> Result<crate::settings::Settings, String> {
    let s = state.lock().await;
    Ok(s.settings())
}

/// Persist the session recorder's options. The blob is validated as
/// `SessionOptions` before it is stored, so nothing unparseable ever reaches
/// `start_session` from disk.
#[tauri::command]
pub async fn set_session_options(
    state: tauri::State<'_, State>,
    options: serde_json::Value,
) -> Result<(), String> {
    serde_json::from_value::<crate::recorder::SessionOptions>(options.clone())
        .map_err(|e| format!("invalid session options: {e}"))?;
    let mut s = state.lock().await;
    s.session_options = options;
    persist(&s);
    Ok(())
}

/// Dismiss (or, for testing, re-arm) the first-run panel.
#[tauri::command]
pub async fn set_onboarded(state: tauri::State<'_, State>, value: bool) -> Result<(), String> {
    let mut s = state.lock().await;
    s.onboarded = value;
    persist(&s);
    Ok(())
}

#[tauri::command]
pub async fn get_environment(state: tauri::State<'_, State>) -> Result<EnvironmentInfo, String> {
    let dir = {
        let s = state.lock().await;
        capture::get_output_dir(&s.config)
    };
    // ffmpeg probing shells out twice; keep it off the async runtime.
    tokio::task::spawn_blocking(move || {
        let (free, total) = match crate::nativeui::disk_space(&dir) {
            Some((f, t)) => (Some(f), Some(t)),
            None => (None, None),
        };
        let ffmpeg_path = capture::find_ffmpeg();
        let nvenc = ffmpeg_path
            .as_deref()
            .map(|f| crate::recorder::detect_encoders(f).h264_nvenc)
            .unwrap_or(false);
        EnvironmentInfo {
            output_dir: crate::nativeui::display_path(&dir),
            output_dir_exists: dir.exists(),
            disk_free_bytes: free,
            disk_total_bytes: total,
            ffmpeg_path,
            nvenc,
            legacy_output: crate::settings::find_legacy_output(&dir),
            settings_path: crate::nativeui::display_path(&crate::settings::settings_path()),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Native folder picker. Applies and persists the choice, and returns the
/// resolved path — or `None` if the user cancelled, which is not an error.
#[tauri::command]
pub async fn pick_output_dir(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<Option<String>, String> {
    let start = {
        let s = state.lock().await;
        capture::get_output_dir(&s.config)
    };
    // Owning the dialog to the widget matters: the widget is always-on-top, so
    // an unowned dialog opens behind it.
    let parent = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize);

    let picked = tokio::task::spawn_blocking(move || {
        crate::nativeui::pick_folder(Some(&start), parent)
    })
    .await
    .map_err(|e| e.to_string())??;

    let Some(picked) = picked else { return Ok(None) };

    let mut s = state.lock().await;
    s.config.output_dir = picked;
    persist(&s);
    Ok(Some(capture::get_output_dir(&s.config).to_string_lossy().to_string()))
}

// ---------------------------------------------------------------------------
// First-run FFmpeg install
// ---------------------------------------------------------------------------

/// What the widget needs to decide whether to offer the download, and what to
/// say about it. `download_bytes` and `version` are surfaced so the user is told
/// the size *before* agreeing to a 105 MB transfer, never after.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FfmpegSetup {
    /// A usable ffmpeg was found. False is the only state that offers a download.
    pub ffmpeg_path: Option<String>,
    /// Resolved separately from ffmpeg because a machine can have one without
    /// the other, and the session recorder needs both.
    pub ffprobe_path: Option<String>,
    /// This app's own install exists under `%APPDATA%`.
    pub managed: bool,
    /// The folder the user pointed at by hand, if any.
    pub manual_dir: Option<String>,
    /// Where a download would land.
    pub install_dir: String,
    pub download_url: String,
    pub download_bytes: u64,
    pub version: String,
    /// True while an install is running, so a reopened panel rejoins rather than
    /// offering a second download.
    pub installing: bool,
}

/// One install at a time. Two concurrent runs would fight over the same staging
/// directory and the loser would delete the winner's work.
static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
pub async fn get_ffmpeg_setup() -> Result<FfmpegSetup, String> {
    // Both lookups spawn a process when PATH is populated; keep them off the
    // async runtime like `get_environment` does.
    tokio::task::spawn_blocking(|| FfmpegSetup {
        ffmpeg_path: capture::find_ffmpeg(),
        ffprobe_path: crate::session::find_ffprobe(),
        managed: crate::ffmpeg_install::is_installed(),
        manual_dir: crate::settings::ffmpeg_dir_override(),
        install_dir: crate::nativeui::display_path(&crate::ffmpeg_install::bin_dir()),
        download_url: crate::ffmpeg_install::FFMPEG_URL.to_string(),
        download_bytes: crate::ffmpeg_install::FFMPEG_ZIP_BYTES,
        version: crate::ffmpeg_install::FFMPEG_VERSION.to_string(),
        installing: INSTALLING.load(std::sync::atomic::Ordering::SeqCst),
    })
    .await
    .map_err(|e| e.to_string())
}

/// Download, verify and install FFmpeg. **Only ever called from a button** —
/// nothing in the app reaches this on its own, because ~105 MB of someone
/// else's bandwidth is not ours to spend unasked.
///
/// Progress is emitted as `ffmpeg-install-progress` events rather than returned,
/// so a 105 MB download is not a silent hang. The returned string is the
/// resolved `ffmpeg.exe` path.
#[tauri::command]
pub async fn install_ffmpeg(app: tauri::AppHandle) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("An FFmpeg install is already running.".into());
    }

    let result = tokio::task::spawn_blocking(move || {
        crate::ffmpeg_install::install(|p| {
            use tauri::Emitter;
            // A dropped progress event is cosmetic; it must never fail the
            // install that is otherwise going fine.
            let _ = app.emit("ffmpeg-install-progress", p);
        })
    })
    .await;

    INSTALLING.store(false, Ordering::SeqCst);

    let dir = result.map_err(|e| e.to_string())??;
    capture::find_ffmpeg().ok_or_else(|| {
        format!(
            "FFmpeg installed to {} but still cannot be resolved.",
            dir.display()
        )
    })
}

/// Point ChirpTime at an FFmpeg the user already has. This is the way out for
/// someone whose network blocks the download — without it, a blocked download is
/// a dead end.
///
/// The folder is rejected unless it holds **both** binaries and both of them
/// run, so an accepted choice is one that will actually work rather than one
/// that fails later during a recording.
#[tauri::command]
pub async fn pick_ffmpeg_dir(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<Option<String>, String> {
    // The widget is always-on-top; an unowned dialog opens behind it.
    let parent = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize);

    let picked = tokio::task::spawn_blocking(move || crate::nativeui::pick_folder(None, parent))
        .await
        .map_err(|e| e.to_string())??;

    let Some(picked) = picked else { return Ok(None) };

    let dir = std::path::PathBuf::from(&picked);
    // gyan-style archives put the binaries in `bin/`, so accept either the
    // folder itself or its `bin` — picking the extracted root is the obvious
    // mistake and there is no reason to punish it.
    let candidates = [dir.clone(), dir.join("bin")];
    let found = candidates.iter().find(|d| {
        d.join("ffmpeg.exe").is_file() && d.join("ffprobe.exe").is_file()
    });
    let Some(found) = found else {
        return Err(format!(
            "{} does not contain both ffmpeg.exe and ffprobe.exe. Pick the folder \
             holding them (often the build's \"bin\" folder).",
            crate::nativeui::display_path(&dir)
        ));
    };

    let resolved = found.to_string_lossy().to_string();
    {
        let mut s = state.lock().await;
        s.ffmpeg_dir = Some(resolved.clone());
        persist(&s);
    }

    // Persisted before verifying so `find_ffmpeg` picks it up, then verified for
    // real — a folder with the right filenames but an unrunnable binary is
    // rejected and the setting rolled back rather than left to fail mid-capture.
    let ok = tokio::task::spawn_blocking(|| capture::find_ffmpeg().is_some() && crate::session::find_ffprobe().is_some())
        .await
        .map_err(|e| e.to_string())?;
    if !ok {
        let mut s = state.lock().await;
        s.ffmpeg_dir = None;
        persist(&s);
        return Err(format!("{resolved} holds both binaries but they could not be run."));
    }
    Ok(Some(resolved))
}

/// Forget a hand-picked folder and fall back to the normal lookup order.
#[tauri::command]
pub async fn clear_ffmpeg_dir(state: tauri::State<'_, State>) -> Result<(), String> {
    let mut s = state.lock().await;
    s.ffmpeg_dir = None;
    persist(&s);
    Ok(())
}

/// Show a folder in Explorer. With no argument this is "where are my
/// recordings" — the configured output dir, created first if the user has not
/// recorded yet so the button is never a dead end.
#[tauri::command]
pub async fn open_folder(state: tauri::State<'_, State>, path: Option<String>) -> Result<(), String> {
    let dir = match path {
        Some(p) => crate::nativeui::absolute(&p),
        None => {
            let s = state.lock().await;
            let dir = capture::get_output_dir(&s.config);
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("create {}: {e}", dir.display()))?;
            dir
        }
    };
    if !dir.exists() {
        return Err(format!("{} does not exist", crate::nativeui::display_path(&dir)));
    }
    crate::nativeui::open_in_explorer(&dir)
}

#[tauri::command]
pub async fn start_capture(state: tauri::State<'_, State>) -> Result<(), String> {
    let state_inner = state.inner().clone();

    {
        let mut s = state_inner.lock().await;
        if s.status.running {
            return Err("Capture already running".to_string());
        }
        s.status.running = true;

        let interval = s.config.interval_minutes;
        let now = chrono::Local::now();
        let next = now + chrono::Duration::minutes(interval as i64);
        s.status.next_capture_time = Some(next.format("%H:%M:%S").to_string());
    }

    let state_clone = state_inner.clone();

    // Run first capture immediately
    let _ = capture::run_capture_cycle(state_clone.clone()).await;

    // Spawn background loop
    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
    {
        let mut s = state_inner.lock().await;
        s.capture_cancel = Some(tx);
    }

    tokio::spawn(async move {
        let interval_minutes = {
            let s = state_clone.lock().await;
            s.config.interval_minutes
        };
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_minutes as u64 * 60));
        interval.tick().await; // skip first (already ran)

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let _ = capture::run_capture_cycle(state_clone.clone()).await;
                }
                _ = &mut rx => {
                    break;
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn stop_capture(state: tauri::State<'_, State>) -> Result<(), String> {
    let mut s = state.lock().await;
    s.status.running = false;
    s.status.next_capture_time = None;
    if let Some(cancel) = s.capture_cancel.take() {
        let _ = cancel.send(());
    }
    Ok(())
}

#[tauri::command]
pub async fn get_capture_status(state: tauri::State<'_, State>) -> Result<CaptureStatus, String> {
    let s = state.lock().await;
    Ok(s.status.clone())
}

#[tauri::command]
pub async fn run_single_capture(state: tauri::State<'_, State>) -> Result<String, String> {
    let state_inner = state.inner().clone();
    capture::run_capture_cycle(state_inner).await
}

#[tauri::command]
pub async fn get_ffmpeg_path() -> Result<String, String> {
    capture::find_ffmpeg().ok_or_else(|| "FFmpeg not found".to_string())
}

#[derive(Serialize)]
pub struct DshowDevices {
    pub video: Vec<String>,
    pub audio: Vec<String>,
}

#[tauri::command]
pub async fn list_devices() -> Result<DshowDevices, String> {
    let ffmpeg = capture::find_ffmpeg().ok_or("FFmpeg not found")?;
    let (video, audio) = capture::list_dshow_devices(&ffmpeg);
    Ok(DshowDevices { video, audio })
}

// ===========================================================================
// Continuous session recording
// ===========================================================================

#[derive(Serialize)]
pub struct TrackSummary {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub status: crate::session::TrackStatus,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub start_epoch_ms: i64,
    pub segment_count: usize,
    pub bytes: u64,
    pub last_segment_end_ms: Option<i64>,
    pub gap_count: usize,
    pub restarts: u32,
    pub error: Option<String>,
    #[serde(flatten)]
    pub health: crate::recorder::TrackHealth,
}

#[derive(Serialize)]
pub struct SessionStatus {
    pub recording: bool,
    pub session_id: Option<String>,
    pub session_dir: Option<String>,
    pub started_epoch_ms: Option<i64>,
    pub elapsed_ms: i64,
    pub segment_seconds: u32,
    pub tracks: Vec<TrackSummary>,
    pub total_bytes: u64,
    pub segment_count: usize,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    /// Free bytes on the recording volume. `None` when it could not be read.
    pub disk_free_bytes: Option<u64>,
    /// The recorder stops the session cleanly below this — the UI colours
    /// `disk_free_bytes` against the backend's own threshold, never its own.
    pub min_free_bytes: u64,
}

impl SessionStatus {
    fn idle() -> Self {
        Self {
            recording: false,
            session_id: None,
            session_dir: None,
            started_epoch_ms: None,
            elapsed_ms: 0,
            segment_seconds: 0,
            tracks: Vec::new(),
            total_bytes: 0,
            segment_count: 0,
            warnings: Vec::new(),
            error: None,
            disk_free_bytes: None,
            min_free_bytes: crate::recorder::MIN_FREE_BYTES,
        }
    }

    fn from_recorder(rec: &crate::recorder::Recorder) -> Self {
        let m = rec.manifest();
        let now = crate::session::now_epoch_ms();
        Self {
            recording: !rec.is_stopping() && m.ended_epoch_ms.is_none(),
            session_id: Some(m.id.clone()),
            session_dir: Some(rec.session_dir().to_string_lossy().to_string()),
            started_epoch_ms: Some(m.started_epoch_ms),
            elapsed_ms: m.ended_epoch_ms.unwrap_or(now) - m.started_epoch_ms,
            segment_seconds: m.segment_seconds,
            tracks: m
                .tracks
                .iter()
                .map(|t| TrackSummary {
                    id: t.id.clone(),
                    kind: t.kind.clone(),
                    label: t.label.clone(),
                    status: t.status,
                    codec: t.codec.clone(),
                    width: t.width,
                    height: t.height,
                    fps: t.fps,
                    start_epoch_ms: t.start_epoch_ms,
                    segment_count: t.segments.len(),
                    bytes: t.total_bytes(),
                    last_segment_end_ms: t.end_epoch_ms(),
                    gap_count: t.gaps.len(),
                    restarts: t.restarts,
                    error: t.error.clone(),
                    health: rec.track_health(&t.id),
                })
                .collect(),
            total_bytes: m.total_bytes(),
            segment_count: m.segment_count(),
            warnings: rec.warnings(),
            error: rec.fatal().or(m.error.clone()),
            disk_free_bytes: crate::recorder::free_space_bytes(&rec.session_dir()),
            min_free_bytes: crate::recorder::MIN_FREE_BYTES,
        }
    }
}

fn sessions_root_for(config: &CaptureConfig) -> PathBuf {
    crate::recorder::sessions_root(&capture::get_output_dir(config))
}

#[tauri::command]
pub async fn start_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    options: Option<crate::recorder::SessionOptions>,
) -> Result<SessionStatus, String> {
    let state_inner = state.inner().clone();

    let root = {
        let s = state_inner.lock().await;
        if let Some(rec) = &s.recorder {
            if !rec.is_stopping() {
                return Err("A session is already recording".to_string());
            }
        }
        sessions_root_for(&s.config)
    };
    std::fs::create_dir_all(&root).map_err(|e| format!("create {}: {}", root.display(), e))?;

    let env = crate::recorder::RecorderEnv::from_app(&app);
    let recorder = crate::recorder::Recorder::start(env, &root, options.unwrap_or_default())?;

    let status = SessionStatus::from_recorder(&recorder);
    state_inner.lock().await.recorder = Some(recorder);
    Ok(status)
}

#[tauri::command]
pub async fn stop_session(state: tauri::State<'_, State>) -> Result<SessionStatus, String> {
    let recorder = {
        let mut s = state.lock().await;
        let rec = s.recorder.take();
        // Until `stop()` returns, its final flush can still overwrite the
        // manifest; the edit commands refuse this id meanwhile.
        s.stopping_id = rec.as_ref().map(|r| r.manifest().id);
        rec
    };
    let Some(recorder) = recorder else {
        return Err("No session is recording".to_string());
    };

    // `stop` joins the supervisor threads and can take several seconds waiting on
    // graceful ffmpeg shutdowns — never block the async runtime on that.
    let rec = recorder.clone();
    let stopped = tokio::task::spawn_blocking(move || rec.stop()).await;
    state.lock().await.stopping_id = None;
    stopped.map_err(|e| format!("stop panicked: {}", e))?;

    Ok(SessionStatus::from_recorder(&recorder))
}

#[tauri::command]
pub async fn get_session_status(state: tauri::State<'_, State>) -> Result<SessionStatus, String> {
    let s = state.lock().await;
    Ok(match &s.recorder {
        Some(rec) => SessionStatus::from_recorder(rec),
        None => {
            // Idle still reports the disk, so the user can see there is room
            // before starting rather than after being stopped.
            let mut idle = SessionStatus::idle();
            idle.disk_free_bytes =
                crate::recorder::free_space_bytes(&capture::get_output_dir(&s.config));
            idle
        }
    })
}

#[tauri::command]
pub async fn list_sessions(
    state: tauri::State<'_, State>,
) -> Result<Vec<crate::recorder::SessionSummary>, String> {
    let root = {
        let s = state.lock().await;
        sessions_root_for(&s.config)
    };
    // `edit::visible_sessions` is `recorder::list_sessions` minus the `.trash`
    // folder whole-session deletes park in.
    Ok(tokio::task::spawn_blocking(move || crate::edit::visible_sessions(&root))
        .await
        .map_err(|e| e.to_string())?)
}

/// Full manifest for one session. Reads the live in-memory manifest when the
/// requested session is the one currently recording, so the UI never sees a
/// manifest staler than the last flushed segment.
#[tauri::command]
pub async fn get_session(
    state: tauri::State<'_, State>,
    id: String,
) -> Result<crate::session::SessionManifest, String> {
    let dir = {
        let s = state.lock().await;
        if let Some(rec) = &s.recorder {
            let m = rec.manifest();
            if m.id == id {
                return Ok(m);
            }
        }
        sessions_root_for(&s.config).join(&id)
    };
    tokio::task::spawn_blocking(move || {
        crate::session::read_manifest(&dir)
            // No manifest on disk means the app died before flushing one.
            .or_else(|_| crate::session::rebuild_manifest_from_disk(&dir))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn add_marker(
    state: tauri::State<'_, State>,
    label: String,
) -> Result<crate::session::Marker, String> {
    let s = state.lock().await;
    let rec = s.recorder.as_ref().ok_or("No session is recording")?;
    rec.add_marker(label)
}

/// Set or clear a mark's note (S6). Lead pre-wire: `library` replaces the stub.
#[tauri::command]
pub async fn set_marker_label(
    session_id: String,
    epoch_ms: i64,
    label: String,
) -> Result<(), String> {
    let _ = (session_id, epoch_ms, label);
    Err("Editing notes is not built yet".into())
}

/// Reconstruct a session's manifest purely by scanning its directory and
/// ffprobing every segment. This is the crash-recovery path: it works with no
/// `session.json` present, and the rebuilt manifest is written back atomically.
#[tauri::command]
pub async fn rebuild_session(
    state: tauri::State<'_, State>,
    id: String,
) -> Result<crate::session::SessionManifest, String> {
    // The same gate as the edits: a self-stopped recorder is finished first, so
    // its last flush can never land on top of the rebuilt manifest.
    let (_, dir) = editable_session(state.inner(), &id)
        .await
        .map_err(|e| match e.as_str() {
            crate::edit::LIVE_SESSION_ERR => {
                "Cannot rebuild a session that is still recording".to_string()
            }
            _ => e,
        })?;
    // Under the edit lock, so a rebuild never interleaves with a delete or finalize.
    blocking(move || crate::edit::rebuild_session(&dir)).await
}

// ---------------------------------------------------------------------------
// Deleting time (see edit.rs)
// ---------------------------------------------------------------------------

/// The sessions root, and `id`'s directory under it, through `edit::editable_dir`:
/// a valid id, and neither the session recording nor the one still stopping.
async fn editable_session(state: &State, id: &str) -> Result<(PathBuf, PathBuf), String> {
    crate::edit::check_id(id)?;
    finish_self_stop(state, id).await?;
    let s = state.lock().await;
    let held = s.recorder.as_ref().map(|rec| {
        let phase = if rec.is_stopping() {
            crate::edit::RecorderPhase::StoppedItself
        } else {
            crate::edit::RecorderPhase::Recording
        };
        (rec.manifest().id, phase)
    });
    let root = sessions_root_for(&s.config);
    let dir = crate::edit::editable_dir(
        &root,
        held.as_ref().map(|(rid, phase)| (rid.as_str(), *phase)),
        s.stopping_id.as_deref(),
        id,
    )?;
    Ok((root, dir))
}

/// A recorder that stopped itself (disk full, a fatal error) stays in state with
/// its `stop()` never run, and that final flush would overwrite any edit. Run it
/// now, marked as stopping meanwhile; `stop()` is idempotent, so a later
/// `stop_session` only reads the result. The recorder stays in state so its
/// error is still reported.
async fn finish_self_stop(state: &State, id: &str) -> Result<(), String> {
    let rec = {
        let mut s = state.lock().await;
        match &s.recorder {
            Some(rec)
                if rec.is_stopping()
                    && s.stopping_id.as_deref() != Some(id)
                    && rec.manifest().id == id =>
            {
                let rec = rec.clone();
                s.stopping_id = Some(id.to_string());
                rec
            }
            _ => return Ok(()),
        }
    };
    let done = tokio::task::spawn_blocking(move || {
        rec.stop();
    })
    .await;
    state.lock().await.stopping_id = None;
    done.map_err(|e| format!("stop panicked: {}", e))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// After the undo window, finalize whatever cuts in `dir` are due.
fn schedule_finalize(dir: PathBuf) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(crate::edit::FINALIZE_AFTER_MS)).await;
        if let Err(e) = blocking(move || crate::edit::finalize_due_cuts(&dir)).await {
            eprintln!("[chirptime] edit: finalize failed: {}", e);
        }
    });
}

/// Delete `[from_epoch_ms, to_epoch_ms)` from a session, on every track or only
/// on `tracks`. Disk is freed after a 10s undo window.
#[tauri::command]
pub async fn delete_range(
    state: tauri::State<'_, State>,
    id: String,
    from_epoch_ms: i64,
    to_epoch_ms: i64,
    tracks: Option<Vec<String>>,
) -> Result<crate::edit::CutReceipt, String> {
    let (_, dir) = editable_session(state.inner(), &id).await?;
    let d = dir.clone();
    let receipt =
        blocking(move || crate::edit::delete_range(&d, from_epoch_ms, to_epoch_ms, tracks)).await?;
    schedule_finalize(dir);
    Ok(receipt)
}

#[tauri::command]
pub async fn undo_cut(
    state: tauri::State<'_, State>,
    id: String,
    cut_id: String,
) -> Result<crate::session::SessionManifest, String> {
    crate::edit::check_id(&cut_id)?;
    let (_, dir) = editable_session(state.inner(), &id).await?;
    blocking(move || crate::edit::undo_cut(&dir, &cut_id)).await
}

/// Finalize every pending cut in a session now, without waiting out the undo window.
#[tauri::command]
pub async fn finalize_cuts(
    state: tauri::State<'_, State>,
    id: String,
) -> Result<crate::session::SessionManifest, String> {
    let (_, dir) = editable_session(state.inner(), &id).await?;
    blocking(move || crate::edit::finalize_cuts(&dir)).await
}

/// Delete a whole session: instant, undoable for 10s, then purged.
#[tauri::command]
pub async fn delete_session(
    state: tauri::State<'_, State>,
    id: String,
) -> Result<crate::edit::SessionTrashReceipt, String> {
    let (root, _) = editable_session(state.inner(), &id).await?;
    let (r, i) = (root.clone(), id.clone());
    let receipt = blocking(move || crate::edit::delete_session(&r, &i)).await?;
    let token = receipt.undo_until_epoch_ms;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(crate::edit::FINALIZE_AFTER_MS)).await;
        if let Err(e) = blocking(move || crate::edit::purge_session(&root, &id, token)).await {
            eprintln!("[chirptime] edit: purge failed: {}", e);
        }
    });
    Ok(receipt)
}

#[tauri::command]
pub async fn undo_delete_session(state: tauri::State<'_, State>, id: String) -> Result<(), String> {
    let (root, _) = editable_session(state.inner(), &id).await?;
    blocking(move || crate::edit::undo_delete_session(&root, &id)).await
}

/// One row of the monitor picker: the raw DXGI output plus the recorder's own
/// `screen_index` — the `N` in `screen-N`, and the value `SessionOptions.screens`
/// takes. `output_idx` cannot serve that role because it repeats across adapters.
#[derive(Serialize)]
pub struct DisplayOutputInfo {
    #[serde(flatten)]
    pub output: crate::recorder::DisplayOutput,
    /// `None` for an output with no desktop extended onto it — it gets no slot.
    pub screen_index: Option<u32>,
}

/// The DXGI output ↔ monitor mapping the recorder will use.
#[tauri::command]
pub async fn list_display_outputs(app: tauri::AppHandle) -> Result<Vec<DisplayOutputInfo>, String> {
    let _ = &app;
    tokio::task::spawn_blocking(|| {
        let outputs = crate::recorder::enumerate_dxgi_outputs()?;
        // Enumeration alone cannot say whether an output is capturable — that
        // depends on which encoders exist. Without this the picker would offer
        // the iGPU monitor on a machine with no QSV, and the track would simply
        // never appear.
        let caps = capture::find_ffmpeg()
            .map(|f| crate::recorder::detect_encoders(&f))
            .unwrap_or_default();

        let mut next_index: u32 = 0;
        Ok(outputs
            .into_iter()
            .map(|mut o| {
                let screen_index = if o.attached {
                    let i = next_index;
                    next_index += 1;
                    Some(i)
                } else {
                    None
                };
                o.capturable = crate::recorder::output_is_capturable(&o, &caps);
                DisplayOutputInfo {
                    output: o,
                    screen_index,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn open_viewer(
    app: tauri::AppHandle,
    date: Option<String>,
    timestamp: Option<String>,
    session: Option<String>,
    tab: Option<String>,
) -> Result<(), String> {
    let mut url = "/?mode=viewer".to_string();
    if let Some(d) = &date {
        url.push_str(&format!("&date={}", d));
    }
    if let Some(s) = &session {
        url.push_str(&format!("&session={}", s));
    }
    if let Some(t) = &tab {
        url.push_str(&format!("&tab={}", t));
    }
    // `ts` is the clip's `HH-MM-SS` for a date link and an epoch-ms instant for a
    // session link — which one is meant is decided by `date` vs `session`.
    if let Some(t) = &timestamp {
        url.push_str(&format!("&ts={}", t));
    }

    let deep_link = date.is_some() || session.is_some() || tab.is_some();

    // If viewer window already exists, close and reopen with new URL
    if let Some(w) = app.get_webview_window("viewer") {
        if deep_link {
            let _ = w.close();
            // Small delay to let window close
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        } else {
            w.set_focus().map_err(|e| e.to_string())?;
            return Ok(());
        }
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        "viewer",
        tauri::WebviewUrl::App(url.into()),
    )
    .title("ChirpTime")
    .inner_size(1280.0, 800.0)
    .min_inner_size(1024.0, 700.0)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}
