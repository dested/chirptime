//! Recorder controls shared by the widget, the tray and the global hotkeys, so
//! all three do exactly the same thing: start and stop (with the short-session
//! discard), pause and resume, mark and note, plus the window preferences
//! (content protection, opening the app on a route).
//!
//! Every action that changes the recorder emits `recorder-changed` with the new
//! [`RecorderControl`]; a mark emits `recorder-mark` with the marker, so the
//! widget can offer the note whichever surface made it; a stop that discarded a
//! short session emits `recorder-discarded` with its receipt, for the Undo.
//!
//! Start, stop, pause, resume and Restart screens run one at a time under
//! [`ActionLock`]. A toggle (hotkey, tray) that arrives while another action
//! is under way is dropped rather than queued, so a double press never starts
//! and then stops.

use crate::state::AppState;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

type State = Arc<Mutex<AppState>>;

pub const CHANGED_EVENT: &str = "recorder-changed";
pub const MARK_EVENT: &str = "recorder-mark";
pub const DISCARDED_EVENT: &str = "recorder-discarded";
/// Shorter than this, and a session counts as a misfire for the discard setting.
pub const SHORT_SESSION_MS: i64 = 60_000;

/// What the tray, the widget and the hotkeys need to agree on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecorderControl {
    /// A session is open (recording or paused) and not stopping.
    pub recording: bool,
    pub session_id: Option<String>,
    pub paused_since_epoch_ms: Option<i64>,
}

/// What `stop_recording` did.
#[derive(Serialize)]
pub struct StopOutcome {
    pub status: crate::commands::SessionStatus,
    /// Set when the session was under a minute and the discard setting is on.
    /// It is the ordinary whole-session delete, so it has the ordinary undo.
    pub discarded: Option<crate::edit::SessionTrashReceipt>,
}

/// Window preferences read where no async lock may be taken (window events).
#[derive(Debug)]
pub struct WindowPrefs {
    pub content_protected: AtomicBool,
}

/// Set once an exit has been asked for and [`shutdown`] is running.
pub static EXITING: AtomicBool = AtomicBool::new(false);
/// Set when [`shutdown`] is done (or gave up waiting): the exit may proceed.
pub static EXIT_READY: AtomicBool = AtomicBool::new(false);

/// Held for the whole of every recorder-changing action. Managed by lib.rs.
#[derive(Default)]
pub struct ActionLock(Mutex<()>);

/// Whether quitting now would cut a session short: one is open (recording,
/// paused, or stopped by itself and not yet finalised) or a stop is still
/// writing. Contended state counts as busy. Safe from window and run events.
pub fn busy(app: &tauri::AppHandle) -> bool {
    match app.state::<State>().try_lock() {
        Ok(s) => s.recorder.is_some() || s.stopping_id.is_some(),
        Err(_) => true,
    }
}

