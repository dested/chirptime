use serde::{Deserialize, Serialize};

/// Every field is `#[serde(default)]` so a settings file written by an older
/// build — or one missing a key entirely — still loads the fields it does have
/// instead of being thrown away wholesale. See `settings::load_from`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

/// Where recordings go when nothing has been configured: `<Videos>/ChirpTime`,
/// falling back to `<profile>/Videos/ChirpTime` and finally a relative `output/`.
///
/// This used to be a hardcoded absolute path on the original author's machine,
/// which meant the app silently wrote nowhere useful for anybody else. Because
/// this decides where hours of somebody's life get written, the UI must show the
/// resolved path before the first recording rather than leaving it implicit.
pub fn default_output_dir() -> String {
    #[cfg(target_os = "windows")]
    {
        // USERPROFILE is set on every normal Windows session; the Videos folder
        // can be relocated, but %USERPROFILE%\Videos is right in the common case
        // and is a far better guess than a path on someone else's D drive.
        if let Ok(profile) = std::env::var("USERPROFILE") {
            if !profile.is_empty() {
                return format!("{}\\Videos\\ChirpTime", profile.trim_end_matches('\\'));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().to_string();
        if !home.is_empty() {
            return format!("{}/Videos/ChirpTime", home.trim_end_matches('/'));
        }
    }
    "output".to_string()
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
            output_dir: default_output_dir(),
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
    /// Continuous-session recorder. Independent of the interval capture loop
    /// above — both can run at once.
    pub recorder: Option<std::sync::Arc<crate::recorder::Recorder>>,
    /// Last-used `recorder::SessionOptions`, kept as raw JSON because that type
    /// is deserialize-only. Written by the settings panel, read back by it, and
    /// handed to `start_session` by the frontend. `Null` = never configured.
    pub session_options: serde_json::Value,
    /// The first-run panel has been dismissed.
    pub onboarded: bool,
    /// A folder holding FFmpeg that the user pointed at by hand, or `None`.
    /// Mirrored here only so `settings()` can round-trip it — the lookups in
    /// `capture::find_ffmpeg` read it off disk, because they run on threads
    /// that never see this state.
    pub ffmpeg_dir: Option<String>,
}

impl AppState {
    pub fn from_settings(s: crate::settings::Settings) -> Self {
        Self {
            config: s.capture,
            session_options: s.session,
            onboarded: s.onboarded,
            ffmpeg_dir: s.ffmpeg_dir,
            ..Default::default()
        }
    }

    /// The persistable half of this state.
    pub fn settings(&self) -> crate::settings::Settings {
        crate::settings::Settings {
            version: crate::settings::SETTINGS_VERSION,
            capture: self.config.clone(),
            session: self.session_options.clone(),
            onboarded: self.onboarded,
            ffmpeg_dir: self.ffmpeg_dir.clone(),
        }
    }
}
