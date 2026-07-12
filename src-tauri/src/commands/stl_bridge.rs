use tauri::Manager;
use tracing::info;

use crate::stl_watcher::{StlFile, StlWatcherState};

fn lock_recover<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Set the STL watch directory and start watching.
#[tauri::command]
pub async fn set_stl_watch_dir(app: tauri::AppHandle, path: String) -> Result<(), String> {
    info!("Setting STL watch directory to: {}", path);

    // Save preference
    use tauri_plugin_store::StoreExt;
    if let Ok(store) = app.store("preferences.json") {
        store.set("stl_watch_dir", serde_json::Value::String(path.clone()));
    }

    // Start watching
    let state = app.state::<StlWatcherState>();
    state.start_watching(&path)?;

    Ok(())
}

/// Get the current STL watch directory.
#[tauri::command]
pub async fn get_stl_watch_dir(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let state = app.state::<StlWatcherState>();
    let dir = lock_recover(&state.watch_dir).clone();
    Ok(dir)
}

/// List all received STL files.
#[tauri::command]
pub async fn list_received_stls(app: tauri::AppHandle) -> Result<Vec<StlFile>, String> {
    let state = app.state::<StlWatcherState>();
    let files = lock_recover(&state.received_files).snapshot();
    Ok(files)
}

/// Clear all received STL files.
#[tauri::command]
pub async fn clear_received_stls(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<StlWatcherState>();
    lock_recover(&state.received_files).clear();
    Ok(())
}

/// Dismiss a single STL file from the received list.
#[tauri::command]
pub async fn dismiss_stl(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let state = app.state::<StlWatcherState>();
    lock_recover(&state.received_files).remove(&path);
    Ok(())
}
