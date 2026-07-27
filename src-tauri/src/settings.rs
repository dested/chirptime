//! Persisted user settings.
//!
//! Everything the user chooses in the widget — where recordings are written,
//! the interval-clip config, the session recorder's options, and whether the
//! first-run panel has been dismissed — lives in one JSON file at
//! `%APPDATA%\com.chirptime.app\settings.json`. That is the same location
//! Tauri's own `app_config_dir()` resolves to on Windows; it is computed here
//! without an `AppHandle` because settings are loaded in `run()` *before* the
//! Tauri app (and the file server thread) exist, and both need the output dir.
//!
//! **Loading never fails.** A missing, truncated, hand-edited or older file
//! must not stop the app from starting — every one of those cases degrades to
//! the built-in defaults, per field where possible, and says so on stderr.

use crate::state::CaptureConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Must match `identifier` in `tauri.conf.json` — that is what makes this the
/// same directory `app_config_dir()` would hand back.
pub const APP_IDENTIFIER: &str = "com.chirptime.app";
pub const SETTINGS_FILE: &str = "settings.json";
/// Bumped only for a change old files cannot be read through. Unknown versions
/// are read best-effort rather than discarded.
pub const SETTINGS_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub version: u32,
    pub capture: CaptureConfig,
    /// `recorder::SessionOptions` as raw JSON. It is stored opaquely because
    /// that type is deserialize-only and lives in the recording pipeline; this
    /// module validates the blob by parsing it and then hands it back to the
    /// frontend byte for byte. `Null` means "never configured — use defaults".
    pub session: serde_json::Value,
    /// True once the first-run panel has been dismissed. The whole point of
    /// persisting anything is that this stays true.
    pub onboarded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            capture: CaptureConfig::default(),
            session: serde_json::Value::Null,
            onboarded: false,
        }
    }
}

/// `%APPDATA%\com.chirptime.app`, falling back to `~/.config/...` off-Windows
/// and finally to a relative directory so this can never panic.
pub fn settings_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let appdata = PathBuf::from(appdata);
        if !appdata.as_os_str().is_empty() {
            return appdata.join(APP_IDENTIFIER);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if !home.as_os_str().is_empty() {
            return home.join(".config").join(APP_IDENTIFIER);
        }
    }
    PathBuf::from(APP_IDENTIFIER)
}

pub fn settings_path() -> PathBuf {
    settings_dir().join(SETTINGS_FILE)
}

pub fn load() -> Settings {
    load_from(&settings_path())
}

pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&settings_path(), settings)
}

/// Read settings from `path`, degrading to defaults for anything unreadable.
///
/// Deliberately field-by-field: a file written by a newer build, or one where a
/// single key has been mangled, still yields every *other* setting the user
/// chose. Losing the output dir because an unrelated key is bad would send
/// hours of recording somewhere they did not ask for.
pub fn load_from(path: &Path) -> Settings {
    let mut out = Settings::default();

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        // Not an error: no file simply means this is the first run.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            eprintln!("[chirptime] settings unreadable ({}): {e}", path.display());
            return out;
        }
    };

    let root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[chirptime] settings are not valid JSON, using defaults: {e}");
            return out;
        }
    };
    let Some(map) = root.as_object() else {
        eprintln!("[chirptime] settings are not a JSON object, using defaults");
        return out;
    };

    if let Some(v) = map.get("version").and_then(|v| v.as_u64()) {
        out.version = v as u32;
    }

    if let Some(v) = map.get("capture") {
        match serde_json::from_value::<CaptureConfig>(v.clone()) {
            Ok(c) => out.capture = c,
            Err(e) => eprintln!("[chirptime] settings.capture ignored: {e}"),
        }
    }

    // An empty or whitespace-only path would resolve to the process cwd, which
    // is never what anyone meant by it.
    if out.capture.output_dir.trim().is_empty() {
        out.capture.output_dir = crate::state::default_output_dir();
    }

    if let Some(v) = map.get("session") {
        if v.is_null() {
            // Explicitly "not configured" — leave the default.
        } else if serde_json::from_value::<crate::recorder::SessionOptions>(v.clone()).is_ok() {
            out.session = v.clone();
        } else {
            eprintln!("[chirptime] settings.session ignored: not valid SessionOptions");
        }
    }

    if let Some(v) = map.get("onboarded").and_then(|v| v.as_bool()) {
        out.onboarded = v;
    }

    out
}

/// Write settings to `path` via a temp file + rename, so a crash mid-write can
/// never leave a half-written file where a readable one used to be.
pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    // std::fs::rename replaces an existing destination on Windows.
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Legacy output directory
// ---------------------------------------------------------------------------

/// A pre-existing `output/` the app is no longer looking at.
#[derive(Debug, Clone, Serialize)]
pub struct LegacyOutput {
    pub path: String,
    /// `YYYY-MM-DD` directories of interval clips.
    pub day_count: usize,
    /// Directories under `sessions/`.
    pub session_count: usize,
}

fn looks_like_date_dir(name: &str) -> bool {
    name.len() == 10
        && name.as_bytes()[4] == b'-'
        && name.as_bytes()[7] == b'-'
        && name.chars().filter(|c| c.is_ascii_digit()).count() == 8
}

/// Count what an `output/` directory holds. `None` when it holds nothing this
/// app would have written — an empty or unrelated directory is not a find.
fn inspect_output_dir(dir: &Path) -> Option<LegacyOutput> {
    let mut day_count = 0usize;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if looks_like_date_dir(&entry.file_name().to_string_lossy()) {
            day_count += 1;
        }
    }
    let session_count = std::fs::read_dir(dir.join("sessions"))
        .map(|it| {
            it.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0);

    if day_count == 0 && session_count == 0 {
        return None;
    }
    Some(LegacyOutput {
        path: dir.to_string_lossy().to_string(),
        day_count,
        session_count,
    })
}

