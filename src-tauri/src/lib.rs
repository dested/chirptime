mod capture;
mod commands;
// Public so the cut / undo / finalize paths can be exercised on synthetic
// sessions in `tests/edit_cuts.rs`.
pub mod edit;
// Public so the pin (URL + SHA-256) and the install layout can be asserted
// without a window system.
pub mod ffmpeg_install;
mod fileserver;
mod nativeui;
// Public so the settings round-trip can be exercised without a window system.
pub mod settings;
// Public so the live hardware tests in `tests/live_session.rs` can drive the
// recorder headlessly, without a window system.
pub mod recorder;
pub mod session;
pub mod state;
mod sysaudio;

use state::AppState;
use std::sync::Arc;
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
    std::thread::spawn(move || edit::recover_sessions(&recovery_root));

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
        .manage(app_state)
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
