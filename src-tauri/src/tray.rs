//! The tray: an icon that says idle / recording / paused, a menu with the same
//! actions as the widget, and the one Windows toast per screen outage.
//!
//! A watcher polls the recorder once a second. It is the only thing that knows
//! about starts and stops made elsewhere (the app window's recorder card calls
//! `start_session` directly), so it also re-announces `recorder-changed`.

use crate::controls::{self, RecorderControl};
use crate::session::TrackStatus;
use crate::state::AppState;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Mutex;

type State = Arc<Mutex<AppState>>;

const TRAY_ID: &str = "chirptime";
const WATCH_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Idle,
    Recording,
    Paused,
}

impl Look {
    fn of(c: &RecorderControl) -> Self {
        match (c.recording, c.paused_since_epoch_ms) {
            (false, _) => Look::Idle,
            (true, None) => Look::Recording,
            (true, Some(_)) => Look::Paused,
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Look::Idle => "ChirpTime · not recording",
            Look::Recording => "ChirpTime · recording",
            Look::Paused => "ChirpTime · paused",
        }
    }
}

// ---------------------------------------------------------------------------
// Icon
// ---------------------------------------------------------------------------

const ICON_PX: u32 = 32;
const INK_3: [u8; 3] = [0x8C, 0x88, 0x7F];
const REC: [u8; 3] = [0xE5, 0x37, 0x2B];

/// The ChirpTime mark (a dot with two chirp arcs) as 32 px RGBA, drawn here so
/// the tray needs no image files. Idle is a warm grey that reads on light and
/// dark taskbars; recording is the red dot; paused is a red ring.
pub fn icon_rgba(look: Look) -> Vec<u8> {
    let s = ICON_PX as f32 / 24.0;
    let (cx, cy) = (9.0 * s, 12.0 * s);
    let dot_r = 5.2 * s;
    let arc_w = 1.0 * s;
    let (dot, arcs) = match look {
        Look::Idle => (INK_3, INK_3),
        Look::Recording | Look::Paused => (REC, REC),
    };
    // Coverage of each shape at one sample point.
    let shape = |x: f32, y: f32| -> Option<([u8; 3], f32)> {
        let (dx, dy) = (x - cx, y - cy);
        let d = (dx * dx + dy * dy).sqrt();
        let dot_hit = match look {
            Look::Paused => d <= dot_r && d >= dot_r - 1.6 * s,
            _ => d <= dot_r,
        };
        if dot_hit {
            return Some((dot, 1.0));
        }
        // The arcs sit right of the dot, within ±48° of horizontal.
        if dx > 0.0 && dy.abs() <= dx * 1.1 {
            let r1 = 7.6 * s;
            let r2 = 11.6 * s;
            if (d - r1).abs() <= arc_w {
                return Some((arcs, 1.0));
            }
            if (d - r2).abs() <= arc_w {
                return Some((arcs, 0.55));
            }
        }
        None
    };
    let mut out = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];
    const SS: u32 = 4;
    for py in 0..ICON_PX {
        for px in 0..ICON_PX {
            let (mut a, mut rgb) = (0.0f32, [0.0f32; 3]);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    if let Some((c, alpha)) = shape(x, y) {
                        a += alpha;
                        for i in 0..3 {
                            rgb[i] += c[i] as f32 * alpha;
                        }
                    }
                }
            }
            let i = ((py * ICON_PX + px) * 4) as usize;
            if a > 0.0 {
                for k in 0..3 {
                    out[i + k] = (rgb[k] / a).round() as u8;
                }
                out[i + 3] = (a / (SS * SS) as f32 * 255.0).round() as u8;
            }
        }
    }
    out
}

fn icon(look: Look) -> Image<'static> {
    Image::new_owned(icon_rgba(look), ICON_PX, ICON_PX)
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

