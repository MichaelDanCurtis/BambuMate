use serde::Serialize;
use tracing::info;

use crate::profile::generator;

/// Result from launching Bambu Studio.
#[derive(Debug, Clone, Serialize)]
pub struct LaunchResult {
    pub launched: bool,
    pub app_path: String,
    pub was_already_running: bool,
}

/// Detect the Bambu Studio application path.
///
/// Priority:
/// 1. User-configured `bambu_studio_path` preference
/// 2. Default `/Applications/BambuStudio.app`
/// 3. Spotlight search via `mdfind`
#[tauri::command]
pub async fn detect_bambu_studio_path(app: tauri::AppHandle) -> Result<String, String> {
    // 1. Check user preference
    if let Some(path) = get_bs_preference(&app) {
        let p = std::path::Path::new(&path);
        if p.exists() {
            return Ok(path);
        }
    }

    // 2. Default macOS path
    let default_path = "/Applications/BambuStudio.app";
    if std::path::Path::new(default_path).exists() {
        return Ok(default_path.to_string());
    }

    // 3. Spotlight search
    if let Ok(output) = std::process::Command::new("mdfind")
        .arg("kMDItemCFBundleIdentifier == 'com.bambulab.bambu-studio'")
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().next() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && std::path::Path::new(trimmed).exists() {
                    return Ok(trimmed.to_string());
                }
            }
        }
    }

    Err("Bambu Studio not found. Please install it or set the path in Settings.".to_string())
}

/// Launch Bambu Studio with optional STL and profile file arguments.
#[tauri::command]
pub async fn launch_bambu_studio(
    app: tauri::AppHandle,
    stl_path: Option<String>,
    profile_path: Option<String>,
) -> Result<LaunchResult, String> {
    // Resolve BS path
    let bs_path = resolve_bs_path(&app)?;
    info!("Launching Bambu Studio from: {}", bs_path);

    let was_running = generator::is_bambu_studio_running();

    // Build the command
    let mut cmd = std::process::Command::new("open");
    cmd.arg("-a").arg(&bs_path);

    // Add file arguments after "--args" separator
    let mut has_args = false;

    if let Some(ref stl) = stl_path {
        if std::path::Path::new(stl).exists() {
            if !has_args {
                cmd.arg("--args");
                has_args = true;
            }
            cmd.arg(stl);
            info!("  with STL: {}", stl);
        }
    }

    if let Some(ref profile) = profile_path {
        if std::path::Path::new(profile).exists() {
            if !has_args {
                cmd.arg("--args");
            }
            cmd.arg("--load-filaments").arg(profile);
            info!("  with profile: {}", profile);
        }
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to launch Bambu Studio: {}", e))?;

    info!(
        "Bambu Studio launch initiated (was_already_running: {})",
        was_running
    );

    Ok(LaunchResult {
        launched: true,
        app_path: bs_path,
        was_already_running: was_running,
    })
}

/// Resolve the Bambu Studio path from preferences or defaults.
fn resolve_bs_path(app: &tauri::AppHandle) -> Result<String, String> {
    // Check preference
    if let Some(path) = get_bs_preference(app) {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // Default path
    let default = "/Applications/BambuStudio.app";
    if std::path::Path::new(default).exists() {
        return Ok(default.to_string());
    }

    Err("Bambu Studio not found. Set the path in Settings.".to_string())
}

/// Read the bambu_studio_path preference from the Tauri store.
fn get_bs_preference(app: &tauri::AppHandle) -> Option<String> {
    use tauri_plugin_store::StoreExt;
    let store = app.store("preferences.json").ok()?;
    store
        .get("bambu_studio_path")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
}
