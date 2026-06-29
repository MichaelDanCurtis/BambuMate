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

/// Detect the Bambu Studio application binary path.
///
/// Priority:
/// 1. Platform-specific default location
/// 2. Platform-specific search (Spotlight on macOS, registry/PATH on Windows)
///
/// Note: The "bambu_studio_path" preference stores the CONFIG DIRECTORY,
/// not the application binary path.
#[tauri::command]
pub async fn detect_bambu_studio_path(_app: tauri::AppHandle) -> Result<String, String> {
    // 1. Platform-specific default path
    if let Some(path) = default_bs_path() {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // 2. Platform-specific search
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

/// Resolve the Bambu Studio application binary path.
///
/// Note: The "bambu_studio_path" preference stores the CONFIG DIRECTORY
/// (e.g., %APPDATA%\BambuStudio), not the application binary. We must not
/// use it here. Instead, we search for the actual binary.
fn resolve_bs_path(_app: &tauri::AppHandle) -> Result<String, String> {
    // 1. Platform-specific default path
    if let Some(path) = default_bs_path() {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // 2. Platform-specific search (Spotlight, registry, PATH, etc.)
    if let Some(path) = search_bs_path() {
        return Ok(path);
    }

    Err("Bambu Studio application not found. Please install Bambu Studio.".to_string())
}

/// Get the platform-specific default Bambu Studio path.
#[cfg(target_os = "macos")]
fn default_bs_path() -> Option<String> {
    Some("/Applications/BambuStudio.app".to_string())
}

#[cfg(target_os = "windows")]
fn default_bs_path() -> Option<String> {
    // Common install locations on Windows.
    // Bambu Studio installs to "Bambu Studio" (with space) by default.
    let candidates = [
        r"C:\Program Files\Bambu Studio\bambu-studio.exe",
        r"C:\Program Files\Bambu Studio\BambuStudio.exe",
        r"C:\Program Files\BambuStudio\BambuStudio.exe",
        r"C:\Program Files (x86)\Bambu Studio\bambu-studio.exe",
        r"C:\Program Files (x86)\Bambu Studio\BambuStudio.exe",
        r"C:\Program Files (x86)\BambuStudio\BambuStudio.exe",
    ];

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    // Check %PROGRAMFILES% and %PROGRAMFILES(X86)% environment variables
    for env_var in &["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(program_files) = std::env::var(env_var) {
            for folder in &["Bambu Studio", "BambuStudio"] {
                for exe in &["bambu-studio.exe", "BambuStudio.exe"] {
                    let path = std::path::PathBuf::from(&program_files)
                        .join(folder)
                        .join(exe);
                    if path.exists() {
                        return Some(path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn default_bs_path() -> Option<String> {
    let candidates = ["/usr/bin/BambuStudio", "/opt/BambuStudio/BambuStudio"];
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
    // 1. Search Windows registry (most reliable for installed apps)
    if let Some(path) = search_registry_for_bs() {
        return Some(path);
    }

    // 2. Search PATH for both possible executable names
    for exe_name in &["BambuStudio.exe", "bambu-studio.exe"] {
        if let Ok(output) = std::process::Command::new("where").arg(exe_name).output() {
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
    }

    // 3. Check common user-level install locations (with and without space)
    if let Some(local_data) = dirs::data_local_dir() {
        for folder in &["Bambu Studio", "BambuStudio"] {
            for exe in &["bambu-studio.exe", "BambuStudio.exe"] {
                let path = local_data.join(folder).join(exe);
                if path.exists() {
                    return Some(path.to_string_lossy().to_string());
                }
            }
        }
        // Also check %LOCALAPPDATA%\Programs\
        for folder in &["Bambu Studio", "BambuStudio"] {
            for exe in &["bambu-studio.exe", "BambuStudio.exe"] {
                let path = local_data.join("Programs").join(folder).join(exe);
                if path.exists() {
                    return Some(path.to_string_lossy().to_string());
                }
            }
        }
    }

    None
}

/// Search the Windows registry for the Bambu Studio install location.
#[cfg(target_os = "windows")]
fn search_registry_for_bs() -> Option<String> {
    let reg_keys = [
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\BambuStudio",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\BambuStudio",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\BambuStudio",
        r"HKLM\SOFTWARE\Bambu Lab\BambuStudio",
        r"HKCU\SOFTWARE\Bambu Lab\BambuStudio",
    ];

    for reg_key in &reg_keys {
        if let Ok(output) = std::process::Command::new("reg")
            .args(["query", reg_key, "/v", "InstallLocation"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.trim_start().starts_with("InstallLocation") {
                        // reg query output format: "    InstallLocation    REG_SZ    C:\path\to\app"
                        let parts: Vec<&str> = line.splitn(4, "    ").collect();
                        if let Some(install_dir) = parts.last() {
                            let install_dir = install_dir.trim();
                            if !install_dir.is_empty() {
                                for exe in &["bambu-studio.exe", "BambuStudio.exe"] {
                                    let exe_path = std::path::PathBuf::from(install_dir).join(exe);
                                    if exe_path.exists() {
                                        return Some(exe_path.to_string_lossy().to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Also try searching all uninstall entries for "Bambu Studio" display name
    for hive in &["HKLM", "HKCU"] {
        let uninstall_key = format!(
            r"{}\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            hive
        );
        if let Ok(output) = std::process::Command::new("reg")
            .args(["query", &uninstall_key, "/s", "/v", "DisplayName"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let lines: Vec<&str> = stdout.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.contains("DisplayName") && trimmed.contains("Bambu Studio") {
                        // Find the preceding registry key line (starts with HKLM/HKCU)
                        for j in (0..i).rev() {
                            let key_line = lines[j].trim();
                            if key_line.starts_with("HKEY_") {
                                // Now query InstallLocation from this specific key
                                if let Ok(loc_output) = std::process::Command::new("reg")
                                    .args(["query", key_line, "/v", "InstallLocation"])
                                    .output()
                                {
                                    if loc_output.status.success() {
                                        let loc_stdout =
                                            String::from_utf8_lossy(&loc_output.stdout);
                                        for loc_line in loc_stdout.lines() {
                                            if loc_line.trim_start().starts_with("InstallLocation")
                                            {
                                                let parts: Vec<&str> =
                                                    loc_line.splitn(4, "    ").collect();
                                                if let Some(install_dir) = parts.last() {
                                                    let install_dir = install_dir.trim();
                                                    if !install_dir.is_empty() {
                                                        for exe in
                                                            &["bambu-studio.exe", "BambuStudio.exe"]
                                                        {
                                                            let exe_path =
                                                                std::path::PathBuf::from(
                                                                    install_dir,
                                                                )
                                                                .join(exe);
                                                            if exe_path.exists() {
                                                                return Some(
                                                                    exe_path
                                                                        .to_string_lossy()
                                                                        .to_string(),
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
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

/// Open an external URL in the system's default browser.
/// Uses the tauri-plugin-opener to handle cross-platform URL opening.
#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), String> {
    info!("Opening external URL: {}", url);
    tauri_plugin_opener::open_url(&url, None::<&str>)
        .map_err(|e| format!("Failed to open URL: {}", e))
}
