use tauri::Manager;
use tracing::info;

use crate::stl_watcher::{StlFile, StlWatcherState};

/// Set the STL watch directory and start watching.
#[tauri::command]
pub async fn set_stl_watch_dir(
    app: tauri::AppHandle,
    path: String,
) -> Result<(), String> {
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
    let dir = state.watch_dir.lock().unwrap().clone();
    Ok(dir)
}

/// List all received STL files.
#[tauri::command]
pub async fn list_received_stls(app: tauri::AppHandle) -> Result<Vec<StlFile>, String> {
    let state = app.state::<StlWatcherState>();
    let files = state.received_files.lock().unwrap().clone();
    Ok(files)
}

/// Clear all received STL files.
#[tauri::command]
pub async fn clear_received_stls(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<StlWatcherState>();
    state.received_files.lock().unwrap().clear();
    Ok(())
}

/// Dismiss a single STL file from the received list.
#[tauri::command]
pub async fn dismiss_stl(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let state = app.state::<StlWatcherState>();
    let mut files = state.received_files.lock().unwrap();
    files.retain(|f| f.path != path);
    Ok(())
}
