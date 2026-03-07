use crate::state::{AppState, CaptureConfig};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
extern "system" {
    fn Beep(dwFreq: u32, dwDuration: u32) -> i32;
}

/// Play a pleasant two-tone chirp before capture starts.
fn chirp() {
    #[cfg(target_os = "windows")]
    {
        std::thread::spawn(|| {
            unsafe {
                Beep(1200, 80);
                Beep(1600, 120);
            }
        });
    }
}

/// Create a Command that won't spawn a visible console window on Windows.
fn silent_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

pub fn get_output_dir(config: &CaptureConfig) -> PathBuf {
    let path = PathBuf::from(&config.output_dir);
    if path.is_absolute() {
        path
    } else {
        // Resolve relative paths from cwd
        std::env::current_dir().unwrap_or_default().join(&config.output_dir)
    }
}

pub fn ensure_day_dirs(output_dir: &PathBuf, date_str: &str) -> std::io::Result<(PathBuf, PathBuf, PathBuf)> {
    let day_dir = output_dir.join(date_str);
    let screenshots = day_dir.join("screenshots");
    let video = day_dir.join("video");
    let webcam = day_dir.join("webcam");
    std::fs::create_dir_all(&screenshots)?;
    std::fs::create_dir_all(&video)?;
    std::fs::create_dir_all(&webcam)?;
    Ok((screenshots, video, webcam))
}

