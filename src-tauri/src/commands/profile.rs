use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::profile::generator;
use crate::profile::inheritance::resolve_inheritance;
use crate::profile::paths::BambuPaths;
use crate::profile::reader::{read_profile, read_profile_metadata};
use crate::profile::registry::ProfileRegistry;
use crate::profile::sync::{write_profile_edit, write_profile_new};
use crate::profile::types::{FilamentProfile, ProfileMetadata};
use crate::profile::writer::register_filament_in_conf;

const DEFAULT_TARGET_PRINTER_LABEL: &str = "Bambu Lab H2C 0.4 nozzle";
const DEFAULT_TARGET_PRINTER_MODEL: &str = "H2C";
const DEFAULT_NOZZLE_SIZE: &str = "0.4";
const FALLBACK_TARGET_PRINTER_LABELS: &[&str] = &[
    DEFAULT_TARGET_PRINTER_LABEL,
    "Bambu Lab H2D 0.4 nozzle",
    "Bambu Lab X1 Carbon 0.4 nozzle",
    "Bambu Lab X1E 0.4 nozzle",
    "Bambu Lab P1P 0.4 nozzle",
    "Bambu Lab P1S 0.4 nozzle",
    "Bambu Lab A1 0.4 nozzle",
    "Bambu Lab A1 mini 0.4 nozzle",
    "Bambu Lab X1 Carbon 0.2 nozzle",
    "Bambu Lab X1 Carbon 0.6 nozzle",
    "Bambu Lab X1 Carbon 0.8 nozzle",
];

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

