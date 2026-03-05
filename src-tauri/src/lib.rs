mod capture;
mod commands;
mod fileserver;
mod state;

use state::AppState;
use std::sync::Arc;
use tokio::sync::Mutex;

const FILE_SERVER_PORT: u16 = 9120;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
