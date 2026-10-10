mod capture;
mod commands;
// Recorder actions shared by the widget, the tray and the hotkeys (S3).
mod controls;
pub mod cursor;
pub mod export;
// Public so the cut / undo / finalize paths can be exercised on synthetic
// sessions in `tests/edit_cuts.rs`.
pub mod edit;
// Public so the pin (URL + SHA-256) and the install layout can be asserted
// without a window system.
pub mod ffmpeg_install;
mod fileserver;
mod hotkeys;
mod nativeui;
// Public so the settings round-trip can be exercised without a window system.
pub mod settings;
// Public so the live hardware tests in `tests/live_session.rs` can drive the
// recorder headlessly, without a window system.
pub mod recorder;
pub mod session;
pub mod state;
mod sysaudio;
mod tray;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

const FILE_SERVER_PORT: u16 = 9120;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before Tauri builds any window and before anything reads a monitor rect.
    // Without this the whole process sees DPI-virtualised coordinates — including
    // `available_monitors()` and DXGI — and every rectangle the recorder writes
    // into a session manifest is silently scaled. See
    // `recorder::ensure_process_dpi_aware`.
    if let Err(e) = recorder::ensure_process_dpi_aware() {
        eprintln!("[chirptime] {}", e);
    }

    // Settings are read before anything else, and never fail: a missing or
    // corrupt file degrades to defaults. The file server thread below resolves
    // media paths against the output dir, so it has to be the saved one from the
    // first request, not a default that gets corrected later.
    let saved = settings::load();
    eprintln!(
        "[chirptime] settings: {} · recordings: {}",
        settings::settings_path().display(),
        saved.capture.output_dir
    );
    // Finish deletes a previous run left half done (crash recovery for
    // `edit.rs`). Nothing can be recording yet, and it never blocks startup.
    let recovery_root = recorder::sessions_root(&capture::get_output_dir(&saved.capture));
    std::thread::spawn(move || {
        // Cuts still inside their undo window get the finalize their timer
        // would have run.
        let young = edit::recover_sessions(&recovery_root);
        if !young.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(edit::FINALIZE_AFTER_MS));
            for dir in young {
                if let Err(e) = edit::finalize_due_cuts(&dir) {
                    eprintln!("[chirptime] edit: finalize failed: {}", e);
                }
            }
        }
    });

    let hotkeys = saved.hotkeys.clone();
    let window_prefs = controls::WindowPrefs {
        content_protected: std::sync::atomic::AtomicBool::new(saved.content_protection),
    };
    let app_state = Arc::new(Mutex::new(AppState::from_settings(saved)));

    // Start file server in background
    let server_state = app_state.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(fileserver::start(server_state, FILE_SERVER_PORT));
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(hotkeys::plugin())
        // Content protection for every window as it is created: the recorder
        // from the config, and the app window `open_viewer` builds later.
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("chirptime-windows")
                .on_window_ready(|w| controls::protect(w.app_handle(), &w))
                .build(),
        )
        .manage(app_state)
        .manage(window_prefs)
        .manage(hotkeys::HotkeyState::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            // Neither may keep the app from starting: a tray or a hotkey that
            // fails is reported, and recording still works from the widget.
            if let Err(e) = tray::init(&handle) {
                eprintln!("[chirptime] tray not created: {e}");
            }
            hotkeys::register(&handle, &hotkeys);
            Ok(())
        })
        // Closing the recorder while a session runs hides it to the tray; the
        // recording goes on. Idle, it closes as before.
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<Arc<Mutex<AppState>>>();
                // Contended means something is mid-change: keep it running.
                let recording = state
                    .try_lock()
                    .map(|s| s.recorder.as_ref().is_some_and(|r| !r.is_stopping()))
                    .unwrap_or(true);
                if recording {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_dates,
            commands::get_date_entries,
            commands::get_monitor_config,
            commands::save_monitor_config,
            commands::delete_entry,
            commands::delete_file,
            commands::get_output_dir,
            commands::set_output_dir,
            commands::get_capture_config,
            commands::set_capture_config,
            commands::start_capture,
            commands::stop_capture,
            commands::get_capture_status,
            commands::run_single_capture,
            commands::get_ffmpeg_path,
            commands::list_devices,
            commands::open_viewer,
            // Settings persistence + first-run onboarding
            commands::get_settings,
            commands::set_session_options,
            commands::set_onboarded,
            commands::get_environment,
            commands::pick_output_dir,
            commands::open_folder,
            // First-run FFmpeg install
            commands::get_ffmpeg_setup,
            commands::install_ffmpeg,
            commands::pick_ffmpeg_dir,
            commands::clear_ffmpeg_dir,
            // Continuous session recording
            commands::start_session,
            commands::stop_session,
            commands::get_session_status,
            commands::list_sessions,
            commands::get_session,
            commands::add_marker,
            commands::rebuild_session,
            commands::list_display_outputs,
            // Deleting time
            commands::delete_range,
            commands::undo_cut,
            commands::finalize_cuts,
            commands::delete_session,
            commands::undo_delete_session,
            // Marks (S6, owner: library)
            commands::set_marker_label,
            // Cursor track (S3, owner: widget)
            cursor::get_cursor_track,
            // Recorder controls, tray and hotkeys (S3, owner: widget)
            controls::get_recorder_control,
            controls::start_recording,
            controls::stop_recording,
            controls::pause_session,
            controls::resume_session,
            controls::restart_screens,
            controls::mark_now,
            controls::set_live_marker_label,
            controls::open_app,
            controls::show_recorder_window,
            controls::set_content_protection,
            controls::set_discard_short_sessions,
            hotkeys::get_hotkeys,
            hotkeys::set_hotkeys,
            // Export composer (S5, owner: export)
            export::estimate_export,
            export::start_export,
            export::cancel_export,
            export::list_exports,
            export::delete_export,
            export::reveal_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
