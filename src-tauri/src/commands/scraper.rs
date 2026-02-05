use keyring::Entry;
use tauri::Manager;
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

use crate::scraper::types::FilamentSpecs;

/// Get the configured AI provider from preferences, defaulting to "claude".
fn get_ai_provider(app: &tauri::AppHandle) -> Result<String, String> {
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open preferences store: {}", e);
        e.to_string()
    })?;
    let provider = store
        .get("ai_provider")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "claude".to_string());
    Ok(provider)
}

/// Get the configured AI model from preferences, defaulting to "claude-sonnet-4-20250514".
fn get_ai_model(app: &tauri::AppHandle) -> Result<String, String> {
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open preferences store: {}", e);
        e.to_string()
    })?;
    let model = store
        .get("ai_model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
    Ok(model)
}

/// Get the API key from the system keychain for the given provider.
fn get_api_key_for_provider(provider: &str) -> Result<String, String> {
    let service = match provider {
        "claude" => "bambumate-claude-api",
        "openai" => "bambumate-openai-api",
        "kimi" => "bambumate-kimi-api",
        "openrouter" => "bambumate-openrouter-api",
        _ => return Err(format!("Unknown AI provider: '{}'. Supported: claude, openai, kimi, openrouter", provider)),
    };
    let entry = Entry::new(service, "bambumate").map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => {
            Err(format!("No API key configured for '{}'. Please set it in Settings.", provider))
        }
        Err(e) => Err(format!("Failed to read API key for '{}': {}", provider, e)),
    }
}

/// Get the cache directory for the app, creating it if needed.
fn get_cache_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))?;
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create app data directory: {}", e))?;
    Ok(cache_dir)
}

/// Search for filament specifications by name.
/// Checks the cache first; if not cached, fetches from the web using
/// the configured AI provider for spec extraction.
#[tauri::command]
pub async fn search_filament(
    app: tauri::AppHandle,
    filament_name: String,
) -> Result<FilamentSpecs, String> {
    info!("search_filament called for: {}", filament_name);

    let provider = get_ai_provider(&app)?;
    let model = get_ai_model(&app)?;
    let api_key = get_api_key_for_provider(&provider)?;
    let cache_dir = get_cache_dir(&app)?;

    info!(
        "Using AI provider '{}' model '{}' for extraction",
        provider, model
    );

    crate::scraper::search_filament(&filament_name, &provider, &model, &api_key, &cache_dir).await
}

/// Look up cached filament specs without any network requests.
/// Returns null if the filament is not cached or the cache has expired.
#[tauri::command]
pub async fn get_cached_filament(
    app: tauri::AppHandle,
    filament_name: String,
) -> Result<Option<FilamentSpecs>, String> {
    info!("get_cached_filament called for: {}", filament_name);

    let cache_dir = get_cache_dir(&app)?;
    crate::scraper::search_filament_cached_only(&filament_name, &cache_dir).await
}

/// Clear expired entries from the filament specification cache.
/// Returns the number of entries removed.
#[tauri::command]
pub async fn clear_filament_cache(app: tauri::AppHandle) -> Result<usize, String> {
    info!("clear_filament_cache called");

    let cache_dir = get_cache_dir(&app)?;
    crate::scraper::clear_expired_cache(&cache_dir).await
}
