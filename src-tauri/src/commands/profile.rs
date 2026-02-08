use serde::Serialize;
use tracing::info;
use walkdir::WalkDir;

use crate::profile::generator;
use crate::profile::inheritance::resolve_inheritance;
use crate::profile::paths::BambuPaths;
use crate::profile::reader::{read_profile, read_profile_metadata};
use crate::profile::registry::ProfileRegistry;
use crate::profile::types::{FilamentProfile, ProfileMetadata};
use crate::profile::writer::{write_profile_atomic, write_profile_with_metadata};

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

/// List all system/factory filament profiles bundled with Bambu Studio.
///
/// Scans the system filament directory and returns summary info for each profile.
/// These are the built-in profiles like "Generic PLA", "Bambu PLA Basic", etc.
#[tauri::command]
pub fn list_system_profiles() -> Result<Vec<ProfileInfo>, String> {
    let paths = match BambuPaths::detect() {
        Ok(p) => p,
        Err(_) => {
            info!("Bambu Studio not detected, returning empty system profile list");
            return Ok(Vec::new());
        }
    };

    let system_dir = paths.system_filament_dir();
    if !system_dir.exists() {
        info!("System filament directory does not exist: {:?}", system_dir);
        return Ok(Vec::new());
    }

    let mut profiles: Vec<ProfileInfo> = Vec::new();

    for entry in WalkDir::new(&system_dir)
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
                let name = match profile.name() {
                    Some(n) => n.to_string(),
                    None => continue, // Skip registry files like BBL.json
                };

                profiles.push(ProfileInfo {
                    name,
                    filament_type: profile.filament_type().map(|s| s.to_string()),
                    filament_id: profile.filament_id().map(|s| s.to_string()),
                    path: path.to_string_lossy().to_string(),
                    is_user_profile: false,
                });
            }
            Err(e) => {
                info!("Skipping unreadable system profile at {:?}: {}", path, e);
            }
        }
    }

    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    info!("Found {} system profiles", profiles.len());
    Ok(profiles)
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
    pub diffs: Vec<ProfileDiff>,
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

/// A single field difference between the base profile and the generated profile.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileDiff {
    pub key: String,
    pub label: String,
    pub base_value: String,
    pub new_value: String,
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

    // Resolve the base profile to compare against later
    let base_resolved = resolve_inheritance(
        registry.get_by_name(&base_name).unwrap(),
        &registry,
    )
    .map_err(|e| format!("Failed to resolve base profile: {}", e))?;

    // Generate the profile
    let (profile, metadata, filename) =
        generator::generate_profile(&specs, &registry, target_printer.as_deref())
            .map_err(|e| format!("Failed to generate profile: {}", e))?;

    // Compute diffs between base and generated profile
    let diffs = compute_profile_diffs(&base_resolved, &profile);

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
        "Generated profile '{}' with {} fields, {} diffs from base (base: {})",
        profile_name,
        profile.field_count(),
        diffs.len(),
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
        diffs,
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

/// Delete a user filament profile and its companion .info file.
///
/// Safety: Validates that the path is within the user filament directory
/// to prevent deletion of arbitrary files.
#[tauri::command]
pub fn delete_profile(path: String) -> Result<(), String> {
    let file_path = std::path::Path::new(&path);

    // Safety check: path must be within user filament directory
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    let user_dir = paths
        .user_filament_dir()
        .ok_or_else(|| "User filament directory not found".to_string())?;

    let canonical_path = file_path
        .canonicalize()
        .map_err(|e| format!("Invalid path: {}", e))?;
    let canonical_user_dir = user_dir
        .canonicalize()
        .map_err(|e| format!("Cannot resolve user directory: {}", e))?;

    if !canonical_path.starts_with(&canonical_user_dir) {
        return Err("Cannot delete profiles outside the user filament directory".to_string());
    }

    // Delete the JSON file
    std::fs::remove_file(&file_path).map_err(|e| format!("Failed to delete profile: {}", e))?;

    // Delete companion .info file if it exists
    let info_path = file_path.with_extension("info");
    if info_path.exists() {
        if let Err(e) = std::fs::remove_file(&info_path) {
            info!("Could not delete companion .info file: {}", e);
        }
    }

    info!("Deleted profile at {:?}", file_path);
    Ok(())
}

