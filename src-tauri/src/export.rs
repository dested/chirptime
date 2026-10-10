//! Export composer backend (S5). Lead pre-wire: the command names below are
//! registered in lib.rs; `export` owns this file and replaces the stubs.

const NOT_BUILT: &str = "Export is not built yet";

#[tauri::command]
pub async fn estimate_export(req: serde_json::Value) -> Result<serde_json::Value, String> {
    let _ = req;
    Err(NOT_BUILT.into())
}

#[tauri::command]
pub async fn start_export(req: serde_json::Value) -> Result<String, String> {
    let _ = req;
    Err(NOT_BUILT.into())
}

#[tauri::command]
pub async fn cancel_export(job_id: String) -> Result<(), String> {
    let _ = job_id;
    Err(NOT_BUILT.into())
}

#[tauri::command]
pub async fn list_exports() -> Result<Vec<serde_json::Value>, String> {
    Ok(Vec::new())
}

#[tauri::command]
pub async fn delete_export(id: String) -> Result<(), String> {
    let _ = id;
    Err(NOT_BUILT.into())
}

#[tauri::command]
pub async fn reveal_export(id: String) -> Result<(), String> {
    let _ = id;
    Err(NOT_BUILT.into())
}
