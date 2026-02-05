use anyhow::{anyhow, Result};
use chrono::Utc;
use tracing::debug;

use super::inheritance::resolve_inheritance;
use super::paths::BambuPaths;
use super::registry::ProfileRegistry;
use super::types::{FilamentProfile, ProfileMetadata};
use crate::scraper::types::{FilamentSpecs, MaterialType};

/// Map a MaterialType to the corresponding Bambu Studio base profile name.
/// These are the "Generic X" profiles that ship with Bambu Studio.
pub fn base_profile_name(material: &MaterialType) -> &'static str {
    match material {
        MaterialType::PLA => "Generic PLA",
        MaterialType::PETG => "Generic PETG",
        MaterialType::ABS => "Generic ABS",
        MaterialType::ASA => "Generic ASA",
        MaterialType::TPU => "Generic TPU",
        MaterialType::Nylon => "Generic PA",
        MaterialType::PC => "Generic PC",
        MaterialType::PVA => "Generic PVA",
        MaterialType::HIPS => "Generic HIPS",
        MaterialType::Other(_) => "Generic PLA", // Safe fallback
    }
}

/// Generate a random filament_id in the format "P" + 7 hex chars.
///
/// User profiles use "P" prefix (not "GFL" which is for system profiles).
/// The 7 hex chars provide ~268M unique IDs, making collisions negligible.
pub fn generate_filament_id() -> String {
    let bytes: [u8; 4] = rand::random();
    format!("P{:07x}", u32::from_be_bytes(bytes) & 0x0FFF_FFFF)
}

/// Generate a random setting_id in the format "PFUS" + 14 hex chars.
///
/// User profiles use "PFUS" prefix (not "GFS" which is for system profiles).
/// Uses format!("{:02x}") per byte to avoid depending on the hex crate.
pub fn generate_setting_id() -> String {
    let bytes: [u8; 7] = rand::random();
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("PFUS{}", hex)
}

/// Apply scraped filament specs to a profile, overriding the base profile values.
///
/// All array fields are set with exactly 2 elements for dual-extruder compatibility.
/// This matches Bambu Studio's convention where every array field has one element
/// per extruder (H2C/H2D printers have dual extruders).
pub fn apply_specs_to_profile(profile: &mut FilamentProfile, specs: &FilamentSpecs) {
    // Helper: set a 2-element string array from a single value
    let set_dual = |p: &mut FilamentProfile, key: &str, val: String| {
        p.set_string_array(key, vec![val.clone(), val]);
    };

    // Nozzle temperature: use max as primary, min for range_low
    if let Some(temp_max) = specs.nozzle_temp_max {
        set_dual(profile, "nozzle_temperature", temp_max.to_string());
        // Initial layer: +5C is common convention for better first-layer adhesion
        set_dual(
            profile,
            "nozzle_temperature_initial_layer",
            (temp_max + 5).to_string(),
        );
        // Range high: max + 20 for safety margin in BS temperature slider
        set_dual(
            profile,
            "nozzle_temperature_range_high",
            (temp_max + 20).to_string(),
        );
    }
    if let Some(temp_min) = specs.nozzle_temp_min {
        set_dual(
            profile,
            "nozzle_temperature_range_low",
            temp_min.to_string(),
        );
    }

    // Bed temperature: max for primary surfaces, min for cool plate
    if let Some(bed_max) = specs.bed_temp_max {
        set_dual(profile, "bed_temperature", bed_max.to_string());
        set_dual(
            profile,
            "bed_temperature_initial_layer",
            bed_max.to_string(),
        );
        set_dual(profile, "hot_plate_temp", bed_max.to_string());
        set_dual(profile, "hot_plate_temp_initial_layer", bed_max.to_string());
        set_dual(profile, "eng_plate_temp", bed_max.to_string());
        set_dual(profile, "eng_plate_temp_initial_layer", bed_max.to_string());
    }
    if let Some(bed_min) = specs.bed_temp_min {
        set_dual(profile, "cool_plate_temp", bed_min.to_string());
        set_dual(
            profile,
            "cool_plate_temp_initial_layer",
            bed_min.to_string(),
        );
        // Textured plate: slightly lower than min, floored at 0
        let textured = bed_min.saturating_sub(5);
        set_dual(profile, "textured_plate_temp", textured.to_string());
        set_dual(
            profile,
            "textured_plate_temp_initial_layer",
            textured.to_string(),
        );
    }

    // Fan speed: convert percentage to Bambu Studio format
    if let Some(fan) = specs.fan_speed_percent {
        set_dual(profile, "fan_max_speed", format!("{}%", fan));
        // Min speed: 60% of max is a reasonable default
        let fan_min = (fan as f32 * 0.6) as u8;
        set_dual(profile, "fan_min_speed", format!("{}%", fan_min));
    }

    // Retraction
    if let Some(dist) = specs.retraction_distance_mm {
        set_dual(
            profile,
            "filament_retraction_length",
            format!("{:.1}", dist),
        );
    }
    if let Some(speed) = specs.retraction_speed_mm_s {
        set_dual(profile, "filament_retraction_speed", speed.to_string());
    }

    // Density
    if let Some(density) = specs.density_g_cm3 {
        set_dual(profile, "filament_density", format!("{:.2}", density));
    }

    // Material identity (always set, not optional)
    set_dual(profile, "filament_type", specs.material.clone());
    set_dual(profile, "filament_vendor", specs.brand.clone());
}

