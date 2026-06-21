use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

/// Feature flags indicating which modules are enabled.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureFlags {
    pub profiles_enabled: bool,
    pub analysis_enabled: bool,
}

#[tauri::command]
pub fn get_preference(app: AppHandle, key: &str) -> Result<Option<String>, String> {
    info!("Getting preference: {}", key);
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open store: {}", e);
        e.to_string()
    })?;
    let value = store.get(key).and_then(|v| v.as_str().map(|s| s.to_string()));
    Ok(value)
}

#[tauri::command]
pub fn set_preference(app: AppHandle, key: &str, value: &str) -> Result<(), String> {
    info!("Setting preference: {} = {}", key, value);
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open store: {}", e);
        e.to_string()
    })?;
    store.set(key, serde_json::json!(value));
    store.save().map_err(|e| {
        warn!("Failed to save store: {}", e);
        e.to_string()
    })
}

/// Returns the current feature flags. Both modules are enabled by default.
#[tauri::command]
pub fn get_feature_flags(app: AppHandle) -> Result<FeatureFlags, String> {
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open store: {}", e);
        e.to_string()
    })?;

    let profiles_enabled = store
        .get("feature_profiles_enabled")
        .and_then(|v| v.as_str().map(|s| s != "false"))
        .unwrap_or(true);

    let analysis_enabled = store
        .get("feature_analysis_enabled")
        .and_then(|v| v.as_str().map(|s| s != "false"))
        .unwrap_or(true);

    Ok(FeatureFlags {
        profiles_enabled,
        analysis_enabled,
    })
}
