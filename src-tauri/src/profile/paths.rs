use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Resolved paths to Bambu Studio configuration and profile directories.
pub struct BambuPaths {
    /// Root configuration directory (e.g., ~/Library/Application Support/BambuStudio/)
    pub config_root: PathBuf,
    /// System filament profiles directory (e.g., .../system/BBL/filament/)
    pub system_filaments: PathBuf,
    /// User profiles root (e.g., .../user/)
    pub user_root: PathBuf,
    /// The active preset folder name from BambuStudio.conf (e.g., "1881310893")
    pub preset_folder: Option<String>,
}

impl BambuPaths {
    /// Detect Bambu Studio paths on the current platform.
    ///
    /// On macOS, looks for `~/Library/Application Support/BambuStudio/`.
    /// Reads `preset_folder` from `BambuStudio.conf` if available.
    pub fn detect() -> Result<Self> {
        let config_root = Self::find_config_root()?;
        let system_filaments = config_root.join("system").join("BBL").join("filament");
        let user_root = config_root.join("user");

        let preset_folder = Self::read_preset_folder(&config_root);
        if let Some(ref folder) = preset_folder {
            debug!("Detected preset_folder: {}", folder);
        } else {
            debug!("No preset_folder found in BambuStudio.conf");
        }

        Ok(Self {
            config_root,
            system_filaments,
            user_root,
            preset_folder,
        })
    }

    /// Find the Bambu Studio config root directory.
    #[cfg(target_os = "macos")]
    fn find_config_root() -> Result<PathBuf> {
        // Try dirs crate first (maps to ~/Library/Application Support on macOS)
        if let Some(data_dir) = dirs::data_dir() {
            let bs_dir = data_dir.join("BambuStudio");
            if bs_dir.exists() {
                debug!("Found Bambu Studio config at {:?} (via dirs::data_dir)", bs_dir);
                return Ok(bs_dir);
            }
        }

        // Fallback: explicit path
        if let Some(home) = dirs::home_dir() {
            let bs_dir = home.join("Library/Application Support/BambuStudio");
            if bs_dir.exists() {
                debug!(
                    "Found Bambu Studio config at {:?} (via home_dir fallback)",
                    bs_dir
                );
                return Ok(bs_dir);
            }
        }

        bail!("Bambu Studio config directory not found. Is Bambu Studio installed?")
    }

    /// Windows stub -- not yet supported.
    #[cfg(target_os = "windows")]
    fn find_config_root() -> Result<PathBuf> {
        bail!("Windows support is not yet implemented")
    }

    /// Linux stub -- not yet supported.
    #[cfg(target_os = "linux")]
    fn find_config_root() -> Result<PathBuf> {
        bail!("Linux support is not yet implemented")
    }

    /// Fallback for other platforms.
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn find_config_root() -> Result<PathBuf> {
        bail!("Unsupported platform")
    }

    /// Read the `preset_folder` value from BambuStudio.conf (JSON file).
    fn read_preset_folder(config_root: &Path) -> Option<String> {
        let conf_path = config_root.join("BambuStudio.conf");
        let content = match std::fs::read_to_string(&conf_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Could not read BambuStudio.conf: {}", e);
                return None;
            }
        };
        let conf: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                warn!("Could not parse BambuStudio.conf as JSON: {}", e);
                return None;
            }
        };
        conf.get("preset_folder")?.as_str().map(|s| s.to_string())
    }

    /// Get the active user filament profile directory.
    ///
    /// Looks for `user/{preset_folder}/filament/base/` first, then falls back
    /// to scanning for non-"default" directories that have a `filament/base/`
    /// subdirectory.
    pub fn user_filament_dir(&self) -> Option<PathBuf> {
        // Try preset_folder first
        if let Some(ref folder) = self.preset_folder {
            let path = self.user_root.join(folder).join("filament").join("base");
            if path.exists() {
                debug!("Found user filament dir via preset_folder: {:?}", path);
                return Some(path);
            }
        }

        // Fallback: scan for non-default directories with filament/base/
        if let Ok(entries) = std::fs::read_dir(&self.user_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name != "default" && entry.path().is_dir() {
                    let path = entry.path().join("filament").join("base");
                    if path.exists() {
                        debug!(
                            "Found user filament dir via directory scan: {:?}",
                            path
                        );
                        return Some(path);
                    }
                }
            }
        }

        warn!("No user filament directory found");
        None
    }

    /// Get the system filament profiles directory.
    pub fn system_filament_dir(&self) -> PathBuf {
        self.system_filaments.clone()
    }
}
