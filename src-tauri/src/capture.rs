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

/// Locate `ffmpeg.exe`.
///
/// Order matters and is deliberate. A folder the user pointed at by hand wins
/// outright — they said so explicitly. Then the three lookups that predate the
/// first-run installer, so a machine that already had FFmpeg keeps using the
/// exact binary it always used and downloads nothing. The app-managed install
/// from `ffmpeg_install` is last: it is the fallback for a machine that had
/// none, and it must never quietly displace a user's own build.
///
/// **`session::find_ffprobe` mirrors this list** and has to keep mirroring it —
/// the recorder needs both halves, and finding one without the other records
/// footage it then cannot index.
pub fn find_ffmpeg() -> Option<String> {
    if let Some(p) = crate::settings::ffmpeg_dir_override() {
        let exe = std::path::Path::new(&p).join("ffmpeg.exe");
        if exe.is_file() {
            return Some(exe.to_string_lossy().to_string());
        }
    }
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
    crate::ffmpeg_install::managed("ffmpeg.exe")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectedMonitor {
    pub id: u32,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Detect actual monitor layout from Windows, mapped to the 3840px-wide scaled output space.
/// FFmpeg gdigrab captures the full virtual desktop and `scale=3840:-2` scales it to 3840px wide.
pub fn detect_monitors() -> Option<(u32, u32, Vec<DetectedMonitor>)> {
    let ps_script = r#"
Add-Type -AssemblyName System.Windows.Forms
$screens = [System.Windows.Forms.Screen]::AllScreens
foreach ($s in $screens) {
    $b = $s.Bounds
    Write-Output "$($s.DeviceName)|$($b.X)|$($b.Y)|$($b.Width)|$($b.Height)|$($s.Primary)"
}
"#;
    let output = silent_command("powershell")
        .args(["-NoProfile", "-Command", ps_script])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut raw_monitors: Vec<(String, i32, i32, u32, u32)> = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.trim().split('|').collect();
        if parts.len() < 5 { continue; }
        let name = parts[0].to_string();
        let x: i32 = parts[1].parse().ok()?;
        let y: i32 = parts[2].parse().ok()?;
        let w: u32 = parts[3].parse().ok()?;
        let h: u32 = parts[4].parse().ok()?;
        raw_monitors.push((name, x, y, w, h));
    }

    if raw_monitors.is_empty() {
        return None;
    }

    // Compute virtual desktop bounding box
    let min_x = raw_monitors.iter().map(|m| m.1).min().unwrap();
    let min_y = raw_monitors.iter().map(|m| m.2).min().unwrap();
    let max_x = raw_monitors.iter().map(|m| m.1 + m.3 as i32).max().unwrap();
    let max_y = raw_monitors.iter().map(|m| m.2 + m.4 as i32).max().unwrap();
    let vd_width = (max_x - min_x) as f64;
    let vd_height = (max_y - min_y) as f64;

    // FFmpeg scales to 3840px wide, height is proportional
    let scale = 3840.0 / vd_width;
    let scaled_height = (vd_height * scale).round() as u32;
    // Make height even for video codec
    let scaled_height = if scaled_height % 2 != 0 { scaled_height + 1 } else { scaled_height };

    let monitors: Vec<DetectedMonitor> = raw_monitors
        .iter()
        .enumerate()
        .map(|(i, (name, x, y, w, h))| {
            let sx = ((*x - min_x) as f64 * scale).round() as i32;
            let sy = ((*y - min_y) as f64 * scale).round() as i32;
            let sw = (*w as f64 * scale).round() as u32;
            let sh = (*h as f64 * scale).round() as u32;
            // Clean up device name: \\.\DISPLAY1 -> DISPLAY1
            let label = name.trim_start_matches("\\\\.\\").to_string();
            DetectedMonitor {
                id: (i + 1) as u32,
                label,
                x: sx,
                y: sy,
                w: sw,
                h: sh,
            }
        })
        .collect();

    Some((3840, scaled_height, monitors))
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

    let mut results = Vec::new();

    // Spawn ALL captures simultaneously so they're in sync
    let screenshot_path = screenshots_dir.join(format!("{}.png", timestamp));
    let screenshot_thread = {
        let ffmpeg = ffmpeg.clone();
        let path = screenshot_path.clone();
        std::thread::spawn(move || {
            silent_command(&ffmpeg)
                .args([
                    "-f", "gdigrab",
                    "-framerate", "1",
                    "-i", "desktop",
                    "-vframes", "1",
                    "-vf", "scale=3840:-2",
                    "-y",
                ])
                .arg(&path)
                .output()
        })
    };

    let screen_path = video_dir.join(format!("{}_screen.mp4", timestamp));
    let webcam_path = video_dir.join(format!("{}_webcam.mp4", timestamp));
    let screen_thread = if config.video_mode {
        let ffmpeg = ffmpeg.clone();
        let duration = config.video_duration_seconds.to_string();
        let framerate = config.video_framerate.to_string();
        let preset = config.video_preset.clone();
        let path = screen_path.clone();
        let audio_dev = audio_device.clone();
        Some(std::thread::spawn(move || {
            let mut cmd = silent_command(&ffmpeg);
            cmd.args([
                "-f", "gdigrab",
                "-framerate", &framerate,
                "-i", "desktop",
            ]);
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
        }))
    } else {
        None
    };

    let webcam_thread = if config.video_mode {
        webcam_device.as_ref().map(|device| {
            let ffmpeg = ffmpeg.clone();
            let duration = config.video_duration_seconds.to_string();
            let preset = config.video_preset.clone();
            let device = device.clone();
            let path = webcam_path.clone();
            let audio_dev = audio_device.clone();
            std::thread::spawn(move || {
                let mut cmd = silent_command(&ffmpeg);
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
        })
    } else {
        None
    };

    // Wait for screenshot
    match screenshot_thread.join() {
        Ok(Ok(output)) if output.status.success() => {
            results.push(format!("Screenshot: {}", screenshot_path.display()));
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            results.push(format!("Screenshot failed: {}", &stderr[stderr.len().saturating_sub(200)..]));
        }
        Ok(Err(e)) => results.push(format!("Screenshot failed: {}", e)),
        Err(_) => results.push("Screenshot failed: thread panicked".to_string()),
    }

    // Wait for screen recording
    if let Some(handle) = screen_thread {
        let screen_ok = match handle.join() {
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
                    "-filter_complex", "[1:v]scale=320:-2[pip];[0:v][pip]overlay=W-w-10:10:eof_action=pass",
                    "-c:v", "libx264",
                    "-preset", &config.video_preset,
                    "-crf", "23",
                    "-pix_fmt", "yuv420p",
                    "-c:a", "copy",
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