/// Update a single field in a profile and write it back atomically.
///
/// The value is a JSON string that will be parsed as a serde_json::Value.
/// Returns the updated ProfileDetail.
#[tauri::command]
pub fn update_profile_field(
    path: String,
    key: String,
    value: String,
) -> Result<ProfileDetail, String> {
    let file_path = std::path::Path::new(&path);

    let mut profile = read_profile(file_path).map_err(|e| e.to_string())?;

    // Parse value as JSON to support arrays, strings, numbers, etc.
    let json_value: serde_json::Value =
        serde_json::from_str(&value).map_err(|e| format!("Invalid JSON value: {}", e))?;

    profile
        .raw_mut()
        .insert(key.clone(), json_value);

    write_profile_atomic(&profile, file_path)
        .map_err(|e| format!("Failed to write profile: {}", e))?;

    info!("Updated field '{}' in {:?}", key, file_path);

    // Return updated detail
    read_profile_command(path)
}

/// Duplicate a profile with a new name and IDs.
///
/// Copies the profile, assigns new filament_id and name, and writes it
/// to the user filament directory.
#[tauri::command]
pub fn duplicate_profile(path: String, new_name: String) -> Result<ProfileDetail, String> {
    let file_path = std::path::Path::new(&path);

    let mut profile = read_profile(file_path).map_err(|e| e.to_string())?;

    // Generate a new unique filament_id
    let new_id = format!(
        "BambuMate_{}_{:x}",
        new_name.replace(' ', "_"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    // Update name and ID
    profile.set_string("name", new_name.clone());
    profile.set_string("filament_id", new_id.clone());
    profile.set_string_array("filament_settings_id", vec![new_id.clone()]);

    // Determine output path
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    let user_dir = paths
        .user_filament_dir()
        .ok_or_else(|| "User filament directory not found".to_string())?;

    let filename = format!("{}.json", new_id);
    let target_path = user_dir.join(&filename);

    // Create metadata
    let metadata = ProfileMetadata {
        setting_id: new_id.clone(),
        ..ProfileMetadata::default()
    };

    write_profile_with_metadata(&profile, &target_path, &metadata)
        .map_err(|e| format!("Failed to write duplicated profile: {}", e))?;

    info!("Duplicated profile to {:?} as '{}'", target_path, new_name);

    read_profile_command(target_path.to_string_lossy().to_string())
}

/// Extract FilamentSpecs from an existing profile for editing.
///
/// Reads the profile and maps BS profile fields back to the FilamentSpecs struct,
/// so the SpecsEditor UI can display and edit them.
#[tauri::command]
pub fn extract_specs_from_profile(path: String) -> Result<crate::scraper::types::FilamentSpecs, String> {
    let file_path = std::path::Path::new(&path);
    let profile = read_profile(file_path).map_err(|e| e.to_string())?;
    Ok(generator::extract_specs_from_profile(&profile))
}

/// Save edited FilamentSpecs back to an existing profile.
///
/// Reads the profile, applies the specs overrides (same mapping as generate),
/// and writes it back atomically. Returns the updated ProfileDetail.
#[tauri::command]
pub fn save_profile_specs(
    path: String,
    specs: crate::scraper::types::FilamentSpecs,
) -> Result<ProfileDetail, String> {
    let file_path = std::path::Path::new(&path);

    let mut profile = read_profile(file_path).map_err(|e| e.to_string())?;

    // Apply specs to the existing profile (overwrites only the mapped fields)
    generator::apply_specs_to_profile(&mut profile, &specs);

    // Also update the profile name if provided
    if !specs.name.is_empty() {
        profile.set_string("name", specs.name.clone());
    }

    write_profile_atomic(&profile, file_path)
        .map_err(|e| format!("Failed to write profile: {}", e))?;

    info!("Saved edited specs to {:?}", file_path);

    // Return updated detail
    read_profile_command(path)
}

/// A group of diffs for a single category.
#[derive(Debug, Clone, Serialize)]
pub struct DiffCategory {
    pub category: String,
    pub diffs: Vec<ProfileDiff>,
}

/// Result from comparing two profiles.
#[derive(Debug, Clone, Serialize)]
pub struct CompareResult {
    pub profile_a_name: String,
    pub profile_b_name: String,
    pub categories: Vec<DiffCategory>,
    pub total_fields: usize,
    pub changed_fields: usize,
}

/// Map a BS profile key to a display category.
fn key_to_category(key: &str) -> &'static str {
    match key {
        k if k.contains("temperature") || k.contains("temp") => "Temperature",
        k if k.contains("speed") || k.contains("flow") || k.contains("volumetric")
            || k.contains("acceleration") || k.contains("jerk") => "Speed & Flow",
        k if k.contains("fan") || k.contains("cool") || k.contains("slow_down") => "Cooling & Fan",
        k if k.contains("retract") || k.contains("wipe") || k.contains("z_hop") => "Retraction",
        k if k.contains("density") || k.contains("diameter") || k.contains("cost")
            || k.contains("vitrification") || k.contains("shrinkage") => "Physical Properties",
        k if k.contains("name") || k.contains("id") || k.contains("version")
            || k.contains("inherits") || k.contains("from") || k.contains("vendor")
            || k.contains("type") || k.contains("compatible") || k.contains("setting")
            || k.contains("instantiation") => "Identity & Metadata",
        _ => "Other",
    }
}

/// Compare two profiles side-by-side, returning differences grouped by category.
#[tauri::command]
pub fn compare_profiles(
    path_a: String,
    path_b: String,
    show_identical: bool,
) -> Result<CompareResult, String> {
    let profile_a = read_profile(std::path::Path::new(&path_a)).map_err(|e| e.to_string())?;
    let profile_b = read_profile(std::path::Path::new(&path_b)).map_err(|e| e.to_string())?;

    let raw_a = profile_a.raw();
    let raw_b = profile_b.raw();

    // Collect all keys from both profiles
    let mut all_keys: Vec<String> = raw_a
        .keys()
        .chain(raw_b.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    all_keys.sort();

    let total_fields = all_keys.len();
    let mut changed_fields = 0;

    // Build diffs grouped by category
    let mut category_map: std::collections::BTreeMap<&str, Vec<ProfileDiff>> =
        std::collections::BTreeMap::new();

    for key in &all_keys {
        let val_a = raw_a.get(key);
        let val_b = raw_b.get(key);
        let display_a = value_to_display(val_a);
        let display_b = value_to_display(val_b);

        let is_different = display_a != display_b;
        if is_different {
            changed_fields += 1;
        }

        if is_different || show_identical {
            let cat = key_to_category(key);
            category_map
                .entry(cat)
                .or_default()
                .push(ProfileDiff {
                    key: key.clone(),
                    label: key_to_label(key),
                    base_value: display_a,
                    new_value: display_b,
                });
        }
    }

    let categories: Vec<DiffCategory> = category_map
        .into_iter()
        .map(|(cat, diffs)| DiffCategory {
            category: cat.to_string(),
            diffs,
        })
        .collect();

    Ok(CompareResult {
        profile_a_name: profile_a.name().unwrap_or("<unnamed>").to_string(),
        profile_b_name: profile_b.name().unwrap_or("<unnamed>").to_string(),
        categories,
        total_fields,
        changed_fields,
    })
}

/// Compare two profiles field-by-field and return a list of differences.
///
/// Skips identity/metadata fields that always differ (name, filament_id, etc.)
/// and only reports printing-relevant setting changes.
fn compute_profile_diffs(base: &FilamentProfile, generated: &FilamentProfile) -> Vec<ProfileDiff> {
    // Fields to skip — these are identity/metadata, not actual settings
    let skip_fields: &[&str] = &[
        "name",
        "filament_id",
        "filament_settings_id",
        "setting_id",
        "from",
        "inherits",
        "instantiation",
        "compatible_printers",
        "compatible_printers_condition",
        "filament_vendor",
        "filament_type",
        "version",
    ];

    let mut diffs = Vec::new();
    let base_raw = base.raw();
    let gen_raw = generated.raw();

    for (key, gen_value) in gen_raw.iter() {
        if skip_fields.contains(&key.as_str()) {
            continue;
        }

        let base_value = base_raw.get(key);
        let base_str = value_to_display(base_value);
        let gen_str = value_to_display(Some(gen_value));

        if base_str != gen_str {
            diffs.push(ProfileDiff {
                key: key.clone(),
                label: key_to_label(key),
                base_value: base_str,
                new_value: gen_str,
            });
        }
    }

    // Sort by label for consistent display
    diffs.sort_by(|a, b| a.label.cmp(&b.label));
    diffs
}

/// Convert a JSON value to a human-readable display string.
/// For arrays, shows the first element (since dual-extruder arrays repeat the same value).
fn value_to_display(value: Option<&serde_json::Value>) -> String {
    match value {
        None => "--".to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(serde_json::Value::Null) => "--".to_string(),
        Some(serde_json::Value::Array(arr)) => {
            // Show first element for dual-extruder arrays
            arr.first()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "[]".to_string())
        }
        Some(serde_json::Value::Object(_)) => "{...}".to_string(),
    }
}

/// Convert a snake_case profile key to a human-readable label.
fn key_to_label(key: &str) -> String {
    key.replace('_', " ")
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