/// Printer and nozzle choices for generated profiles.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetPrinterOptions {
    pub printer_models: Vec<String>,
    pub nozzle_sizes: Vec<String>,
    pub default_printer_model: String,
    pub default_nozzle_size: String,
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

    for entry in WalkDir::new(&user_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        match read_profile(path) {
            Ok(profile) => {
                let name = profile.name().unwrap_or("<unnamed>").to_string();

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
            e.path().is_file() && e.path().extension().and_then(|ext| ext.to_str()) == Some("json")
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

    for entry in WalkDir::new(&system_dir).into_iter().filter_map(|e| e.ok()) {
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

/// List the Bambu printer models and nozzle sizes available for profile generation.
///
/// Discovers printer+nozzle combinations from installed Bambu Studio profiles and
/// falls back to a built-in set so the UI can always prompt explicitly.
#[tauri::command]
pub fn list_target_printer_options() -> Result<TargetPrinterOptions, String> {
    let mut labels = HashSet::new();

    if let Ok(paths) = BambuPaths::detect() {
        collect_target_printer_labels(&paths.system_filament_dir(), &mut labels);

        if let Some(user_dir) = paths.user_filament_dir() {
            collect_target_printer_labels(&user_dir, &mut labels);
        }
    }

    Ok(build_target_printer_options(&labels))
}

/// Result from profile generation (preview step, no files written).
#[derive(Debug, Clone, Serialize)]
pub struct GenerateResult {
    pub profile_name: String,
    pub filament_id: String,
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
///
/// `existing_filament_id` — when the caller already knows the `filament_id`
/// to use (e.g. the first profile in a multi-nozzle batch already resolved it),
/// pass it here so all variants of the same filament share the same ID.
/// When `None`, the command looks up the user filament directory for an existing
/// profile with the same brand/material/serial and reuses its ID if found,
/// falling back to generating a fresh ID only when none exists yet.
#[tauri::command]
pub async fn generate_profile_from_specs(
    specs: crate::scraper::types::FilamentSpecs,
    target_printer: Option<String>,
    base_profile_path: Option<String>,
    existing_filament_id: Option<String>,
) -> Result<GenerateResult, String> {
    info!(
        "generate_profile_from_specs called for: {} {}",
        specs.brand, specs.serial
    );

    // Detect Bambu Studio paths
    let paths = BambuPaths::detect().map_err(|e| {
        format!(
            "Bambu Studio not found: {}. Please install Bambu Studio first.",
            e
        )
    })?;

    // Build registry from system + user filament profiles
    let system_dir = paths.system_filament_dir();
    if !system_dir.exists() {
        return Err(format!(
            "System filament directory not found at {:?}. Is Bambu Studio installed correctly?",
            system_dir
        ));
    }

    let mut registry = ProfileRegistry::discover_system_profiles(&system_dir)
        .map_err(|e| format!("Failed to load system profiles: {}", e))?;
    let user_dir = paths.user_filament_dir();
    if let Some(ref ud) = user_dir {
        if ud.exists() {
            registry
                .discover_user_profiles(ud)
                .map_err(|e| format!("Failed to load user profiles: {}", e))?;
        }
    }

    // Determine the filament_id to use:
    //   1. Caller-supplied (takes priority — already resolved for this batch)
    //   2. Found on disk (same filament, different nozzle/printer already installed)
    //   3. Fresh random ID (first time this filament is ever generated)
    let resolved_filament_id = existing_filament_id.or_else(|| {
        user_dir.as_ref().and_then(|ud| {
            generator::find_existing_filament_id(&specs.brand, &specs.material, &specs.serial, ud)
        })
    });

    // Determine the base profile to use (selected path or default by material).
    let material = crate::scraper::types::MaterialType::from_str(&specs.material);
    let default_base_name = generator::base_profile_name(&material).to_string();
    let (base_name, base_resolved) = if let Some(path) = base_profile_path
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let selected = read_profile(std::path::Path::new(&path))
            .map_err(|e| format!("Failed to read selected base profile: {}", e))?;
        let selected_name = selected
            .name()
            .ok_or_else(|| "Selected base profile has no name".to_string())?
            .to_string();
        let resolved = resolve_inheritance(&selected, &registry).map_err(|e| {
            format!(
                "Failed to resolve selected base profile '{}': {}",
                selected_name, e
            )
        })?;
        registry.insert(selected);
        (selected_name, resolved)
    } else {
        let base = registry.get_by_name(&default_base_name).ok_or_else(|| {
            format!(
                "Base profile '{}' not found in registry. Is Bambu Studio installed with system profiles?",
                default_base_name
            )
        })?;
        let resolved = resolve_inheritance(base, &registry).map_err(|e| {
            format!(
                "Failed to resolve base profile '{}': {}",
                default_base_name, e
            )
        })?;
        (default_base_name, resolved)
    };

    // Generate the profile
    let (profile, metadata, filename) = generator::generate_profile(
        &specs,
        &registry,
        target_printer.as_deref(),
        Some(base_name.as_str()),
        resolved_filament_id,
    )
    .map_err(|e| format!("Failed to generate profile: {}", e))?;

    // Capture the filament_id that was used (for the caller to propagate in batches)
    let filament_id = profile.filament_id().unwrap_or("").to_string();

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

    // Tell the user when the generator throttled flow for a small nozzle, so a
    // profile that silently differs from the requested specs (or from the base
    // profile it inherited from) is never a surprise.
    if let Some(diameter) = target_printer
        .as_deref()
        .or(Some(DEFAULT_TARGET_PRINTER_LABEL))
        .and_then(crate::profile::nozzle::parse_nozzle_diameter)
    {
        let cap = crate::profile::nozzle::max_volumetric_speed_cap(diameter);
        let requested = specs.max_volumetric_speed.or_else(|| {
            base_resolved
                .get_first_array_value("filament_max_volumetric_speed")
                .and_then(|v| v.trim().parse::<f32>().ok())
        });
        if let Some(requested) = requested {
            if requested > cap {
                warnings.push(format!(
                    "Max volumetric flow reduced from {:.0} to {:.0} mm³/s — {:.1} mm nozzles cannot sustain more.",
                    requested, cap, diameter
                ));
            }
        }
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

    let profile_name = profile.name().unwrap_or("<unnamed>").to_string();

    info!(
        "Generated profile '{}' with {} fields, {} diffs from base (base: {}, filament_id: {})",
        profile_name,
        profile.field_count(),
        diffs.len(),
        base_name,
        filament_id,
    );

    Ok(GenerateResult {
        profile_name,
        filament_id,
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
    let profile = FilamentProfile::from_json(&profile_json)
        .map_err(|e| format!("Invalid profile JSON: {}", e))?;
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
        info!("Overwriting existing profile at {:?}", target_path);
    }

    // Write profile + metadata atomically. profile::sync decides the sync
    // fields: a fresh file is "new" to Bambu Studio (empty setting_id), and
    // replacing a preset that already has a cloud id keeps that id and marks
    // it "update", so the cloud copy is not duplicated.
    let outcome = write_profile_new(&profile, &target_path, &metadata.user_id)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
    // Best effort: a ledger failure is logged and never fails the install.
    crate::history::ledger::note_new_preset_write(&outcome, &target_path);

    let profile_name = profile.name().unwrap_or("<unnamed>").to_string();

    // Register the filament in BambuStudio.conf so it appears as visible/available.
    // The profile name in the filaments array is the file stem (filename without .json).
    let file_stem = target_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&filename);
    if let Err(e) = register_filament_in_conf(&paths.config_root, file_stem) {
        warn!(
            "Failed to register filament in BambuStudio.conf (profile still installed): {}",
            e
        );
    }

    info!("Installed profile '{}' to {:?}", profile_name, target_path);

    Ok(InstallResult {
        installed_path: target_path.to_string_lossy().to_string(),
        profile_name,
        bambu_studio_was_running: bs_running,
    })
}

/// Assert that `path` resolves to a location inside the user filament
/// directory. Rejects both `..` traversal and absolute paths outside the
/// allowed root. Returns the canonical target path on success.
///
/// Used by all mutation commands (`update_profile_field`, `save_profile_specs`,
/// `duplicate_profile`, `delete_profile`) to prevent a compromised renderer
/// or a frontend bug from rewriting arbitrary files on disk.
///
/// If `must_exist` is false the target itself is allowed to be missing (used
/// by `duplicate_profile` writing a new file); the parent directory is
/// canonicalised instead.
fn assert_in_user_filament_dir(
    file_path: &std::path::Path,
    must_exist: bool,
) -> Result<std::path::PathBuf, String> {
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    let user_dir = paths
        .user_filament_dir()
        .ok_or_else(|| "User filament directory not found".to_string())?;
    crate::profile::paths::ensure_within(&user_dir, file_path, must_exist)
}

/// Delete a user filament profile and its companion .info file.
///
/// Safety: Validates that the path is within the user filament directory
/// to prevent deletion of arbitrary files.
#[tauri::command]
pub fn delete_profile(path: String) -> Result<(), String> {
    let file_path = std::path::Path::new(&path);
    // The canonical path doubles as the ledger key; take it before the file
    // is gone and can no longer be canonicalised.
    let canonical = assert_in_user_filament_dir(file_path, true)?;

    // Delete the JSON file
    std::fs::remove_file(&file_path).map_err(|e| format!("Failed to delete profile: {}", e))?;

    // Delete companion .info file if it exists
    let info_path = file_path.with_extension("info");
    if info_path.exists() {
        if let Err(e) = std::fs::remove_file(&info_path) {
            info!("Could not delete companion .info file: {}", e);
        }
    }

    crate::history::ledger::forget_preset(&crate::history::ledger::ledger_key(&canonical));

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
    assert_in_user_filament_dir(file_path, true)?;

    let mut profile = read_profile(file_path).map_err(|e| e.to_string())?;

    // Parse value as JSON to support arrays, strings, numbers, etc.
    let json_value: serde_json::Value =
        serde_json::from_str(&value).map_err(|e| format!("Invalid JSON value: {}", e))?;

    profile.raw_mut().insert(key.clone(), json_value);

    let outcome = write_profile_edit(&profile, file_path)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
    // Best effort: records the preset if the edit left it new to Bambu Studio.
    crate::history::ledger::note_new_preset_write(&outcome, file_path);

    info!("Updated field '{}' in {:?}", key, file_path);

    // Return updated detail
    read_profile_command(path)
}

/// The name and file for a duplicate called `new_name`, following the
/// generator's convention: the file is `<name>.json` and the name doubles as
/// `filament_settings_id`. If that file already exists, " (2)", " (3)", …
/// is appended to the name (and so to the file), so a duplicate never
/// overwrites a preset or shares its name. Path separators become `_` in the
/// file name only.
fn duplicate_target(user_dir: &std::path::Path, new_name: &str) -> (String, std::path::PathBuf) {
    let file_for = |name: &str| user_dir.join(format!("{}.json", name.replace(['/', '\\'], "_")));
    let mut name = new_name.to_string();
    let mut n = 2;
    while file_for(&name).exists() {
        name = format!("{new_name} ({n})");
        n += 1;
    }
    let path = file_for(&name);
    (name, path)
}

/// Duplicate a profile with a new name and IDs.
///
/// Copies the profile, gives it a fresh generated `filament_id` and the new
/// name (as `name` and `filament_settings_id`, the way the generator does),
/// and writes it to the user filament directory as a new preset.
#[tauri::command]
pub fn duplicate_profile(path: String, new_name: String) -> Result<ProfileDetail, String> {
    let paths = BambuPaths::detect().map_err(|e| format!("Bambu Studio not found: {}", e))?;
    let user_dir = paths
        .user_filament_dir()
        .ok_or_else(|| "User filament directory not found".to_string())?;
    let fallback_user_id = paths.preset_folder.clone().unwrap_or_default();

    let (target_path, outcome) = duplicate_into(
        std::path::Path::new(&path),
        &user_dir,
        &new_name,
        &fallback_user_id,
    )?;
    crate::history::ledger::note_new_preset_write(&outcome, &target_path);

    read_profile_command(target_path.to_string_lossy().to_string())
}

/// The body of [`duplicate_profile`], against an explicit `user_dir`.
///
/// `setting_id` is left to [`write_profile_new`]: empty, so Bambu Studio
/// uploads the copy and the cloud assigns its id. The old code wrote the file
/// stem there, which Bambu Studio read as "already synced" and never
/// uploaded, and made up a `BambuMate_<name>_<ms>` filament id and file name.
fn duplicate_into(
    source: &std::path::Path,
    user_dir: &std::path::Path,
    new_name: &str,
    fallback_user_id: &str,
) -> Result<(std::path::PathBuf, crate::profile::sync::NewPresetWrite), String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("Enter a name for the copy".to_string());
    }
    let mut profile = read_profile(source).map_err(|e| e.to_string())?;

    let (name, target_path) = duplicate_target(user_dir, new_name);
    // Even though we construct target_path ourselves from user_dir, run it
    // through the shared guard so any future refactor that lets a caller
    // pass in a target path stays safe.
    crate::profile::paths::ensure_within(user_dir, &target_path, false)?;

    profile.set_string("name", name.clone());
    profile.set_string("filament_id", generator::generate_filament_id());
    profile.set_string_array("filament_settings_id", vec![name.clone()]);

    let outcome = write_profile_new(&profile, &target_path, fallback_user_id)
        .map_err(|e| format!("Failed to write duplicated profile: {}", e))?;
    info!("Duplicated profile to {:?} as '{}'", target_path, name);
    Ok((target_path, outcome))
}

/// Extract FilamentSpecs from an existing profile for editing.
///
/// Reads the profile and maps BS profile fields back to the FilamentSpecs struct,
/// so the SpecsEditor UI can display and edit them.
#[tauri::command]
pub fn extract_specs_from_profile(
    path: String,
) -> Result<crate::scraper::types::FilamentSpecs, String> {
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
    assert_in_user_filament_dir(file_path, true)?;

    let mut profile = read_profile(file_path).map_err(|e| e.to_string())?;

    // Apply specs to the existing profile (overwrites only the mapped fields)
    generator::apply_specs_to_profile(&mut profile, &specs);

    // The profile is already bound to a printer + nozzle, so re-apply the
    // nozzle's physical limits: hand-edited specs describe the filament and can
    // easily ask a small nozzle for a flow it cannot deliver.
    let nozzle_label = profile
        .compatible_printers()
        .and_then(|printers| printers.first().map(|p| p.to_string()))
        .or_else(|| profile.name().map(|n| n.to_string()))
        .unwrap_or_default();
    if let Some(diameter) = crate::profile::nozzle::parse_nozzle_diameter(&nozzle_label) {
        for adjustment in crate::profile::nozzle::apply_nozzle_limits(&mut profile, diameter) {
            info!(
                "Clamped {} for {}mm nozzle: {} -> {}",
                adjustment.field, diameter, adjustment.from, adjustment.to
            );
        }
    }

    // Recompose the full profile name from brand + material + serial, preserving @printer suffix
    let existing_name = profile.name().unwrap_or("").to_string();
    let printer_suffix = existing_name
        .find(" @")
        .map(|i| existing_name[i..].to_string())
        .unwrap_or_default();
    let new_name = if specs.serial.is_empty() {
        format!("{} {}{}", specs.brand, specs.material, printer_suffix)
    } else {
        format!(
            "{} {} {}{}",
            specs.brand, specs.material, specs.serial, printer_suffix
        )
    };
    profile.set_string("name", new_name);

    let outcome = write_profile_edit(&profile, file_path)
        .map_err(|e| format!("Failed to write profile: {}", e))?;
    // Best effort: records the preset if the edit left it new to Bambu Studio.
    crate::history::ledger::note_new_preset_write(&outcome, file_path);

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
        k if k.contains("speed")
            || k.contains("flow")
            || k.contains("volumetric")
            || k.contains("acceleration")
            || k.contains("jerk") =>
        {
            "Speed & Flow"
        }
        k if k.contains("fan") || k.contains("cool") || k.contains("slow_down") => "Cooling & Fan",
        k if k.contains("retract") || k.contains("wipe") || k.contains("z_hop") => "Retraction",
        k if k.contains("density")
            || k.contains("diameter")
            || k.contains("cost")
            || k.contains("vitrification")
            || k.contains("shrinkage") =>
        {
            "Physical Properties"
        }
        k if k.contains("name")
            || k.contains("id")
            || k.contains("version")
            || k.contains("inherits")
            || k.contains("from")
            || k.contains("vendor")
            || k.contains("type")
            || k.contains("compatible")
            || k.contains("setting")
            || k.contains("instantiation") =>
        {
            "Identity & Metadata"
        }
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
            category_map.entry(cat).or_default().push(ProfileDiff {
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

/// A base profile match from Bambu Studio system profiles.
#[derive(Debug, Clone, Serialize)]
pub struct BaseProfileMatch {
    pub name: String,
    pub path: String,
    pub filament_type: Option<String>,
}

/// Search Bambu Studio's system profiles for filaments matching a query string.
/// Searches by name and material type. Returns up to 20 matches.
///
/// Performance: results are served from an in-memory index of every filament
/// profile on disk (system + user). The index is built once on first use and
/// reused for 5 minutes; subsequent calls skip disk I/O entirely and only
/// filter/sort the cached entries. Call `refresh_base_profile_index` after
/// installing/removing profiles to force an immediate rebuild.
#[tauri::command]
pub fn search_base_profiles(
    query: String,
    material_type: Option<String>,
) -> Result<Vec<BaseProfileMatch>, String> {
    info!(
        "Searching installed profiles for: {} (material: {:?})",
        query, material_type
    );

    let index = get_or_build_base_profile_index()?;

    let query_lower = query.to_lowercase();
    let material_lower = material_type.as_deref().map(|m| m.to_lowercase());

    let mut matches: Vec<BaseProfileMatch> = index
        .iter()
        .filter(|e| match material_lower.as_deref() {
            Some(m) if !m.is_empty() => e.ftype_lower.contains(m),
            _ => true,
        })
        .filter(|e| {
            query_lower.is_empty()
                || e.name_lower.contains(&query_lower)
                || e.ftype_lower.contains(&query_lower)
        })
        .map(|e| BaseProfileMatch {
            name: e.name.clone(),
            path: e.path.clone(),
            filament_type: e.filament_type.clone(),
        })
        .collect();

    // Dedupe by path (user dir can shadow a system profile with the same name).
    let mut seen_paths = HashSet::new();
    matches.retain(|m| seen_paths.insert(m.path.clone()));

    matches.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    matches.truncate(20);

    info!("Found {} matching base profiles", matches.len());
    Ok(matches)
}

/// Force a rebuild of the base-profile index on the next `search_base_profiles`
/// call. Cheap to call — this only drops the cache; the next search will
/// rebuild lazily.
#[tauri::command]
pub fn refresh_base_profile_index() -> Result<(), String> {
    if let Ok(mut guard) = base_profile_cache().lock() {
        *guard = None;
        info!("Base-profile index invalidated");
    }
    Ok(())
}

fn collect_target_printer_labels(dir: &std::path::Path, labels: &mut HashSet<String>) {
    if !dir.exists() {
        return;
    }

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let Ok(profile) = read_profile(path) else {
            continue;
        };

        let Some(compatible_printers) = profile.compatible_printers() else {
            continue;
        };

        for printer in compatible_printers {
            if parse_target_printer_label(printer).is_some() {
                labels.insert(printer.to_string());
            }
        }
    }
}

fn build_target_printer_options(discovered_labels: &HashSet<String>) -> TargetPrinterOptions {
    let mut all_labels = discovered_labels.clone();
    for label in FALLBACK_TARGET_PRINTER_LABELS {
        all_labels.insert((*label).to_string());
    }

    let mut printer_models = BTreeSet::new();
    let mut nozzle_sizes = BTreeSet::new();

    for label in &all_labels {
        if let Some((printer_model, nozzle_size)) = parse_target_printer_label(label) {
            printer_models.insert(printer_model);
            nozzle_sizes.insert(nozzle_size);
        }
    }

    printer_models.insert(DEFAULT_TARGET_PRINTER_MODEL.to_string());
    nozzle_sizes.insert(DEFAULT_NOZZLE_SIZE.to_string());

    TargetPrinterOptions {
        printer_models: prioritize_default(
            printer_models.into_iter().collect(),
            DEFAULT_TARGET_PRINTER_MODEL,
        ),
        nozzle_sizes: prioritize_default(nozzle_sizes.into_iter().collect(), DEFAULT_NOZZLE_SIZE),
        default_printer_model: DEFAULT_TARGET_PRINTER_MODEL.to_string(),
        default_nozzle_size: DEFAULT_NOZZLE_SIZE.to_string(),
    }
}

fn parse_target_printer_label(label: &str) -> Option<(String, String)> {
    let trimmed = label.trim();
    let without_prefix = trimmed.strip_prefix("Bambu Lab ")?;
    let without_suffix = without_prefix.strip_suffix(" nozzle")?;
    let (printer_model, nozzle_size) = without_suffix.rsplit_once(' ')?;

    if printer_model.is_empty() || nozzle_size.is_empty() {
        return None;
    }

    Some((printer_model.to_string(), nozzle_size.to_string()))
}

fn prioritize_default(mut values: Vec<String>, default: &str) -> Vec<String> {
    values.sort();

    if let Some(index) = values.iter().position(|value| value == default) {
        let default_value = values.remove(index);
        values.insert(0, default_value);
    }

    values
}

// ---------------------------------------------------------------------------
// Base-profile index (in-memory cache)
// ---------------------------------------------------------------------------

/// One row in the base-profile index. Pre-lowercased fields let each
/// keystroke do only string contains-checks instead of allocating.
#[derive(Debug, Clone)]
struct BaseProfileIndexEntry {
    name: String,
    name_lower: String,
    filament_type: Option<String>,
    ftype_lower: String,
    path: String,
}

struct BaseProfileCache {
    entries: Vec<BaseProfileIndexEntry>,
    built_at: Instant,
}

/// Rebuild the cache if it's older than this. Keeps long-running sessions
/// picking up newly-installed system profiles without a manual refresh.
const BASE_PROFILE_CACHE_TTL: Duration = Duration::from_secs(300);

fn base_profile_cache() -> &'static Mutex<Option<BaseProfileCache>> {
    static CACHE: std::sync::OnceLock<Mutex<Option<BaseProfileCache>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Return a snapshot of the base-profile index, rebuilding if empty or stale.
/// Cloned so the mutex isn't held while callers filter (keeps concurrent
/// searches from serialising on the lock).
fn get_or_build_base_profile_index() -> Result<Vec<BaseProfileIndexEntry>, String> {
    // Fast path: fresh cache, take a clone and go.
    {
        let guard = base_profile_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(cache) = guard.as_ref() {
            if cache.built_at.elapsed() < BASE_PROFILE_CACHE_TTL {
                return Ok(cache.entries.clone());
            }
        }
    }

    // Slow path: rebuild.
    let entries = build_base_profile_index()?;
    {
        let mut guard = base_profile_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some(BaseProfileCache {
            entries: entries.clone(),
            built_at: Instant::now(),
        });
    }
    Ok(entries)
}

/// One-shot walk of the two filament-profile roots. Callers should not hit
/// this in hot loops — use `get_or_build_base_profile_index` instead.
fn build_base_profile_index() -> Result<Vec<BaseProfileIndexEntry>, String> {
    let started = Instant::now();
    let paths = match BambuPaths::detect() {
        Ok(p) => p,
        Err(_) => {
            info!("Bambu Studio not detected, base-profile index is empty");
            return Ok(Vec::new());
        }
    };

    let mut roots: Vec<std::path::PathBuf> = Vec::with_capacity(2);
    let bbl_dir = paths
        .config_root
        .join("system")
        .join("BBL")
        .join("filament");
    if bbl_dir.exists() {
        roots.push(bbl_dir);
    } else {
        let alt = paths.config_root.join("system").join("filament");
        if alt.exists() {
            roots.push(alt);
        }
    }
    if let Some(user_dir) = paths.user_filament_dir() {
        if user_dir.exists() {
            roots.push(user_dir);
        }
    }

    let mut entries = Vec::new();
    for root in &roots {
        index_dir_into(root, &mut entries);
    }

    info!(
        "Built base-profile index: {} entries from {} root(s) in {:?}",
        entries.len(),
        roots.len(),
        started.elapsed()
    );
    Ok(entries)
}

fn index_dir_into(dir: &std::path::Path, out: &mut Vec<BaseProfileIndexEntry>) {
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(profile) = read_profile(path) else {
            continue;
        };
        let name = profile.name().unwrap_or("").to_string();
        // Skip nameless profiles — they'd never match a user query anyway.
        if name.is_empty() {
            continue;
        }
        let filament_type = profile.filament_type().map(|s| s.to_string());
        let name_lower = name.to_lowercase();
        let ftype_lower = filament_type.as_deref().unwrap_or("").to_lowercase();
        out.push(BaseProfileIndexEntry {
            name,
            name_lower,
            filament_type,
            ftype_lower,
            path: path.to_string_lossy().to_string(),
        });
    }
}

/// Filter a pre-built index the same way `search_base_profiles` does.
/// Extracted so unit tests can exercise the query semantics without touching
/// disk or the global cache.
#[cfg(test)]
fn filter_base_profile_index(
    index: &[BaseProfileIndexEntry],
    query: &str,
    material_type: Option<&str>,
) -> Vec<BaseProfileMatch> {
    let query_lower = query.to_lowercase();
    let material_lower = material_type.map(|m| m.to_lowercase());
    let mut matches: Vec<BaseProfileMatch> = index
        .iter()
        .filter(|e| match material_lower.as_deref() {
            Some(m) if !m.is_empty() => e.ftype_lower.contains(m),
            _ => true,
        })
        .filter(|e| {
            query_lower.is_empty()
                || e.name_lower.contains(&query_lower)
                || e.ftype_lower.contains(&query_lower)
        })
        .map(|e| BaseProfileMatch {
            name: e.name.clone(),
            path: e.path.clone(),
            filament_type: e.filament_type.clone(),
        })
        .collect();
    let mut seen = HashSet::new();
    matches.retain(|m| seen.insert(m.path.clone()));
    matches.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    matches.truncate(20);
    matches
}

#[cfg(test)]
mod tests {
    use super::{
        build_target_printer_options, duplicate_into, filter_base_profile_index,
        parse_target_printer_label, BaseProfileIndexEntry, DEFAULT_NOZZLE_SIZE,
        DEFAULT_TARGET_PRINTER_MODEL,
    };
    use crate::profile::reader::{read_profile, read_profile_metadata};
    use crate::profile::sync::NewPresetWrite;
    use std::collections::HashSet;

    #[test]
    fn duplicate_uses_generated_ids_and_names_the_file_after_the_preset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_dir = tmp.path().join("user/1881310893/filament/base");
        std::fs::create_dir_all(&user_dir).unwrap();
        let source = user_dir.join("Acme PLA.json");
        std::fs::write(
            &source,
            r#"{"name":"Acme PLA","filament_id":"P1234567","filament_settings_id":["Acme PLA"]}"#,
        )
        .unwrap();

        let (first, outcome) = duplicate_into(&source, &user_dir, "Acme PLA Copy", "").unwrap();
        let (second, _) = duplicate_into(&source, &user_dir, " Acme PLA Copy ", "").unwrap();

        assert_eq!(outcome, NewPresetWrite::Created);
        assert_eq!(first, user_dir.join("Acme PLA Copy.json"));
        assert_eq!(
            second,
            user_dir.join("Acme PLA Copy (2).json"),
            "never overwrites"
        );
        let copy = read_profile(&first).unwrap();
        assert_eq!(copy.name(), Some("Acme PLA Copy"));
        let id = copy.filament_id().unwrap();
        assert!(
            id.len() == 8 && id.starts_with('P') && id != "P1234567",
            "a generated filament id: {id}"
        );
        assert_eq!(
            copy.raw()["filament_settings_id"],
            serde_json::json!(["Acme PLA Copy"])
        );
        assert_eq!(
            read_profile(&second).unwrap().name(),
            Some("Acme PLA Copy (2)")
        );
        let meta = read_profile_metadata(&first).unwrap().unwrap();
        assert_eq!(meta.setting_id, "", "the cloud assigns the id");
        assert_eq!(meta.user_id, "1881310893");
        let files = std::fs::read_dir(&user_dir).unwrap().count();
        assert_eq!(files, 5, "source plus two copies, each with an .info");
        assert!(duplicate_into(&source, &user_dir, "  ", "").is_err());
    }

    fn entry(name: &str, ftype: &str, path: &str) -> BaseProfileIndexEntry {
        BaseProfileIndexEntry {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            filament_type: Some(ftype.to_string()),
            ftype_lower: ftype.to_lowercase(),
            path: path.to_string(),
        }
    }

    #[test]
    fn parses_bambu_printer_label() {
        let parsed = parse_target_printer_label("Bambu Lab X1 Carbon 0.6 nozzle");
        assert_eq!(parsed, Some(("X1 Carbon".to_string(), "0.6".to_string())));
    }

    #[test]
    fn rejects_non_bambu_printer_label() {
        assert_eq!(parse_target_printer_label("X1 Carbon 0.6 nozzle"), None);
    }

    #[test]
    fn includes_default_target_when_only_other_printers_are_discovered() {
        let labels = HashSet::from([String::from("Bambu Lab A1 0.8 nozzle")]);

        let options = build_target_printer_options(&labels);

        assert_eq!(options.default_printer_model, DEFAULT_TARGET_PRINTER_MODEL);
        assert_eq!(options.default_nozzle_size, DEFAULT_NOZZLE_SIZE);
        assert_eq!(
            options.printer_models.first().map(String::as_str),
            Some("H2C")
        );
        assert_eq!(
            options.nozzle_sizes.first().map(String::as_str),
            Some("0.4")
        );
        assert!(options.printer_models.iter().any(|printer| printer == "A1"));
        assert!(options.nozzle_sizes.iter().any(|nozzle| nozzle == "0.8"));
    }

    #[test]
    fn empty_query_returns_everything_up_to_20() {
        let idx: Vec<_> = (0..30)
            .map(|i| entry(&format!("PLA {}", i), "PLA", &format!("/tmp/p{}.json", i)))
            .collect();
        let hits = filter_base_profile_index(&idx, "", None);
        assert_eq!(hits.len(), 20);
    }

    #[test]
    fn query_is_case_insensitive_on_name_and_type() {
        let idx = vec![
            entry("Bambu PLA Basic", "PLA", "/a.json"),
            entry("Overture PETG", "PETG", "/b.json"),
            entry("Polymaker ASA", "ASA", "/c.json"),
        ];
        let by_name = filter_base_profile_index(&idx, "OVERTURE", None);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].name, "Overture PETG");

        let by_type = filter_base_profile_index(&idx, "pla", None);
        assert_eq!(by_type.len(), 1);
        assert_eq!(by_type[0].name, "Bambu PLA Basic");
    }

    #[test]
    fn material_filter_narrows_results() {
        let idx = vec![
            entry("Bambu PLA Basic", "PLA", "/a.json"),
            entry("Bambu PETG HF", "PETG", "/b.json"),
            entry("Bambu PLA Matte", "PLA", "/c.json"),
        ];
        let hits = filter_base_profile_index(&idx, "bambu", Some("PLA"));
        assert_eq!(hits.len(), 2);
        assert!(hits
            .iter()
            .all(|m| m.filament_type.as_deref() == Some("PLA")));
    }

    #[test]
    fn duplicates_by_path_are_removed() {
        let idx = vec![
            entry("Bambu PLA Basic", "PLA", "/same.json"),
            entry("Bambu PLA Basic", "PLA", "/same.json"),
        ];
        let hits = filter_base_profile_index(&idx, "", None);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn results_are_sorted_case_insensitively_by_name() {
        let idx = vec![
            entry("zeta", "PLA", "/z.json"),
            entry("Alpha", "PLA", "/a.json"),
            entry("beta", "PLA", "/b.json"),
        ];
        let hits = filter_base_profile_index(&idx, "", None);
        let names: Vec<&str> = hits.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "beta", "zeta"]);
    }

    #[test]
    fn empty_material_filter_is_ignored() {
        let idx = vec![
            entry("Bambu PLA Basic", "PLA", "/a.json"),
            entry("Overture PETG", "PETG", "/b.json"),
        ];
        let hits = filter_base_profile_index(&idx, "", Some(""));
        assert_eq!(hits.len(), 2);
    }
}
