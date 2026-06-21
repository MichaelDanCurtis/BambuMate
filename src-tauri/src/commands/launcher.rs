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
/// 2. Platform-specific default location
/// 3. Platform-specific search (Spotlight on macOS, registry/PATH on Windows)
#[tauri::command]
pub async fn detect_bambu_studio_path(app: tauri::AppHandle) -> Result<String, String> {
    // 1. Check user preference
    if let Some(path) = get_bs_preference(&app) {
        let p = std::path::Path::new(&path);
        if p.exists() {
            return Ok(path);
        }
    }

    // 2. Platform-specific default path
    if let Some(path) = default_bs_path() {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // 3. Platform-specific search
    if let Some(path) = search_bs_path() {
        return Ok(path);
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

    // Platform-specific launch
    launch_platform(&bs_path, stl_path.as_deref(), profile_path.as_deref())?;

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
    if let Some(path) = default_bs_path() {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
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

/// Get the platform-specific default Bambu Studio path.
#[cfg(target_os = "macos")]
fn default_bs_path() -> Option<String> {
    Some("/Applications/BambuStudio.app".to_string())
}

#[cfg(target_os = "windows")]
fn default_bs_path() -> Option<String> {
    // Common install locations on Windows
    let candidates = [
        r"C:\Program Files\BambuStudio\BambuStudio.exe",
        r"C:\Program Files (x86)\BambuStudio\BambuStudio.exe",
    ];

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    // Also check %PROGRAMFILES% environment variable
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        let path = std::path::PathBuf::from(&program_files)
            .join("BambuStudio")
            .join("BambuStudio.exe");
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn default_bs_path() -> Option<String> {
    let candidates = [
        "/usr/bin/BambuStudio",
        "/opt/BambuStudio/BambuStudio",
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn default_bs_path() -> Option<String> {
    None
}

/// Platform-specific search for Bambu Studio.
#[cfg(target_os = "macos")]
fn search_bs_path() -> Option<String> {
    // Spotlight search
    if let Ok(output) = std::process::Command::new("mdfind")
        .arg("kMDItemCFBundleIdentifier == 'com.bambulab.bambu-studio'")
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().next() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && std::path::Path::new(trimmed).exists() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn search_bs_path() -> Option<String> {
    // Search in PATH
    if let Ok(output) = std::process::Command::new("where")
        .arg("BambuStudio.exe")
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().next() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && std::path::Path::new(trimmed).exists() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    // Search in common user install locations
    if let Some(local_data) = dirs::data_local_dir() {
        let path = local_data
            .join("BambuStudio")
            .join("BambuStudio.exe");
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn search_bs_path() -> Option<String> {
    if let Ok(output) = std::process::Command::new("which")
        .arg("BambuStudio")
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let trimmed = stdout.trim();
            if !trimmed.is_empty() && std::path::Path::new(trimmed).exists() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn search_bs_path() -> Option<String> {
    None
}

/// Platform-specific application launch.
#[cfg(target_os = "macos")]
fn launch_platform(
    bs_path: &str,
    stl_path: Option<&str>,
    profile_path: Option<&str>,
) -> Result<(), String> {
    let mut cmd = std::process::Command::new("open");
    cmd.arg("-a").arg(bs_path);

    let mut has_args = false;

    if let Some(stl) = stl_path {
        if std::path::Path::new(stl).exists() {
            if !has_args {
                cmd.arg("--args");
                has_args = true;
            }
            cmd.arg(stl);
            info!("  with STL: {}", stl);
        }
    }

    if let Some(profile) = profile_path {
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
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn launch_platform(
    bs_path: &str,
    stl_path: Option<&str>,
    profile_path: Option<&str>,
) -> Result<(), String> {
    let mut cmd = std::process::Command::new(bs_path);

    if let Some(stl) = stl_path {
        if std::path::Path::new(stl).exists() {
            cmd.arg(stl);
            info!("  with STL: {}", stl);
        }
    }

    if let Some(profile) = profile_path {
        if std::path::Path::new(profile).exists() {
            cmd.arg("--load-filaments").arg(profile);
            info!("  with profile: {}", profile);
        }
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to launch Bambu Studio: {}", e))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn launch_platform(
    _bs_path: &str,
    _stl_path: Option<&str>,
    _profile_path: Option<&str>,
) -> Result<(), String> {
    Err("Unsupported platform for launching Bambu Studio".to_string())
}
