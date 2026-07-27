mod capture;
mod commands;
mod fileserver;
// Public so the live hardware tests in `tests/live_session.rs` can drive the
// recorder headlessly, without a window system.
pub mod recorder;
pub mod session;
mod state;
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

    let app_state = Arc::new(Mutex::new(AppState::default()));

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
            // Continuous session recording
            commands::start_session,
            commands::stop_session,
            commands::get_session_status,
            commands::list_sessions,
            commands::get_session,
            commands::add_marker,
            commands::rebuild_session,
            commands::list_display_outputs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
