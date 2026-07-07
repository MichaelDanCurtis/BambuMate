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
    /// On Windows, looks for `%APPDATA%\BambuStudio\`.
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
                debug!(
                    "Found Bambu Studio config at {:?} (via dirs::data_dir)",
                    bs_dir
                );
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

    /// Find Bambu Studio config root on Windows.
    ///
    /// Searches in order:
    /// 1. `%APPDATA%\BambuStudio\` (primary location)
    /// 2. `%LOCALAPPDATA%\BambuStudio\` (alternate location)
    /// 3. Explicit `%APPDATA%` env var fallback
    #[cfg(target_os = "windows")]
    fn find_config_root() -> Result<PathBuf> {
        // Primary: dirs::data_dir() maps to %APPDATA% on Windows
        if let Some(data_dir) = dirs::data_dir() {
            let bs_dir = data_dir.join("BambuStudio");
            if bs_dir.exists() {
                debug!(
                    "Found Bambu Studio config at {:?} (via dirs::data_dir)",
                    bs_dir
                );
                return Ok(bs_dir);
            }
        }

        // Fallback: check %LOCALAPPDATA% (some versions may use this)
        if let Some(local_data) = dirs::data_local_dir() {
            let bs_dir = local_data.join("BambuStudio");
            if bs_dir.exists() {
                debug!(
                    "Found Bambu Studio config at {:?} (via dirs::data_local_dir)",
                    bs_dir
                );
                return Ok(bs_dir);
            }
        }

        // Second fallback: explicit %APPDATA% path construction
        if let Ok(appdata) = std::env::var("APPDATA") {
            let bs_dir = PathBuf::from(&appdata).join("BambuStudio");
            if bs_dir.exists() {
                debug!(
                    "Found Bambu Studio config at {:?} (via APPDATA env var)",
                    bs_dir
                );
                return Ok(bs_dir);
            }
        }

        bail!("Bambu Studio config directory not found. Is Bambu Studio installed?")
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
                        debug!("Found user filament dir via directory scan: {:?}", path);
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

    /// Detect the installed Bambu Studio schema/format version.
    ///
    /// Reads `system/BBL.json` (present in the user's BS config root on both
    /// Windows and macOS) and returns the `version` field with leading zeros
    /// stripped from each dotted component. That is, the raw value
    /// `"02.07.00.07"` becomes `"2.7.0.7"` — the same shape Bambu Studio
    /// itself stamps onto user profile JSONs when it saves them.
    ///
    /// Returns `None` if BBL.json is missing, unreadable, or malformed.
    pub fn bambu_studio_version(&self) -> Option<String> {
        let bbl_path = self.config_root.join("system").join("BBL.json");
        let content = std::fs::read_to_string(&bbl_path).ok()?;
        let conf: serde_json::Value = serde_json::from_str(&content).ok()?;
        let raw = conf.get("version")?.as_str()?;
        Some(normalize_version(raw))
    }
}

/// Strip leading zeros from each dotted component of a version string.
///
/// `"02.07.00.07"` → `"2.7.0.7"`. Empty components collapse to `"0"` so
/// malformed input like `"..."` produces `"0.0.0.0"` rather than an empty
/// string. Non-numeric components are passed through unchanged.
fn normalize_version(raw: &str) -> String {
    raw.split('.')
        .map(|part| {
            let trimmed = part.trim_start_matches('0');
            if trimmed.is_empty() {
                "0".to_string()
            } else {
                trimmed.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::normalize_version;

    #[test]
    fn strips_leading_zeros_from_each_component() {
        assert_eq!(normalize_version("02.07.00.07"), "2.7.0.7");
    }

    #[test]
    fn leaves_already_normalized_version_unchanged() {
        assert_eq!(normalize_version("2.7.0.7"), "2.7.0.7");
    }

    #[test]
    fn collapses_all_zero_component_to_single_zero() {
        assert_eq!(normalize_version("00.00.00"), "0.0.0");
    }

    #[test]
    fn preserves_non_numeric_components() {
        assert_eq!(normalize_version("2.7.beta.01"), "2.7.beta.1");
    }
}
