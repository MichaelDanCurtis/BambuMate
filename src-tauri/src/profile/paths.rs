use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tracing::{debug, info, warn};

/// Process-wide override for the Bambu Studio config root.
///
/// The setup wizard and Settings both let the user pick their Bambu Studio
/// configuration folder and store it under the `bambu_studio_path`
/// preference. Every profile operation resolves paths through
/// [`BambuPaths::detect`], so the override has to be visible there — and most
/// of those call sites (`install_generated_profile`, `delete_profile`, ...)
/// have no `AppHandle` to read the preference store from. Keeping the
/// resolved value in one process-wide slot, populated at startup and refreshed
/// whenever the preference changes, is what makes the picker actually take
/// effect.
static CONFIG_ROOT_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Install (or clear, with `None`) the user-configured config root.
pub fn set_config_root_override(path: Option<PathBuf>) {
    let normalized = path.filter(|p| !p.as_os_str().is_empty());
    match &normalized {
        Some(p) => info!("Using configured Bambu Studio config root: {:?}", p),
        None => debug!("Cleared Bambu Studio config root override"),
    }
    match CONFIG_ROOT_OVERRIDE.write() {
        Ok(mut slot) => *slot = normalized,
        Err(poisoned) => *poisoned.into_inner() = normalized,
    }
}

/// The currently configured config root, if any.
pub fn config_root_override() -> Option<PathBuf> {
    match CONFIG_ROOT_OVERRIDE.read() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

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
    /// If the user configured a config folder (setup wizard or Settings), that
    /// folder wins. Otherwise the platform default is used: on macOS
    /// `~/Library/Application Support/BambuStudio/`, on Windows
    /// `%APPDATA%\BambuStudio\`. Reads `preset_folder` from
    /// `BambuStudio.conf` if available.
    pub fn detect() -> Result<Self> {
        Self::detect_with_override(config_root_override().as_deref())
    }

    /// Same as [`detect`](Self::detect) but with an explicit override, so the
    /// resolution logic can be tested and diagnosed without touching global
    /// state.
    ///
    /// An override that does not point at an existing directory is ignored
    /// with a warning rather than being a hard failure — an external volume
    /// may simply be unmounted, and falling back to the platform default is
    /// more useful than refusing to work at all.
    pub fn detect_with_override(override_root: Option<&Path>) -> Result<Self> {
        let config_root = match override_root {
            Some(p) if p.is_dir() => {
                debug!("Using configured Bambu Studio config root: {:?}", p);
                p.to_path_buf()
            }
            Some(p) => {
                warn!(
                    "Configured Bambu Studio config root {:?} is not a directory; \
                     falling back to platform detection",
                    p
                );
                Self::find_config_root()?
            }
            None => Self::find_config_root()?,
        };

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
    ///
    /// On Windows the file has a trailing `# MD5 checksum ...` line appended
    /// after the JSON, so it must be stripped before parsing — otherwise this
    /// always fails and callers silently fall back to scanning `user/`.
    fn read_preset_folder(config_root: &Path) -> Option<String> {
        let conf_path = config_root.join("BambuStudio.conf");
        let content = match std::fs::read_to_string(&conf_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Could not read BambuStudio.conf: {}", e);
                return None;
            }
        };
        let conf: serde_json::Value =
            match serde_json::from_str(super::writer::strip_md5_checksum(&content)) {
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
    use super::{normalize_version, BambuPaths};

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

    /// Build a minimal but realistic Bambu Studio config tree.
    fn fake_config_root(dir: &std::path::Path, with_md5_line: bool) -> std::path::PathBuf {
        let root = dir.join("BambuStudio");
        std::fs::create_dir_all(root.join("system").join("BBL").join("filament")).unwrap();
        std::fs::create_dir_all(
            root.join("user")
                .join("1881310893")
                .join("filament")
                .join("base"),
        )
        .unwrap();

        let json = serde_json::to_string_pretty(&serde_json::json!({
            "preset_folder": "1881310893",
            "filaments": ["Bambu PLA Basic"],
        }))
        .unwrap();
        let mut body = json.clone();
        body.push('\n');
        if with_md5_line {
            body.push_str("# MD5 checksum 0123456789ABCDEF0123456789ABCDEF\n");
        }
        std::fs::write(root.join("BambuStudio.conf"), body).unwrap();
        root
    }

    #[test]
    fn override_is_used_instead_of_platform_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = fake_config_root(tmp.path(), false);

        let paths = BambuPaths::detect_with_override(Some(&root)).unwrap();

        assert_eq!(paths.config_root, root);
        assert_eq!(paths.preset_folder.as_deref(), Some("1881310893"));
        assert_eq!(
            paths.user_filament_dir(),
            Some(
                root.join("user")
                    .join("1881310893")
                    .join("filament")
                    .join("base")
            )
        );
        assert_eq!(
            paths.system_filament_dir(),
            root.join("system").join("BBL").join("filament")
        );
    }

    /// Regression: on Windows the conf carries a trailing `# MD5 checksum`
    /// line. Parsing it without stripping made `preset_folder` always `None`.
    #[test]
    fn preset_folder_is_read_even_with_a_trailing_md5_line() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = fake_config_root(tmp.path(), true);

        let paths = BambuPaths::detect_with_override(Some(&root)).unwrap();

        assert_eq!(
            paths.preset_folder.as_deref(),
            Some("1881310893"),
            "the MD5 checksum line must be stripped before parsing BambuStudio.conf"
        );
    }

    /// An override pointing at a missing directory (unmounted volume, deleted
    /// folder) must not hard-fail; it falls back to platform detection.
    #[test]
    fn missing_override_falls_back_to_platform_detection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing = tmp.path().join("definitely-not-here");

        // Either the platform default exists (Bambu Studio installed on this
        // machine) or detection errors — both are acceptable. What must not
        // happen is silently returning the missing override.
        if let Ok(paths) = BambuPaths::detect_with_override(Some(&missing)) {
            assert_ne!(paths.config_root, missing);
        }
    }
}
