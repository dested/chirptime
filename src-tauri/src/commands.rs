use crate::capture;
use crate::state::{AppState, CaptureConfig, CaptureStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
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
    let s = state.lock().await;
    let output_dir = get_output_dir_from_state(&s.config);
    let config_path = output_dir.parent().unwrap_or(&output_dir).join("monitor-config.json");

    if let Ok(data) = std::fs::read_to_string(&config_path) {
        if let Ok(config) = serde_json::from_str::<MonitorConfig>(&data) {
            return Ok(config);
        }
    }

    // Default 4-monitor config
    Ok(MonitorConfig {
        total_width: 3840,
        total_height: 1296,
        monitors: vec![
            MonitorRegion { id: 1, label: "Mon 1".to_string(), x: 0, y: 0, w: 960, h: 1296 },
            MonitorRegion { id: 2, label: "Mon 2".to_string(), x: 960, y: 0, w: 960, h: 1296 },
            MonitorRegion { id: 3, label: "Mon 3".to_string(), x: 1920, y: 0, w: 960, h: 1296 },
            MonitorRegion { id: 4, label: "Mon 4".to_string(), x: 2880, y: 0, w: 960, h: 1296 },
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