/// Finish whatever is recording before the app exits: stop an open session
/// (a self-stopped one too, so its manifest gets its end), and wait out a stop
/// already under way. Bounded, so a wedged ffmpeg cannot keep the app alive.
pub async fn shutdown(app: &tauri::AppHandle) {
    let open = app.state::<State>().lock().await.recorder.is_some();
    if open {
        let al = app.state::<ActionLock>();
        let _g = al.0.lock().await;
        if let Err(e) = stop_locked(app).await {
            eprintln!("[chirptime] stop before exit: {e}");
        }
    }
    for _ in 0..600 {
        if app.state::<State>().lock().await.stopping_id.is_none() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    eprintln!("[chirptime] exiting with a stop still under way");
}

pub async fn control_state(state: &State) -> RecorderControl {
    let s = state.lock().await;
    match &s.recorder {
        Some(rec) if !rec.is_stopping() => RecorderControl {
            recording: true,
            session_id: Some(rec.manifest().id),
            paused_since_epoch_ms: rec.paused_since(),
        },
        _ => RecorderControl {
            recording: false,
            session_id: None,
            paused_since_epoch_ms: None,
        },
    }
}

/// Tell every window (and the tray) the recorder changed.
pub async fn announce(app: &tauri::AppHandle) {
    let state = app.state::<State>();
    let c = control_state(state.inner()).await;
    crate::tray::refresh(app, &c);
    let _ = app.emit(CHANGED_EVENT, c);
}

/// Write the settings file, logging rather than failing: the change already
/// applies in memory, and the user's choice must not bounce off a disk error.
pub fn persist(s: &AppState) {
    if let Err(e) = crate::settings::save(&s.settings()) {
        eprintln!("[chirptime] settings not saved: {e}");
    }
}

fn saved_options(s: &AppState) -> crate::recorder::SessionOptions {
    if s.session_options.is_null() {
        return Default::default();
    }
    serde_json::from_value(s.session_options.clone()).unwrap_or_default()
}

fn live_recorder(s: &AppState) -> Result<Arc<crate::recorder::Recorder>, String> {
    match &s.recorder {
        Some(rec) if !rec.is_stopping() => Ok(rec.clone()),
        _ => Err("No session is recording".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Actions (the tray and hotkeys call these directly)
// ---------------------------------------------------------------------------

/// Start with the saved options. A session already open makes this a no-op
/// that returns its status.
pub async fn start(app: &tauri::AppHandle) -> Result<crate::commands::SessionStatus, String> {
    let al = app.state::<ActionLock>();
    let _g = al.0.lock().await;
    start_locked(app).await
}

async fn start_locked(app: &tauri::AppHandle) -> Result<crate::commands::SessionStatus, String> {
    let state = app.state::<State>();
    let (root, options) = {
        let s = state.lock().await;
        if s.recorder.as_ref().is_some_and(|r| !r.is_stopping()) {
            drop(s);
            return crate::commands::get_session_status(app.state::<State>()).await;
        }
        if s.stopping_id.is_some() {
            return Err("Still finishing the last session; try again in a moment".to_string());
        }
        (
            crate::recorder::sessions_root(&crate::capture::get_output_dir(&s.config)),
            saved_options(&s),
        )
    };
    std::fs::create_dir_all(&root).map_err(|e| format!("create {}: {}", root.display(), e))?;
    // Probing monitors and encoders and spawning every ffmpeg blocks.
    let handle = app.clone();
    let recorder = tokio::task::spawn_blocking(move || {
        let env = crate::recorder::RecorderEnv::from_app(&handle);
        crate::recorder::Recorder::start(env, &root, options)
    })
    .await
    .map_err(|e| format!("start panicked: {e}"))??;
    state.lock().await.recorder = Some(recorder);
    announce(app).await;
    crate::commands::get_session_status(app.state::<State>()).await
}

pub async fn stop(app: &tauri::AppHandle) -> Result<StopOutcome, String> {
    let al = app.state::<ActionLock>();
    let _g = al.0.lock().await;
    stop_locked(app).await
}

async fn stop_locked(app: &tauri::AppHandle) -> Result<StopOutcome, String> {
    let status = crate::commands::stop_session(app.state::<State>()).await?;
    let discard = app.state::<State>().lock().await.discard_short_sessions;
    let mut discarded = None;
    if discard && status.elapsed_ms < SHORT_SESSION_MS {
        if let Some(id) = status.session_id.clone() {
            match crate::commands::delete_session(app.state::<State>(), id).await {
                Ok(r) => discarded = Some(r),
                Err(e) => eprintln!("[chirptime] short session kept: {e}"),
            }
        }
    }
    announce(app).await;
    // Whatever stopped it (widget, tray, hotkey), the widget offers the Undo.
    if let Some(r) = &discarded {
        let _ = app.emit(DISCARDED_EVENT, r.clone());
    }
    Ok(StopOutcome { status, discarded })
}

/// Start or stop. Dropped while another action is under way.
pub async fn toggle_recording(app: &tauri::AppHandle) -> Result<(), String> {
    let al = app.state::<ActionLock>();
    let Ok(_g) = al.0.try_lock() else {
        return Ok(());
    };
    let recording = control_state(app.state::<State>().inner()).await.recording;
    if recording {
        stop_locked(app).await.map(|_| ())
    } else {
        start_locked(app).await.map(|_| ())
    }
}

pub async fn pause(app: &tauri::AppHandle) -> Result<RecorderControl, String> {
    let al = app.state::<ActionLock>();
    let _g = al.0.lock().await;
    pause_locked(app).await
}

async fn pause_locked(app: &tauri::AppHandle) -> Result<RecorderControl, String> {
    let rec = live_recorder(&*app.state::<State>().lock().await)?;
    tokio::task::spawn_blocking(move || rec.pause())
        .await
        .map_err(|e| format!("pause panicked: {e}"))??;
    announce(app).await;
    Ok(control_state(app.state::<State>().inner()).await)
}

pub async fn resume(app: &tauri::AppHandle) -> Result<RecorderControl, String> {
    let al = app.state::<ActionLock>();
    let _g = al.0.lock().await;
    resume_locked(app).await
}

async fn resume_locked(app: &tauri::AppHandle) -> Result<RecorderControl, String> {
    let rec = live_recorder(&*app.state::<State>().lock().await)?;
    let out = tokio::task::spawn_blocking(move || rec.resume())
        .await
        .map_err(|e| format!("resume panicked: {e}"))?;
    announce(app).await;
    out?;
    Ok(control_state(app.state::<State>().inner()).await)
}

/// Pause or resume. Dropped while another action is under way.
pub async fn toggle_pause(app: &tauri::AppHandle) -> Result<(), String> {
    let al = app.state::<ActionLock>();
    let Ok(_g) = al.0.try_lock() else {
        return Ok(());
    };
    let c = control_state(app.state::<State>().inner()).await;
    if !c.recording {
        return Err("No session is recording".to_string());
    }
    if c.paused_since_epoch_ms.is_some() {
        resume_locked(app).await.map(|_| ())
    } else {
        pause_locked(app).await.map(|_| ())
    }
}

/// Retry every screen that is waiting out an outage or gave up.
pub async fn restart_screens_now(app: &tauri::AppHandle) -> Result<usize, String> {
    let al = app.state::<ActionLock>();
    let _g = al.0.lock().await;
    let rec = live_recorder(&*app.state::<State>().lock().await)?;
    let n = tokio::task::spawn_blocking(move || rec.restart_screens())
        .await
        .map_err(|e| format!("restart panicked: {e}"))??;
    announce(app).await;
    Ok(n)
}

/// Mark this instant. While paused there is no footage to mark, so it refuses.
pub async fn mark(app: &tauri::AppHandle, label: String) -> Result<crate::session::Marker, String> {
    let rec = live_recorder(&*app.state::<State>().lock().await)?;
    if rec.paused_since().is_some() {
        return Err("Paused: resume to mark".to_string());
    }
    let m = rec.add_marker(label)?;
    let _ = app.emit(MARK_EVENT, m.clone());
    Ok(m)
}

/// Bring the recorder back: show it, or rebuild it from the config if it was
/// closed while idle.
pub fn show_recorder(app: &tauri::AppHandle) -> Result<(), String> {
    let w = match app.get_webview_window("main") {
        Some(w) => w,
        None => {
            let cfg = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .ok_or("No recorder window in the config")?
                .clone();
            tauri::WebviewWindowBuilder::from_config(app, &cfg)
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?
        }
    };
    w.show().map_err(|e| e.to_string())?;
    let _ = w.unminimize();
    w.set_focus().map_err(|e| e.to_string())
}

/// Open the app window on `route` (`#/settings`, `#/session/<id>`, ...).
pub fn open_app_at(app: &tauri::AppHandle, route: &str) -> Result<(), String> {
    let route = if route.starts_with("#/") { route } else { "#/library" };
    if let Some(w) = app.get_webview_window("viewer") {
        let js = format!(
            "window.location.hash = {};",
            serde_json::to_string(route).map_err(|e| e.to_string())?
        );
        w.eval(&js).map_err(|e| e.to_string())?;
        w.show().map_err(|e| e.to_string())?;
        let _ = w.unminimize();
        return w.set_focus().map_err(|e| e.to_string());
    }
    // The same window `commands::open_viewer` builds, opened on a route.
    tauri::WebviewWindowBuilder::new(
        app,
        "viewer",
        tauri::WebviewUrl::App(format!("/?mode=viewer{}", route).into()),
    )
    .title("ChirpTime")
    .inner_size(1280.0, 800.0)
    .min_inner_size(1024.0, 700.0)
    .resizable(true)
    .build()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Apply the content-protection preference to one window.
pub fn protect<R: tauri::Runtime>(app: &tauri::AppHandle<R>, w: &tauri::Window<R>) {
    let on = app
        .try_state::<WindowPrefs>()
        .map(|p| p.content_protected.load(Ordering::SeqCst))
        .unwrap_or(true);
    if let Err(e) = w.set_content_protected(on) {
        eprintln!("[chirptime] content protection not applied to {}: {e}", w.label());
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_recorder_control(state: tauri::State<'_, State>) -> Result<RecorderControl, String> {
    Ok(control_state(state.inner()).await)
}

/// Start with the saved options, as the tray and Ctrl Alt R do.
#[tauri::command]
pub async fn start_recording(app: tauri::AppHandle) -> Result<crate::commands::SessionStatus, String> {
    start(&app).await
}

/// Stop, then discard the session if it ran under a minute and the setting is on.
#[tauri::command]
pub async fn stop_recording(app: tauri::AppHandle) -> Result<StopOutcome, String> {
    stop(&app).await
}

#[tauri::command]
pub async fn pause_session(app: tauri::AppHandle) -> Result<RecorderControl, String> {
    pause(&app).await
}

#[tauri::command]
pub async fn resume_session(app: tauri::AppHandle) -> Result<RecorderControl, String> {
    resume(&app).await
}

/// Retry every screen that is waiting out an outage or gave up.
#[tauri::command]
pub async fn restart_screens(app: tauri::AppHandle) -> Result<usize, String> {
    restart_screens_now(&app).await
}

#[tauri::command]
pub async fn mark_now(app: tauri::AppHandle, label: Option<String>) -> Result<crate::session::Marker, String> {
    mark(&app, label.unwrap_or_default()).await
}

/// The note on a mark of the session that is recording now.
#[tauri::command]
pub async fn set_live_marker_label(
    state: tauri::State<'_, State>,
    epoch_ms: i64,
    label: String,
) -> Result<crate::session::Marker, String> {
    let rec = live_recorder(&*state.lock().await)?;
    rec.set_marker_label(epoch_ms, &label)
}

#[tauri::command]
pub async fn open_app(app: tauri::AppHandle, route: Option<String>) -> Result<(), String> {
    open_app_at(&app, route.as_deref().unwrap_or("#/library"))
}

#[tauri::command]
pub async fn show_recorder_window(app: tauri::AppHandle) -> Result<(), String> {
    show_recorder(&app)
}

#[tauri::command]
pub async fn set_content_protection(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
    enabled: bool,
) -> Result<(), String> {
    {
        let mut s = state.lock().await;
        s.content_protection = enabled;
        persist(&s);
    }
    app.state::<WindowPrefs>()
        .content_protected
        .store(enabled, Ordering::SeqCst);
    for w in app.webview_windows().values() {
        if let Err(e) = w.set_content_protected(enabled) {
            eprintln!("[chirptime] content protection not applied to {}: {e}", w.label());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_discard_short_sessions(state: tauri::State<'_, State>, enabled: bool) -> Result<(), String> {
    let mut s = state.lock().await;
    s.discard_short_sessions = enabled;
    persist(&s);
    Ok(())
}
