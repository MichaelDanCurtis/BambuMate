use serde::Serialize;
use tracing::info;
use walkdir::WalkDir;

use crate::profile::generator;
use crate::profile::paths::BambuPaths;
use crate::profile::reader::{read_profile, read_profile_metadata};
use crate::profile::registry::ProfileRegistry;
use crate::profile::types::{FilamentProfile, ProfileMetadata};
use crate::profile::writer::write_profile_with_metadata;

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

/// Result from profile generation (preview step, no files written).
#[derive(Debug, Clone, Serialize)]
pub struct GenerateResult {
    pub profile_name: String,
    pub profile_json: String,
    pub metadata_info: String,
    pub filename: String,
    pub field_count: usize,
    pub base_profile_used: String,
    pub specs_applied: GeneratedSpecs,
    pub warnings: Vec<String>,
    pub bambu_studio_running: bool,
}

/// Summary of which scraped specs were applied to the profile.
#[derive(Debug, Clone, Serialize)]
pub struct GeneratedSpecs {
    pub nozzle_temp: Option<String>,
    pub bed_temp: Option<String>,
    pub fan_speed: Option<String>,
    pub retraction: Option<String>,
}

/// Result from profile installation (files written to disk).
#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub installed_path: String,
    pub profile_name: String,
    pub bambu_studio_was_running: bool,
}

/// Generate a filament profile from scraped specifications (preview only).
///
/// This command does NOT write any files. It returns the generated profile
/// data for UI preview. Call `install_generated_profile` to actually write
/// the profile to disk.
///
/// Two-step flow: generate (preview) -> install (write) lets the UI show
/// a preview before committing.
#[tauri::command]
pub async fn generate_profile_from_specs(
    specs: crate::scraper::types::FilamentSpecs,
    target_printer: Option<String>,
) -> Result<GenerateResult, String> {
    info!(
        "generate_profile_from_specs called for: {} {}",
        specs.brand, specs.name
    );

    // Detect Bambu Studio paths
    let paths = BambuPaths::detect().map_err(|e| {
        format!(
            "Bambu Studio not found: {}. Please install Bambu Studio first.",
            e
        )
    })?;

    // Build registry from system filament profiles
    let system_dir = paths.system_filament_dir();
    if !system_dir.exists() {
        return Err(format!(
            "System filament directory not found at {:?}. Is Bambu Studio installed correctly?",
            system_dir
        ));
    }

    let registry = ProfileRegistry::discover_system_profiles(&system_dir)
        .map_err(|e| format!("Failed to load system profiles: {}", e))?;

    // Determine the base profile name for reporting
    let material = crate::scraper::types::MaterialType::from_str(&specs.material);
    let base_name = generator::base_profile_name(&material).to_string();

    // Generate the profile
    let (profile, metadata, filename) =
        generator::generate_profile(&specs, &registry, target_printer.as_deref())
            .map_err(|e| format!("Failed to generate profile: {}", e))?;

    // Serialize for transport
    let profile_json = profile
        .to_json_4space()
        .map_err(|e| format!("Failed to serialize profile: {}", e))?;
    let metadata_info = metadata.to_info_string();

    // Check if Bambu Studio is running
    let bs_running = generator::is_bambu_studio_running();

    // Build warnings
    let mut warnings = Vec::new();
    if bs_running {
        warnings.push(
            "Bambu Studio is running. Profile changes may not take effect until BS is restarted."
                .to_string(),
        );
    }

    // Build specs summary for UI display
    let specs_applied = GeneratedSpecs {
        nozzle_temp: specs.nozzle_temp_max.map(|max| {
            if let Some(min) = specs.nozzle_temp_min {
                format!("{}-{}C", min, max)
            } else {
                format!("{}C", max)
            }
        }),
        bed_temp: specs.bed_temp_max.map(|max| {
            if let Some(min) = specs.bed_temp_min {
                format!("{}-{}C", min, max)
            } else {
                format!("{}C", max)
            }
        }),
        fan_speed: specs.fan_speed_percent.map(|f| format!("{}%", f)),
        retraction: specs.retraction_distance_mm.map(|d| {
            if let Some(s) = specs.retraction_speed_mm_s {
                format!("{:.1}mm @ {}mm/s", d, s)
            } else {
                format!("{:.1}mm", d)
            }
        }),
    };

    let profile_name = profile
        .name()
        .unwrap_or("<unnamed>")
        .to_string();

    info!(
        "Generated profile '{}' with {} fields (base: {})",
        profile_name,
        profile.field_count(),
        base_name
    );

    Ok(GenerateResult {
        profile_name,
        profile_json,
        metadata_info,
        filename,
        field_count: profile.field_count(),
        base_profile_used: base_name,
        specs_applied,
        warnings,
        bambu_studio_running: bs_running,
    })
}

/// Install a previously generated profile to the Bambu Studio user directory.
///
/// Takes the profile JSON and metadata from `generate_profile_from_specs`
/// and writes them atomically to disk. Checks if Bambu Studio is running
/// and requires `force=true` to proceed if it is.
#[tauri::command]
pub async fn install_generated_profile(
    profile_json: String,
    metadata_info: String,
    filename: String,
    force: bool,
) -> Result<InstallResult, String> {
    info!("install_generated_profile called for: {}", filename);

    // Parse the profile and metadata back from serialized form
    let profile =
        FilamentProfile::from_json(&profile_json).map_err(|e| format!("Invalid profile JSON: {}", e))?;
    let metadata = ProfileMetadata::from_info_string(&metadata_info)
        .map_err(|e| format!("Invalid metadata: {}", e))?;

    // Check if Bambu Studio is running
    let bs_running = generator::is_bambu_studio_running();
    if bs_running && !force {
        return Err(
            "Bambu Studio is running. Use force=true to install anyway, but restart BS to see changes."
                .to_string(),
        );
    }

    // Detect paths and get user filament directory
    let paths = BambuPaths::detect().map_err(|e| {
        format!(
            "Bambu Studio not found: {}. Please install Bambu Studio first.",
            e
        )
    })?;

    let user_dir = paths.user_filament_dir().ok_or_else(|| {
        "User filament directory not found. Have you logged into Bambu Studio at least once?"
            .to_string()
    })?;

    // Build target path
    let target_path = user_dir.join(&filename);

    // Check for existing file
    if target_path.exists() {
        info!(
            "Overwriting existing profile at {:?}",
            target_path
        );
    }

    // Write profile + metadata atomically
    write_profile_with_metadata(&profile, &target_path, &metadata)
        .map_err(|e| format!("Failed to write profile: {}", e))?;

    let profile_name = profile
        .name()
        .unwrap_or("<unnamed>")
        .to_string();

    info!(
        "Installed profile '{}' to {:?}",
        profile_name, target_path
    );

    Ok(InstallResult {
        installed_path: target_path.to_string_lossy().to_string(),
        profile_name,
        bambu_studio_was_running: bs_running,
    })
}