/// Generate a fully-flattened filament profile from scraped specifications.
///
/// This is the core value function: it takes a `FilamentSpecs` (from the scraper)
/// and produces a complete `FilamentProfile` ready for installation into Bambu Studio.
///
/// Steps:
/// 1. Determine material type and look up the corresponding base profile
/// 2. Resolve the base profile's inheritance chain to get all ~139 fields
/// 3. Set identity fields (name, filament_id, inherits="")
/// 4. Apply scraped spec overrides (temperatures, speeds, etc.)
/// 5. Generate metadata (.info file content)
///
/// Returns (profile, metadata, filename) tuple.
pub fn generate_profile(
    specs: &FilamentSpecs,
    registry: &ProfileRegistry,
    target_printer: Option<&str>,
) -> Result<(FilamentProfile, ProfileMetadata, String)> {
    let material = MaterialType::from_str(&specs.material);
    let base_name = base_profile_name(&material);

    debug!(
        "Generating profile for {} {} (material={:?}, base={})",
        specs.brand, specs.name, material, base_name
    );

    // 1. Find and resolve the base profile
    let base = registry.get_by_name(base_name).ok_or_else(|| {
        anyhow!(
            "Base profile '{}' not found in registry. Is Bambu Studio installed with system profiles?",
            base_name
        )
    })?;
    let mut profile = resolve_inheritance(base, registry)?;

    // 2. Set identity fields
    let printer = target_printer.unwrap_or("Bambu Lab H2C 0.4 nozzle");
    let profile_name = format!("{} {} {} @{}", specs.brand, specs.material, specs.name, printer);

    profile.set_string("name", profile_name.clone());
    profile.set_string("inherits", String::new()); // Fully flattened
    profile.set_string("from", "User".to_string());
    profile.set_string("filament_id", generate_filament_id());
    profile.set_string("instantiation", "true".to_string());

    // 3. Set display identifier (2-element array)
    profile.set_string_array(
        "filament_settings_id",
        vec![profile_name.clone(), profile_name.clone()],
    );

    // 4. Apply scraped spec overrides
    apply_specs_to_profile(&mut profile, specs);

    // 5. Set compatible_printers to empty array for universal compatibility
    profile.set_string_array("compatible_printers", vec![]);

    // 6. Generate metadata
    // user_id comes from BambuPaths.preset_folder in the calling context
    let paths = BambuPaths::detect().ok();
    let user_id = paths
        .and_then(|p| p.preset_folder.clone())
        .unwrap_or_default();

    let metadata = ProfileMetadata {
        sync_info: String::new(),
        user_id,
        setting_id: generate_setting_id(),
        base_id: String::new(),
        updated_time: Utc::now().timestamp() as u64,
    };

    // 7. Generate filename
    let filename = format!("{} {} {} @{}.json", specs.brand, specs.material, specs.name, printer);

    debug!(
        "Generated profile '{}' with {} fields (base: {})",
        profile_name,
        profile.field_count(),
        base_name
    );

    Ok((profile, metadata, filename))
}

/// Check if Bambu Studio is currently running.
///
/// Uses platform-specific process detection:
/// - macOS/Linux: `pgrep -f BambuStudio`
/// - Windows: stub (returns false)
///
/// This is a lightweight check using std::process::Command to avoid
/// adding the heavyweight sysinfo dependency.
#[cfg(target_os = "macos")]
pub fn is_bambu_studio_running() -> bool {
    std::process::Command::new("pgrep")
        .arg("-f")
        .arg("BambuStudio")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
pub fn is_bambu_studio_running() -> bool {
    false // Windows stub
}

#[cfg(target_os = "linux")]
pub fn is_bambu_studio_running() -> bool {
    std::process::Command::new("pgrep")
        .arg("-f")
        .arg("BambuStudio")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn is_bambu_studio_running() -> bool {
    false
}
