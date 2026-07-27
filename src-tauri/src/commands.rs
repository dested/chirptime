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
pub async fn set_output_dir(state: tauri::State<'_, State>, dir: String) -> Result<(), String> {
    let mut s = state.lock().await;
    s.config.output_dir = dir;
    Ok(())
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
    Ok(())
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
        s.recorder.take()
    };
    let Some(recorder) = recorder else {
        return Err("No session is recording".to_string());
    };

    // `stop` joins the supervisor threads and can take several seconds waiting on
    // graceful ffmpeg shutdowns — never block the async runtime on that.
    let rec = recorder.clone();
    tokio::task::spawn_blocking(move || rec.stop())
        .await
        .map_err(|e| format!("stop panicked: {}", e))?;

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
    Ok(tokio::task::spawn_blocking(move || crate::recorder::list_sessions(&root))
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

/// Reconstruct a session's manifest purely by scanning its directory and
/// ffprobing every segment. This is the crash-recovery path: it works with no
/// `session.json` present, and the rebuilt manifest is written back atomically.
#[tauri::command]
pub async fn rebuild_session(
    state: tauri::State<'_, State>,
    id: String,
) -> Result<crate::session::SessionManifest, String> {
    let dir = {
        let s = state.lock().await;
        if let Some(rec) = &s.recorder {
            if rec.manifest().id == id && !rec.is_stopping() {
                return Err("Cannot rebuild a session that is still recording".to_string());
            }
        }
        sessions_root_for(&s.config).join(&id)
    };
    tokio::task::spawn_blocking(move || {
        let m = crate::session::rebuild_manifest_from_disk(&dir)?;
        crate::session::write_manifest(&dir, &m)?;
        Ok(m)
    })
    .await
    .map_err(|e| e.to_string())?
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
    .title("ChirpTime - Viewer")
    .inner_size(1280.0, 800.0)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}
