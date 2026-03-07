use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub interval_minutes: u32,
    pub video_mode: bool,
    pub video_duration_seconds: u32,
    pub video_framerate: u32,
    pub video_preset: String,
    pub record_audio: bool,
    pub audio_device: Option<String>,
    pub pip_overlay: bool,
    pub webcam_device: Option<String>,
    pub output_dir: String,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            interval_minutes: 10,
            video_mode: true,
            video_duration_seconds: 5,
            video_framerate: 30,
            video_preset: "ultrafast".to_string(),
            record_audio: true,
            audio_device: None,
            pip_overlay: true,
            webcam_device: None,
            output_dir: "G:/code/screener/output".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStatus {
    pub running: bool,
    pub last_capture_time: Option<String>,
    pub next_capture_time: Option<String>,
    pub captures_today: u32,
}

impl Default for CaptureStatus {
    fn default() -> Self {
        Self {
            running: false,
            last_capture_time: None,
            next_capture_time: None,
            captures_today: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct AppState {
    pub config: CaptureConfig,
    pub status: CaptureStatus,
    pub capture_cancel: Option<tokio::sync::oneshot::Sender<()>>,
}