/// Look for a non-empty `output/` beside the executable or the repo that is not
/// the directory currently configured.
///
/// This exists because the default output dir moved: anyone who recorded before
/// that change has footage the app will never list again. Nothing here moves,
/// copies or deletes a byte — it reports a path so the user can decide.
pub fn find_legacy_output(configured: &Path) -> Option<LegacyOutput> {
    let configured_key = normalize(configured);

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        // Installed: `<install>\chirptime.exe`. Dev: `<repo>\src-tauri\target\debug\chirptime.exe`,
        // so the repo root is three levels up.
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        for _ in 0..4 {
            let Some(d) = dir else { break };
            roots.push(d.clone());
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.clone());
        if let Some(parent) = cwd.parent() {
            roots.push(parent.to_path_buf());
        }
    }

    let mut seen: Vec<String> = Vec::new();
    for root in roots {
        let candidate = root.join("output");
        let key = normalize(&candidate);
        if key == configured_key || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if let Some(found) = inspect_output_dir(&candidate) {
            return Some(found);
        }
    }
    None
}

/// Comparable form of a path: canonicalized when it exists, lowercased, with the
/// `\\?\` prefix `canonicalize` adds on Windows stripped off.
fn normalize(path: &Path) -> String {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = p.to_string_lossy().replace('/', "\\");
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_lowercase()
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("chirptime-settings-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrips() {
        let dir = tmpdir("roundtrip");
        let path = dir.join("settings.json");

        let mut s = Settings::default();
        s.capture.output_dir = r"D:\Recordings\ChirpTime".to_string();
        s.capture.interval_minutes = 3;
        s.onboarded = true;
        s.session = serde_json::json!({ "screen_fps": 15, "record_webcam": false });
        save_to(&path, &s).unwrap();

        let back = load_from(&path);
        assert_eq!(back.capture.output_dir, r"D:\Recordings\ChirpTime");
        assert_eq!(back.capture.interval_minutes, 3);
        assert!(back.onboarded);
        assert_eq!(back.session["screen_fps"], 15);
        assert_eq!(back.version, SETTINGS_VERSION);
    }

    #[test]
    fn missing_file_is_defaults_not_an_error() {
        let dir = tmpdir("missing");
        let s = load_from(&dir.join("nope.json"));
        assert!(!s.onboarded);
        assert_eq!(s.capture.output_dir, crate::state::default_output_dir());
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = tmpdir("corrupt");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ this is not json at all ]]").unwrap();
        let s = load_from(&path);
        assert!(!s.onboarded);
        assert_eq!(s.capture.output_dir, crate::state::default_output_dir());
        assert!(s.session.is_null());
    }

    #[test]
    fn a_bad_field_does_not_cost_the_good_ones() {
        let dir = tmpdir("partial");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "version": 99,
                "capture": { "output_dir": "E:\\Keep", "interval_minutes": 7 },
                "session": "this should have been an object",
                "onboarded": true,
                "some_future_key": [1, 2, 3]
            }"#,
        )
        .unwrap();
        let s = load_from(&path);
        // The readable half survives...
        assert_eq!(s.capture.output_dir, r"E:\Keep");
        assert_eq!(s.capture.interval_minutes, 7);
        assert!(s.onboarded);
        assert_eq!(s.version, 99);
        // ...and the unreadable half is simply absent.
        assert!(s.session.is_null());
        // Fields the old file never had come from Default.
        assert_eq!(s.capture.video_framerate, CaptureConfig::default().video_framerate);
    }

    #[test]
    fn empty_output_dir_is_replaced_by_the_default() {
        let dir = tmpdir("emptydir");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"capture":{"output_dir":"   "}}"#).unwrap();
        assert_eq!(load_from(&path).capture.output_dir, crate::state::default_output_dir());
    }

    #[test]
    fn session_blob_must_be_valid_options() {
        let dir = tmpdir("session");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"session":{"screen_fps":"sixty"}}"#).unwrap();
        assert!(load_from(&path).session.is_null());
    }

    #[test]
    fn save_replaces_an_existing_file() {
        let dir = tmpdir("replace");
        let path = dir.join("settings.json");
        save_to(&path, &Settings::default()).unwrap();
        let mut s = Settings::default();
        s.onboarded = true;
        save_to(&path, &s).unwrap();
        assert!(load_from(&path).onboarded);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn an_empty_directory_is_not_a_legacy_find() {
        let dir = tmpdir("legacy-empty");
        std::fs::create_dir_all(dir.join("output")).unwrap();
        assert!(inspect_output_dir(&dir.join("output")).is_none());
    }

    #[test]
    fn counts_days_and_sessions() {
        let dir = tmpdir("legacy-full");
        let out = dir.join("output");
        std::fs::create_dir_all(out.join("2026-03-02")).unwrap();
        std::fs::create_dir_all(out.join("2026-03-03")).unwrap();
        std::fs::create_dir_all(out.join("not-a-date")).unwrap();
        std::fs::create_dir_all(out.join("sessions").join("2026-03-02_10-00-00")).unwrap();
        let found = inspect_output_dir(&out).unwrap();
        assert_eq!(found.day_count, 2);
        assert_eq!(found.session_count, 1);
    }

    #[test]
    fn the_configured_dir_is_never_reported_as_legacy() {
        let dir = tmpdir("legacy-self");
        let out = dir.join("output");
        std::fs::create_dir_all(out.join("2026-03-02")).unwrap();
        // Same directory, spelled differently — normalization has to catch it.
        assert_eq!(normalize(&out), normalize(&dir.join("output/")));
    }
}
