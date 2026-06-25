use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

/// Status of the initial setup wizard.
#[derive(Debug, Clone, Serialize)]
pub struct SetupStatus {
    pub bambu_studio_path: Option<String>,
    pub ai_provider: Option<String>,
    pub has_api_key: bool,
    pub setup_complete: bool,
}

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

/// Returns the current feature flags.
/// Analysis requires AI — it is disabled when `filament_search_use_ai` is `"false"`.
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

    // Analysis requires AI vision models; disabled when user opted out of AI.
    let analysis_enabled = store
        .get("filament_search_use_ai")
        .and_then(|v| v.as_str().map(|s| s != "false"))
        .unwrap_or(true);

    Ok(FeatureFlags {
        profiles_enabled,
        analysis_enabled,
    })
}

/// Check whether the initial setup wizard has been completed.
///
/// Setup is considered complete when:
/// 1. An AI provider is selected
/// 2. Either an API key for that provider is saved, or the provider is "local"
#[tauri::command]
pub fn check_setup_complete(app: AppHandle) -> Result<SetupStatus, String> {
    info!("Checking setup status");
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open store: {}", e);
        e.to_string()
    })?;

    let bambu_studio_path = store
        .get("bambu_studio_path")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let ai_provider = store
        .get("ai_provider")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    // Check if an API key exists for the configured provider
    let has_api_key = if let Some(ref provider) = ai_provider {
        if provider == "local" {
            // Local MCP server doesn't require an API key
            true
        } else {
            let service = match provider.as_str() {
                "claude" => "bambumate-claude-api",
                "openai" => "bambumate-openai-api",
                "kimi" => "bambumate-kimi-api",
                "openrouter" => "bambumate-openrouter-api",
                _ => "",
            };
            if service.is_empty() {
                false
            } else {
                keyring::Entry::new(service, "bambumate")
                    .and_then(|e| e.get_password())
                    .is_ok()
            }
        }
    } else {
        false
    };

    // Also check the setup_complete preference flag
    let setup_flag = store
        .get("setup_complete")
        .and_then(|v| v.as_str().map(|s| s == "true"))
        .unwrap_or(false);

    // When the user opted out of AI, no API key is needed — just the setup flag.
    let use_ai = store
        .get("filament_search_use_ai")
        .and_then(|v| v.as_str().map(|s| s != "false"))
        .unwrap_or(true);

    let setup_complete = if use_ai {
        setup_flag && ai_provider.is_some() && has_api_key
    } else {
        setup_flag
    };

    Ok(SetupStatus {
        bambu_studio_path,
        ai_provider,
        has_api_key,
        setup_complete,
    })
}

/// All keychain service names used by BambuMate.
const KEYCHAIN_SERVICES: &[&str] = &[
    "bambumate-claude-api",
    "bambumate-openai-api",
    "bambumate-kimi-api",
    "bambumate-openrouter-api",
];

/// Reset BambuMate to a clean installation state.
///
/// This clears all preferences from the store and deletes all stored API keys
/// from the system keychain. After calling this, the setup wizard will appear
/// on the next launch.
#[tauri::command]
pub fn reset_to_clean_install(app: AppHandle) -> Result<(), String> {
    info!("Resetting BambuMate to clean installation state");

    // Clear the preferences store
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open store: {}", e);
        e.to_string()
    })?;
    store.clear();
    store.save().map_err(|e| {
        warn!("Failed to save cleared store: {}", e);
        e.to_string()
    })?;
    info!("Preferences store cleared");

    // Delete all API keys from the system keychain
    let mut keychain_errors = Vec::new();
    for service in KEYCHAIN_SERVICES {
        match keyring::Entry::new(service, "bambumate") {
            Ok(entry) => match entry.delete_credential() {
                Ok(()) => info!("Deleted keychain entry: {}", service),
                Err(keyring::Error::NoEntry) => {
                    info!("No keychain entry to delete: {}", service);
                }
                Err(e) => {
                    warn!("Failed to delete keychain entry {}: {}", service, e);
                    keychain_errors.push(format!("{}: {}", service, e));
                }
            },
            Err(e) => {
                warn!("Failed to access keychain for {}: {}", service, e);
                keychain_errors.push(format!("{}: {}", service, e));
            }
        }
    }

    if !keychain_errors.is_empty() {
        let msg = format!(
            "Reset completed but some API keys could not be removed: {}",
            keychain_errors.join("; ")
        );
        warn!("{}", msg);
        return Err(msg);
    }

    info!("Clean install reset complete");
    Ok(())
}
