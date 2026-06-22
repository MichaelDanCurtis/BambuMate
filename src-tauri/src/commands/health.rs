use serde::Serialize;
use std::path::PathBuf;
use tracing::info;

use crate::profile::paths::BambuPaths;

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub bambu_studio_installed: bool,
    pub bambu_studio_path: Option<String>,
    pub profile_dir_accessible: bool,
    pub profile_dir_path: Option<String>,
    pub claude_api_key_set: bool,
    pub openai_api_key_set: bool,
    pub kimi_api_key_set: bool,
    pub openrouter_api_key_set: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathValidation {
    pub valid: bool,
    pub has_system_profiles: bool,
    pub has_config_file: bool,
    pub message: String,
}

#[tauri::command]
pub fn run_health_check() -> Result<HealthReport, String> {
    info!("Running health check");

    // Check Bambu Studio installation using cross-platform detection
    let (bs_installed, bs_path) = detect_bambu_studio_install();
    info!("Bambu Studio installed: {}", bs_installed);

    // Check profile directory using cross-platform BambuPaths
    let (profile_accessible, profile_dir_path) = match BambuPaths::detect() {
        Ok(paths) => {
            let dir = paths.config_root.clone();
            let accessible = dir.exists() && dir.is_dir();
            (accessible, Some(dir.to_string_lossy().to_string()))
        }
        Err(_) => {
            // Fallback: check platform-specific data directory
            let dir = dirs::data_dir()
                .map(|d| d.join("BambuStudio"))
                .unwrap_or_else(|| PathBuf::from(""));
            let accessible = dir.exists() && dir.is_dir();
            (accessible, if accessible { Some(dir.to_string_lossy().to_string()) } else { None })
        }
    };
    info!("Profile directory accessible: {}, path: {:?}", profile_accessible, profile_dir_path);

    // Check API keys
    let claude_key_set = keyring::Entry::new("bambumate-claude-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    let openai_key_set = keyring::Entry::new("bambumate-openai-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    let kimi_key_set = keyring::Entry::new("bambumate-kimi-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    let openrouter_key_set = keyring::Entry::new("bambumate-openrouter-api", "bambumate")
        .and_then(|e| e.get_password())
        .is_ok();
    info!("Claude API key set: {}, OpenAI API key set: {}, Kimi API key set: {}, OpenRouter API key set: {}", claude_key_set, openai_key_set, kimi_key_set, openrouter_key_set);

    Ok(HealthReport {
        bambu_studio_installed: bs_installed,
        bambu_studio_path: bs_path,
        profile_dir_accessible: profile_accessible,
        profile_dir_path,
        claude_api_key_set: claude_key_set,
        openai_api_key_set: openai_key_set,
        kimi_api_key_set: kimi_key_set,
        openrouter_api_key_set: openrouter_key_set,
    })
}

/// Search for the Bambu Studio configuration directory on the system.
/// Returns the path if found, or an error message with guidance.
#[tauri::command]
pub fn search_bambu_studio_config() -> Result<String, String> {
    info!("Searching for Bambu Studio config directory");

    match BambuPaths::detect() {
        Ok(paths) => {
            let path = paths.config_root.to_string_lossy().to_string();
            info!("Found Bambu Studio config at: {}", path);
            Ok(path)
        }
        Err(_) => {
            // Try additional platform-specific search paths
            if let Some(path) = search_config_fallback() {
                info!("Found Bambu Studio config via fallback: {}", path);
                Ok(path)
            } else {
                Err("Could not find Bambu Studio configuration directory. Please select it manually.".to_string())
            }
        }
    }
}

/// Validate that a given path is a valid Bambu Studio configuration directory.
/// Checks for expected subdirectories and config files.
#[tauri::command]
pub fn validate_bambu_studio_path(path: String) -> Result<PathValidation, String> {
    info!("Validating Bambu Studio path: {}", path);
    let p = PathBuf::from(&path);

    if !p.exists() || !p.is_dir() {
        return Ok(PathValidation {
            valid: false,
            has_system_profiles: false,
            has_config_file: false,
            message: "Directory does not exist".to_string(),
        });
    }

    // Check for system profiles (system/BBL/filament/)
    let system_filament = p.join("system").join("BBL").join("filament");
    let has_system_profiles = system_filament.exists() && system_filament.is_dir();

    // Check for BambuStudio.conf
    let conf_file = p.join("BambuStudio.conf");
    let has_config_file = conf_file.exists();

    let valid = has_system_profiles || has_config_file;
    let message = if valid {
        "Valid Bambu Studio configuration directory".to_string()
    } else if p.join("user").exists() {
        "Directory has a user folder but may be incomplete. You can try using it.".to_string()
    } else {
        "This does not appear to be a Bambu Studio configuration directory. Expected to find system/BBL/filament/ or BambuStudio.conf".to_string()
    };

    Ok(PathValidation {
        valid,
        has_system_profiles,
        has_config_file,
        message,
    })
}

/// Platform-specific fallback search for the Bambu Studio config directory.
#[cfg(target_os = "windows")]
fn search_config_fallback() -> Option<String> {
    // Check %APPDATA%\BambuStudio (primary on Windows)
    if let Ok(appdata) = std::env::var("APPDATA") {
        let bs_dir = PathBuf::from(&appdata).join("BambuStudio");
        if bs_dir.exists() {
            return Some(bs_dir.to_string_lossy().to_string());
        }
    }

    // Check %LOCALAPPDATA%\BambuStudio
    if let Some(local_data) = dirs::data_local_dir() {
        let bs_dir = local_data.join("BambuStudio");
        if bs_dir.exists() {
            return Some(bs_dir.to_string_lossy().to_string());
        }
    }

    // Check dirs::data_dir (maps to %APPDATA%)
    if let Some(data_dir) = dirs::data_dir() {
        let bs_dir = data_dir.join("BambuStudio");
        if bs_dir.exists() {
            return Some(bs_dir.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn search_config_fallback() -> Option<String> {
    if let Some(home) = dirs::home_dir() {
        let bs_dir = home.join("Library/Application Support/BambuStudio");
        if bs_dir.exists() {
            return Some(bs_dir.to_string_lossy().to_string());
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn search_config_fallback() -> Option<String> {
    if let Some(home) = dirs::home_dir() {
        let bs_dir = home.join(".config/BambuStudio");
        if bs_dir.exists() {
            return Some(bs_dir.to_string_lossy().to_string());
        }
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn search_config_fallback() -> Option<String> {
    None
}

/// Detect Bambu Studio installation using platform-specific paths.
#[cfg(target_os = "macos")]
fn detect_bambu_studio_install() -> (bool, Option<String>) {
    let path = PathBuf::from("/Applications/BambuStudio.app");
    if path.exists() {
        (true, Some(path.to_string_lossy().to_string()))
    } else {
        (false, None)
    }
}

#[cfg(target_os = "windows")]
fn detect_bambu_studio_install() -> (bool, Option<String>) {
    let candidates = [
        r"C:\Program Files\BambuStudio\BambuStudio.exe",
        r"C:\Program Files (x86)\BambuStudio\BambuStudio.exe",
    ];

    for path_str in &candidates {
        let path = PathBuf::from(path_str);
        if path.exists() {
            return (true, Some(path_str.to_string()));
        }
    }

    // Check %PROGRAMFILES%
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        let path = PathBuf::from(&program_files)
            .join("BambuStudio")
            .join("BambuStudio.exe");
        if path.exists() {
            return (true, Some(path.to_string_lossy().to_string()));
        }
    }

    // Check %LOCALAPPDATA%
    if let Some(local_data) = dirs::data_local_dir() {
        let path = local_data.join("BambuStudio").join("BambuStudio.exe");
        if path.exists() {
            return (true, Some(path.to_string_lossy().to_string()));
        }
    }

    // Check %LOCALAPPDATA%\Programs (common for user-level Windows installs)
    if let Some(local_data) = dirs::data_local_dir() {
        let path = local_data
            .join("Programs")
            .join("BambuStudio")
            .join("BambuStudio.exe");
        if path.exists() {
            return (true, Some(path.to_string_lossy().to_string()));
        }
    }

    (false, None)
}

#[cfg(target_os = "linux")]
fn detect_bambu_studio_install() -> (bool, Option<String>) {
    let candidates = [
        "/usr/bin/BambuStudio",
        "/opt/BambuStudio/BambuStudio",
    ];
    for path_str in &candidates {
        let path = PathBuf::from(path_str);
        if path.exists() {
            return (true, Some(path_str.to_string()));
        }
    }
    (false, None)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn detect_bambu_studio_install() -> (bool, Option<String>) {
    (false, None)
}
