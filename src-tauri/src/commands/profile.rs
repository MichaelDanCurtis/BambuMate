use serde::Serialize;
use tracing::info;
use walkdir::WalkDir;

use crate::profile::paths::BambuPaths;
use crate::profile::reader::{read_profile, read_profile_metadata};

/// Summary information for a filament profile (used in list views).
#[derive(Debug, Clone, Serialize)]
pub struct ProfileInfo {
    pub name: String,
    pub filament_type: Option<String>,
    pub filament_id: Option<String>,
    pub path: String,
    pub is_user_profile: bool,
}

/// Detailed information for a single filament profile.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileDetail {
    pub name: Option<String>,
    pub filament_type: Option<String>,
    pub filament_id: Option<String>,
    pub inherits: Option<String>,
    pub field_count: usize,
    pub nozzle_temperature: Option<Vec<String>>,
    pub bed_temperature: Option<Vec<String>>,
    pub compatible_printers: Option<Vec<String>>,
    pub metadata: Option<ProfileMetadataInfo>,
    pub raw_json: String,
}

/// Serializable metadata from a `.info` companion file.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileMetadataInfo {
    pub sync_info: String,
    pub user_id: String,
    pub setting_id: String,
    pub base_id: String,
    pub updated_time: u64,
}

/// List all user filament profiles.
///
/// Scans the user filament directory and returns summary info for each profile.
/// Returns an empty vec if Bambu Studio is not installed (not an error).
#[tauri::command]
pub fn list_profiles() -> Result<Vec<ProfileInfo>, String> {
    let paths = match BambuPaths::detect() {
        Ok(p) => p,
        Err(_) => {
            info!("Bambu Studio not detected, returning empty profile list");
            return Ok(Vec::new());
        }
    };

    let user_dir = match paths.user_filament_dir() {
        Some(d) => d,
        None => {
            info!("No user filament directory found, returning empty profile list");
            return Ok(Vec::new());
        }
    };

    let mut profiles: Vec<ProfileInfo> = Vec::new();

    for entry in WalkDir::new(&user_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        match read_profile(path) {
            Ok(profile) => {
                let name = profile
                    .name()
                    .unwrap_or("<unnamed>")
                    .to_string();

                profiles.push(ProfileInfo {
                    name,
                    filament_type: profile.filament_type().map(|s| s.to_string()),
                    filament_id: profile.filament_id().map(|s| s.to_string()),
                    path: path.to_string_lossy().to_string(),
                    is_user_profile: true,
                });
            }
            Err(e) => {
                info!("Skipping unreadable profile at {:?}: {}", path, e);
            }
        }
    }

    // Sort alphabetically by name
    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    info!("Found {} user profiles", profiles.len());
    Ok(profiles)
}

/// Read a single profile with full detail.
///
/// Returns the profile data including metadata from the companion .info file.
#[tauri::command]
pub fn read_profile_command(path: String) -> Result<ProfileDetail, String> {
    let file_path = std::path::Path::new(&path);

    let profile = read_profile(file_path).map_err(|e| e.to_string())?;
    let raw_json = profile.to_json_4space().map_err(|e| e.to_string())?;

    // Try to read metadata
    let metadata = match read_profile_metadata(file_path) {
        Ok(Some(meta)) => Some(ProfileMetadataInfo {
            sync_info: meta.sync_info,
            user_id: meta.user_id,
            setting_id: meta.setting_id,
            base_id: meta.base_id,
            updated_time: meta.updated_time,
        }),
        _ => None,
    };

    Ok(ProfileDetail {
        name: profile.name().map(|s| s.to_string()),
        filament_type: profile.filament_type().map(|s| s.to_string()),
        filament_id: profile.filament_id().map(|s| s.to_string()),
        inherits: profile.inherits().map(|s| s.to_string()),
        field_count: profile.field_count(),
        nozzle_temperature: profile
            .nozzle_temperature()
            .map(|v| v.into_iter().map(|s| s.to_string()).collect()),
        bed_temperature: profile
            .get_string_array("bed_temperature")
            .map(|v| v.into_iter().map(|s| s.to_string()).collect()),
        compatible_printers: profile
            .compatible_printers()
            .map(|v| v.into_iter().map(|s| s.to_string()).collect()),
        metadata,
        raw_json,
    })
}

/// Get the count of system filament profiles.
///
/// Quick check: counts .json files in the system filaments directory.
/// Useful for health checks and UI display.
#[tauri::command]
pub fn get_system_profile_count() -> Result<usize, String> {
    let paths = match BambuPaths::detect() {
        Ok(p) => p,
        Err(_) => {
            info!("Bambu Studio not detected, returning 0 system profiles");
            return Ok(0);
        }
    };

    let system_dir = paths.system_filament_dir();
    if !system_dir.exists() {
        info!("System filament directory does not exist: {:?}", system_dir);
        return Ok(0);
    }

    let count = WalkDir::new(&system_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file()
                && e.path().extension().and_then(|ext| ext.to_str()) == Some("json")
        })
        .count();

    info!("Found {} system profiles", count);
    Ok(count)
}
