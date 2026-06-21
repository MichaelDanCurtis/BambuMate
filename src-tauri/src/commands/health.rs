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