fn menu(app: &AppHandle, c: &RecorderControl) -> tauri::Result<Menu<tauri::Wry>> {
    let keys = app.state::<State>().try_lock().map(|s| s.hotkeys.clone()).ok();
    let key = |f: fn(&crate::settings::Hotkeys) -> &String| -> Option<String> {
        keys.as_ref().map(f).filter(|k| !k.is_empty()).cloned()
    };
    let paused = c.paused_since_epoch_ms.is_some();
    let record = MenuItem::with_id(
        app,
        "record",
        if c.recording { "Stop recording" } else { "Start recording" },
        true,
        key(|k| &k.record),
    )?;
    let pause = MenuItem::with_id(
        app,
        "pause",
        if paused { "Resume" } else { "Pause" },
        c.recording,
        key(|k| &k.pause),
    )?;
    let mark = MenuItem::with_id(app, "mark", "Mark", c.recording && !paused, key(|k| &k.mark))?;
    let open = MenuItem::with_id(app, "open", "Open ChirpTime", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show recorder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit ChirpTime", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &record,
            &pause,
            &mark,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &show,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )
}

/// Report a failed tray or hotkey action where the user will see it.
pub fn toast(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("[chirptime] toast not shown ({title}): {e}");
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let result = match id.as_str() {
            "record" => controls::toggle_recording(&app).await,
            "pause" => controls::toggle_pause(&app).await,
            "mark" => controls::mark(&app, String::new()).await.map(|_| ()),
            "open" => controls::open_app_at(&app, "#/library"),
            "show" => controls::show_recorder(&app),
            "quit" => {
                // Finish the segments in flight before leaving.
                if controls::control_state(app.state::<State>().inner()).await.recording {
                    let _ = controls::stop(&app).await;
                }
                app.exit(0);
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            toast(&app, "ChirpTime", &e);
        }
    });
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let c = RecorderControl {
        recording: false,
        session_id: None,
        paused_since_epoch_ms: None,
    };
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(Look::Idle))
        .tooltip(Look::Idle.tooltip())
        .menu(&menu(app, &c)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| on_menu(app, e.id().as_ref()))
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                if let Err(err) = controls::show_recorder(tray.app_handle()) {
                    eprintln!("[chirptime] tray: {err}");
                }
            }
        })
        .build(app)?;
    let handle = app.clone();
    tauri::async_runtime::spawn(watch(handle));
    Ok(())
}

