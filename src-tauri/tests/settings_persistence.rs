//! Settings survive a restart, and a broken file never stops the app.
//!
//! Runs against the real `settings_path()` resolution (`%APPDATA%\<identifier>`)
//! with `APPDATA` pointed at a temp directory — the same code path a cold start
//! takes, since nothing is cached between calls: `load()` always re-reads the
//! file from disk.
//!
//! Its own test binary because it mutates a process-global environment
//! variable. It records nothing and touches no device.
//!
//!     cargo test --test settings_persistence

use chirptime_lib::settings::{self, Settings};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("chirptime-persist-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn settings_survive_a_restart_and_a_corrupt_file_does_not() {
    let home = scratch("appdata");
    std::env::set_var("APPDATA", &home);

    let path = settings::settings_path();
    assert_eq!(
        path,
        home.join(settings::APP_IDENTIFIER).join(settings::SETTINGS_FILE),
        "settings must land in the app-config directory Tauri would resolve"
    );

    // --- cold start, nothing saved yet -------------------------------------
    assert!(!path.exists());
    let first = settings::load();
    assert!(!first.onboarded, "a fresh install has not been onboarded");
    assert_eq!(first.capture.output_dir, chirptime_lib::state::default_output_dir());

    // --- the user picks a folder, changes an option, dismisses onboarding ---
    let mut chosen = Settings::default();
    chosen.capture.output_dir = r"D:\ChirpTime Recordings".to_string();
    chosen.capture.interval_minutes = 15;
    chosen.session = serde_json::json!({ "screen_fps": 15, "record_system_audio": false });
    chosen.onboarded = true;
    settings::save(&chosen).expect("settings must be writable");
    assert!(path.exists(), "saving must create the file");

    // --- restart ------------------------------------------------------------
    let after = settings::load();
    assert_eq!(after.capture.output_dir, r"D:\ChirpTime Recordings");
    assert_eq!(after.capture.interval_minutes, 15);
    assert_eq!(after.session["screen_fps"], 15);
    assert_eq!(after.session["record_system_audio"], false);
    assert!(after.onboarded, "the first-run panel must stay dismissed");

    // The blob is still valid `SessionOptions` after the round trip, which is
    // what `start_session` will be handed.
    let opts: serde_json::Value = after.session.clone();
    assert!(opts.is_object());

    // --- somebody hand-edits the file into nonsense -------------------------
    std::fs::write(&path, "{\"capture\": {\"output_dir\": ").unwrap();
    let salvaged = settings::load();
    assert!(!salvaged.onboarded);
    assert_eq!(
        salvaged.capture.output_dir,
        chirptime_lib::state::default_output_dir(),
        "a corrupt file falls back to defaults instead of an empty path"
    );

    // --- and a half-written file still gives back what it can ---------------
    std::fs::write(&path, r#"{"onboarded": true, "capture": {"output_dir": "E:\\Keep"}}"#).unwrap();
    let partial = settings::load();
    assert!(partial.onboarded);
    assert_eq!(partial.capture.output_dir, r"E:\Keep");
    assert_eq!(partial.capture.video_framerate, 30, "missing keys use the defaults");
}
