//! Global hotkeys: Ctrl Alt R starts and stops, Ctrl Alt M marks, Ctrl Alt P
//! pauses. Registered from Rust (tauri-plugin-global-shortcut), configurable in
//! Settings. A combination another app already holds is reported per hotkey in
//! Settings; it never stops the app from starting.

use crate::controls;
use crate::settings::Hotkeys;
use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tokio::sync::Mutex;

type State = Arc<Mutex<AppState>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Record,
    Mark,
    Pause,
}

/// One hotkey as it stands: what was asked for, and whether Windows gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HotkeyStatus {
    pub action: Action,
    /// As saved, e.g. `Ctrl+Alt+R`. Empty means turned off.
    pub accelerator: String,
    pub registered: bool,
    /// Why it is not registered, in words.
    pub error: Option<String>,
}

/// What is registered right now, keyed by the plugin's shortcut id.
#[derive(Default)]
pub struct HotkeyState {
    bound: std::sync::Mutex<Vec<(u32, Action)>>,
    status: std::sync::Mutex<Vec<HotkeyStatus>>,
}

/// The accelerators `hotkeys` asks for, in a fixed order, each parsed.
/// Pure, for the tests: a malformed or duplicated one is an error for that
/// action only.
pub fn plan(hotkeys: &Hotkeys) -> Vec<(Action, String, Result<Shortcut, String>)> {
    let wanted = [
        (Action::Record, hotkeys.record.trim()),
        (Action::Mark, hotkeys.mark.trim()),
        (Action::Pause, hotkeys.pause.trim()),
    ];
    let mut seen: Vec<u32> = Vec::new();
    wanted
        .iter()
        .map(|(action, acc)| {
            let parsed = if acc.is_empty() {
                Err("Off".to_string())
            } else {
                acc.parse::<Shortcut>()
                    .map_err(|_| format!("{} is not a key combination ChirpTime understands", acc))
                    .and_then(|s| {
                        if seen.contains(&s.id()) {
                            Err(format!("{} is already used by another ChirpTime hotkey", acc))
                        } else {
                            seen.push(s.id());
                            Ok(s)
                        }
                    })
            };
            (*action, acc.to_string(), parsed)
        })
        .collect()
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::<tauri::Wry>::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let Some(hs) = app.try_state::<HotkeyState>() else {
                return;
            };
            let action = hs
                .bound
                .lock()
                .unwrap()
                .iter()
                .find(|(id, _)| *id == shortcut.id())
                .map(|(_, a)| *a);
            if let Some(a) = action {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { run(&app, a).await });
            }
        })
        .build()
}

async fn run(app: &AppHandle, action: Action) {
    let result = match action {
        Action::Record => controls::toggle_recording(app).await,
        Action::Mark => controls::mark(app, String::new()).await.map(|_| ()),
        Action::Pause => controls::toggle_pause(app).await,
    };
    if let Err(e) = result {
        crate::tray::toast(app, "ChirpTime", &e);
    }
}

/// Drop every ChirpTime hotkey and register `hotkeys` afresh. Never fails:
/// each hotkey's outcome lands in [`HotkeyState`].
pub fn register(app: &AppHandle, hotkeys: &Hotkeys) -> Vec<HotkeyStatus> {
    let gs = app.global_shortcut();
    if let Err(e) = gs.unregister_all() {
        eprintln!("[chirptime] hotkeys: unregister failed: {e}");
    }
    let mut bound = Vec::new();
    let mut status = Vec::new();
    for (action, accelerator, parsed) in plan(hotkeys) {
        let (registered, error) = match parsed {
            Err(_) if accelerator.is_empty() => (false, None),
            Err(e) => (false, Some(e)),
            Ok(s) => match (s.id(), gs.register(s)) {
                (id, Ok(())) => {
                    bound.push((id, action));
                    (true, None)
                }
                (_, Err(e)) => {
                    eprintln!("[chirptime] hotkeys: {accelerator}: {e}");
                    (
                        false,
                        Some(format!("{} is taken by another app", accelerator.replace('+', " "))),
                    )
                }
            },
        };
        status.push(HotkeyStatus {
            action,
            accelerator,
            registered,
            error,
        });
    }
    let hs = app.state::<HotkeyState>();
    *hs.bound.lock().unwrap() = bound;
    *hs.status.lock().unwrap() = status.clone();
    status
}

#[tauri::command]
pub async fn get_hotkeys(app: AppHandle) -> Result<Vec<HotkeyStatus>, String> {
    Ok(app.state::<HotkeyState>().status.lock().unwrap().clone())
}

/// Save new hotkeys and re-register them. Returns each one's outcome; a
/// combination that could not be registered is still saved, so it works once
/// the other app lets go of it (at the next start).
#[tauri::command]
pub async fn set_hotkeys(
    app: AppHandle,
    state: tauri::State<'_, State>,
    hotkeys: Hotkeys,
) -> Result<Vec<HotkeyStatus>, String> {
    {
        let mut s = state.lock().await;
        s.hotkeys = hotkeys.clone();
        controls::persist(&s);
    }
    let out = register(&app, &hotkeys);
    controls::announce(&app).await;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(r: &str, m: &str, p: &str) -> Hotkeys {
        Hotkeys {
            record: r.into(),
            mark: m.into(),
            pause: p.into(),
        }
    }

    #[test]
    fn the_defaults_parse() {
        let p = plan(&Hotkeys::default());
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|(_, _, s)| s.is_ok()), "{p:?}");
        assert_eq!(p[0].0, Action::Record);
    }

    #[test]
    fn off_bad_and_duplicate_are_per_action() {
        let p = plan(&keys("Ctrl+Alt+R", "", "Ctrl+Alt+R"));
        assert!(p[0].2.is_ok());
        assert_eq!(p[1].2.as_ref().unwrap_err(), "Off");
        assert!(p[2].2.as_ref().unwrap_err().contains("already used"));
        let p = plan(&keys("Ctrl+Alt+Nope", "Ctrl+Shift+F9", "Alt+P"));
        assert!(p[0].2.as_ref().unwrap_err().contains("not a key combination"));
        assert!(p[1].2.is_ok() && p[2].2.is_ok());
    }
}