/// Bring the icon, tooltip and menu in line with `c`.
pub fn refresh(app: &AppHandle, c: &RecorderControl) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let look = Look::of(c);
    let _ = tray.set_icon(Some(icon(look)));
    let _ = tray.set_tooltip(Some(look.tooltip()));
    match menu(app, c) {
        Ok(m) => {
            let _ = tray.set_menu(Some(m));
        }
        Err(e) => eprintln!("[chirptime] tray menu: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Watcher: state changes made elsewhere, and outage toasts
// ---------------------------------------------------------------------------

/// "Screen 2" for `screen-1`; the camera and sound by name.
pub fn track_name(id: &str) -> String {
    if let Some(n) = id.strip_prefix("screen-").and_then(|n| n.parse::<u32>().ok()) {
        return format!("Screen {}", n + 1);
    }
    match id {
        "webcam" => "The camera".to_string(),
        "mic" => "The microphone".to_string(),
        "sysaudio" => "System audio".to_string(),
        other => other.to_string(),
    }
}

/// One outage: a screen waiting since an instant, or a track that failed for
/// good. The key is stable for the whole outage, so retries never re-toast.
pub fn outage_key(id: &str, kind: &str, status: TrackStatus, waiting_since: Option<i64>) -> Option<String> {
    if status == TrackStatus::Failed {
        return Some(format!("{id}:failed"));
    }
    match waiting_since {
        Some(t) if kind == "screen" => Some(format!("{id}:{t}")),
        _ => None,
    }
}

/// The toast for the tracks that just went down.
pub fn outage_toast(down: &[(String, Option<String>)]) -> (String, String) {
    let names: Vec<String> = down.iter().map(|(id, _)| track_name(id)).collect();
    let title = match names.as_slice() {
        [one] => format!("{one} stopped recording"),
        _ => {
            let screens = down.iter().all(|(id, _)| id.starts_with("screen-"));
            if screens {
                let nums: Vec<String> = names.iter().map(|n| n.trim_start_matches("Screen ").to_string()).collect();
                format!("Screens {} stopped recording", nums.join(", "))
            } else {
                format!("{} stopped recording", names.join(", "))
            }
        }
    };
    let why = down
        .iter()
        .find_map(|(_, h)| h.clone())
        .unwrap_or_else(|| "Recording failed".to_string());
    let why = why.trim_end_matches('.');
    (title, format!("{why}. Everything else is still recording."))
}

async fn watch(app: AppHandle) {
    let mut last: Option<RecorderControl> = None;
    let mut toasted: HashSet<String> = HashSet::new();
    let mut session: Option<String> = None;
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(WATCH_MS)).await;
        let state = app.state::<State>();
        let c = controls::control_state(state.inner()).await;
        if last.as_ref() != Some(&c) {
            refresh(&app, &c);
            use tauri::Emitter;
            let _ = app.emit(controls::CHANGED_EVENT, c.clone());
            last = Some(c.clone());
        }
        if session != c.session_id {
            toasted.clear();
            session = c.session_id.clone();
        }
        if !c.recording || c.paused_since_epoch_ms.is_some() {
            continue;
        }
        let rec = match &state.lock().await.recorder {
            Some(r) => r.clone(),
            None => continue,
        };
        let mut fresh: Vec<(String, Option<String>)> = Vec::new();
        for (id, kind, status) in rec.track_overview() {
            let h = rec.track_health(&id);
            if let Some(k) = outage_key(&id, &kind, status, h.waiting_since_epoch_ms) {
                if toasted.insert(k) {
                    fresh.push((id, h.last_error_human));
                }
            }
        }
        if !fresh.is_empty() {
            let (title, body) = outage_toast(&fresh);
            toast(&app, &title, &body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_opaque_at_the_dot_clear_at_the_corner_and_differs_by_state() {
        let idle = icon_rgba(Look::Idle);
        let rec = icon_rgba(Look::Recording);
        let paused = icon_rgba(Look::Paused);
        let px = |v: &[u8], x: u32, y: u32| {
            let i = ((y * ICON_PX + x) * 4) as usize;
            [v[i], v[i + 1], v[i + 2], v[i + 3]]
        };
        assert_eq!(px(&rec, 12, 16), [0xE5, 0x37, 0x2B, 255]);
        assert_eq!(px(&idle, 12, 16)[3], 255);
        assert_eq!(px(&idle, 0, 0)[3], 0);
        // The paused ring is hollow.
        assert_eq!(px(&paused, 12, 16)[3], 0);
        assert!(idle != rec && rec != paused);
    }

    #[test]
    fn one_outage_is_one_key_however_many_retries() {
        let a = outage_key("screen-1", "screen", TrackStatus::Restarting, Some(1_000));
        let b = outage_key("screen-1", "screen", TrackStatus::Restarting, Some(1_000));
        assert_eq!(a, b);
        assert!(a.is_some());
        // A fresh outage after footage resumed is a new one.
        assert_ne!(a, outage_key("screen-1", "screen", TrackStatus::Restarting, Some(9_000)));
        // A camera restarting is not an outage; one that gave up is.
        assert!(outage_key("webcam", "webcam", TrackStatus::Restarting, None).is_none());
        assert!(outage_key("webcam", "webcam", TrackStatus::Failed, None).is_some());
        assert!(outage_key("screen-0", "screen", TrackStatus::Recording, None).is_none());
    }

    #[test]
    fn the_toast_reads_as_a_sentence() {
        let (t, b) = outage_toast(&[("screen-3".into(), Some("Display was locked or asleep".into()))]);
        assert_eq!(t, "Screen 4 stopped recording");
        assert_eq!(b, "Display was locked or asleep. Everything else is still recording.");
        let (t, b) = outage_toast(&[("screen-0".into(), None), ("screen-1".into(), None)]);
        assert_eq!(t, "Screens 1, 2 stopped recording");
        assert_eq!(b, "Recording failed. Everything else is still recording.");
        let (t, _) = outage_toast(&[("webcam".into(), None)]);
        assert_eq!(t, "The camera stopped recording");
    }
}