pub fn find_ffmpeg() -> Option<String> {
    // Try ffmpeg in PATH
    if silent_command("ffmpeg").arg("-version").output().is_ok() {
        return Some("ffmpeg".to_string());
    }
    // Try common locations on Windows
    let candidates = [
        r"C:\ffmpeg\bin\ffmpeg.exe",
        r"C:\Program Files\ffmpeg\bin\ffmpeg.exe",
    ];
    for c in candidates {
        if std::path::Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    None
}

pub fn list_dshow_devices(ffmpeg: &str) -> (Vec<String>, Vec<String>) {
    let output = silent_command(ffmpeg)
        .args(["-list_devices", "true", "-f", "dshow", "-i", "dummy"])
        .output();

    let mut video = Vec::new();
    let mut audio = Vec::new();

    if let Ok(output) = output {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Regex-like manual parsing: lines like [dshow @ ...] "Device Name" (video)
        for line in stderr.lines() {
            if let Some(start) = line.find('"') {
                if let Some(end) = line[start + 1..].find('"') {
                    let name = &line[start + 1..start + 1 + end];
                    if line.contains("(video)") {
                        video.push(name.to_string());
                    } else if line.contains("(audio)") {
                        audio.push(name.to_string());
                    }
                }
            }
        }
    }

    (video, audio)
}

pub async fn run_capture_cycle(
    state: Arc<Mutex<AppState>>,
) -> Result<String, String> {
    // Chirp before capture
    chirp();

    let config = {
        let s = state.lock().await;
        s.config.clone()
    };

    let output_dir = get_output_dir(&config);
    let now = chrono::Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let timestamp = now.format("%H-%M-%S").to_string();

    let (screenshots_dir, video_dir, _webcam_dir) = ensure_day_dirs(&output_dir, &date_str)
        .map_err(|e| format!("Failed to create directories: {}", e))?;

    let ffmpeg = find_ffmpeg().ok_or("FFmpeg not found in PATH")?;

    // Auto-detect webcam if not configured
    let (auto_video, auto_audio) = list_dshow_devices(&ffmpeg);
    let webcam_device = config.webcam_device.clone().or_else(|| {
        auto_video.into_iter().next()
    });

    // Auto-detect audio device if not configured but record_audio is on
    let audio_device = if config.record_audio {
        config.audio_device.clone().or_else(|| {
            auto_audio.into_iter().next()
        })
    } else {
        None
    };

    // Capture screenshot
    let screenshot_path = screenshots_dir.join(format!("{}.png", timestamp));
    let screenshot_result = silent_command(&ffmpeg)
        .args([
            "-f", "gdigrab",
            "-framerate", "1",
            "-i", "desktop",
            "-vframes", "1",
            "-vf", "scale=3840:-2",
            "-y",
        ])
        .arg(&screenshot_path)
        .output();

    let mut results = Vec::new();
    match screenshot_result {
        Ok(output) if output.status.success() => {
            results.push(format!("Screenshot: {}", screenshot_path.display()));
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            results.push(format!("Screenshot failed: {}", &stderr[stderr.len().saturating_sub(200)..]));
        }
        Err(e) => results.push(format!("Screenshot failed: {}", e)),
    }

    // Record screen + webcam in parallel
    if config.video_mode {
        let screen_path = video_dir.join(format!("{}_screen.mp4", timestamp));
        let webcam_path = video_dir.join(format!("{}_webcam.mp4", timestamp));

        // Spawn screen recording thread
        let screen_thread = {
            let ffmpeg = ffmpeg.clone();
            let duration = config.video_duration_seconds.to_string();
            let framerate = config.video_framerate.to_string();
            let preset = config.video_preset.clone();
            let path = screen_path.clone();
            let audio_dev = audio_device.clone();
            std::thread::spawn(move || {
                let mut cmd = silent_command(&ffmpeg);
                cmd.args([
                    "-f", "gdigrab",
                    "-framerate", &framerate,
                    "-i", "desktop",
                ]);
                // Add audio input if available
                if let Some(ref dev) = audio_dev {
                    cmd.args([
                        "-f", "dshow",
                        "-i", &format!("audio={}", dev),
                    ]);
                }
                cmd.args([
                    "-t", &duration,
                    "-vf", &format!("scale=3840:-2,drawtext=text='%{{localtime\\:%Y-%m-%d %H\\\\\\:%M\\\\\\:%S}}':fontsize=24:fontcolor=white:borderw=2:bordercolor=black:x=10:y=10"),
                    "-c:v", "libx264",
                    "-preset", &preset,
                    "-crf", "23",
                    "-pix_fmt", "yuv420p",
                ]);
                if audio_dev.is_some() {
                    cmd.args(["-c:a", "aac", "-b:a", "128k"]);
                }
                cmd.args(["-y"]).arg(&path).output()
            })
        };

        // Spawn webcam recording thread (if device available)
        let webcam_thread = webcam_device.as_ref().map(|device| {
            let ffmpeg = ffmpeg.clone();
            let duration = config.video_duration_seconds.to_string();
            let preset = config.video_preset.clone();
            let device = device.clone();
            let path = webcam_path.clone();
            let audio_dev = audio_device.clone();
            std::thread::spawn(move || {
                let mut cmd = silent_command(&ffmpeg);
                // If audio device available, combine video+audio in one dshow input
                if let Some(ref adev) = audio_dev {
                    cmd.args([
                        "-f", "dshow",
                        "-i", &format!("video={}:audio={}", device, adev),
                    ]);
                } else {
                    cmd.args([
                        "-f", "dshow",
                        "-i", &format!("video={}", device),
                    ]);
                }
                cmd.args([
                    "-t", &duration,
                    "-vf", &format!("drawtext=text='%{{localtime\\:%Y-%m-%d %H\\\\\\:%M\\\\\\:%S}}':fontsize=24:fontcolor=white:borderw=2:bordercolor=black:x=10:y=10"),
                    "-c:v", "libx264",
                    "-preset", &preset,
                    "-crf", "23",
                    "-pix_fmt", "yuv420p",
                ]);
                if audio_dev.is_some() {
                    cmd.args(["-c:a", "aac", "-b:a", "128k"]);
                }
                cmd.args(["-y"]).arg(&path).output()
            })
        });

        // Wait for both to finish
        let screen_ok = match screen_thread.join() {
            Ok(Ok(output)) if output.status.success() => {
                results.push(format!("Video: {}", screen_path.display()));
                true
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                results.push(format!("Video failed: {}", &stderr[stderr.len().saturating_sub(200)..]));
                false
            }
            Ok(Err(e)) => { results.push(format!("Video failed: {}", e)); false }
            Err(_) => { results.push("Video failed: thread panicked".to_string()); false }
        };

        let webcam_ok = if let Some(handle) = webcam_thread {
            match handle.join() {
                Ok(Ok(output)) if output.status.success() => {
                    results.push(format!("Webcam: {}", webcam_path.display()));
                    true
                }
                Ok(Ok(output)) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    results.push(format!("Webcam failed: {}", &stderr[stderr.len().saturating_sub(200)..]));
                    false
                }
                Ok(Err(e)) => { results.push(format!("Webcam failed: {}", e)); false }
                Err(_) => { results.push("Webcam failed: thread panicked".to_string()); false }
            }
        } else {
            false
        };

        // Combine into PiP if both succeeded
        if screen_ok && webcam_ok && config.pip_overlay {
            let combined_path = video_dir.join(format!("{}_combined.mp4", timestamp));
            let pip_result = silent_command(&ffmpeg)
                .args([
                    "-i", &screen_path.to_string_lossy(),
                    "-i", &webcam_path.to_string_lossy(),
                    "-filter_complex", "[1:v]scale=320:-2[pip];[0:v][pip]overlay=W-w-10:10",
                    "-c:v", "libx264",
                    "-preset", &config.video_preset,
                    "-crf", "23",
                    "-pix_fmt", "yuv420p",
                    "-c:a", "copy",
                    "-shortest",
                    "-y",
                ])
                .arg(&combined_path)
                .output();

            match pip_result {
                Ok(output) if output.status.success() => {
                    results.push(format!("Combined PiP: {}", combined_path.display()));
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    results.push(format!("PiP combine failed: {}", &stderr[stderr.len().saturating_sub(200)..]));
                }
                Err(e) => results.push(format!("PiP combine failed: {}", e)),
            }
        }
    }

    // Update status
    {
        let mut s = state.lock().await;
        s.status.last_capture_time = Some(now.format("%H:%M:%S").to_string());
        s.status.captures_today += 1;

        if s.status.running {
            let next = now + chrono::Duration::minutes(config.interval_minutes as i64);
            s.status.next_capture_time = Some(next.format("%H:%M:%S").to_string());
        }
    }

    Ok(results.join("\n"))
}
