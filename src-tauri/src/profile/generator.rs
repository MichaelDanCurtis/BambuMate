use anyhow::{anyhow, Result};
use chrono::Utc;
use tracing::debug;

use crate::process_command;
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

    // === Nozzle temperatures ===
    // Prefer explicit nozzle_temperature if available, fall back to range max
    if let Some(temp) = specs.nozzle_temperature.or(specs.nozzle_temp_max) {
        set_dual(profile, "nozzle_temperature", temp.to_string());
    }
    if let Some(temp) = specs.nozzle_temperature_initial_layer.or(specs
        .nozzle_temperature
        .map(|t| t + 5)
        .or(specs.nozzle_temp_max.map(|t| t + 5)))
    {
        set_dual(
            profile,
            "nozzle_temperature_initial_layer",
            temp.to_string(),
        );
    }
    // Range bounds for BS temperature slider
    if let Some(temp_max) = specs.nozzle_temp_max {
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

    // === Per-plate bed temperatures ===
    // Prefer explicit plate temps, fall back to bed_temp range
    if let Some(temp) = specs.hot_plate_temp.or(specs.bed_temp_max) {
        set_dual(profile, "hot_plate_temp", temp.to_string());
    }
    if let Some(temp) = specs
        .hot_plate_temp_initial_layer
        .or(specs.hot_plate_temp)
        .or(specs.bed_temp_max)
    {
        set_dual(profile, "hot_plate_temp_initial_layer", temp.to_string());
    }
    if let Some(temp) = specs.cool_plate_temp.or(specs.bed_temp_min) {
        set_dual(profile, "cool_plate_temp", temp.to_string());
    }
    if let Some(temp) = specs
        .cool_plate_temp_initial_layer
        .or(specs.cool_plate_temp)
        .or(specs.bed_temp_min)
    {
        set_dual(profile, "cool_plate_temp_initial_layer", temp.to_string());
    }
    if let Some(temp) = specs.eng_plate_temp.or(specs.bed_temp_max) {
        set_dual(profile, "eng_plate_temp", temp.to_string());
    }
    if let Some(temp) = specs
        .eng_plate_temp_initial_layer
        .or(specs.eng_plate_temp)
        .or(specs.bed_temp_max)
    {
        set_dual(profile, "eng_plate_temp_initial_layer", temp.to_string());
    }
    if let Some(temp) = specs
        .textured_plate_temp
        .or(specs.bed_temp_min.map(|t| t.saturating_sub(5)))
    {
        set_dual(profile, "textured_plate_temp", temp.to_string());
    }
    if let Some(temp) = specs
        .textured_plate_temp_initial_layer
        .or(specs.textured_plate_temp)
        .or(specs.bed_temp_min.map(|t| t.saturating_sub(5)))
    {
        set_dual(
            profile,
            "textured_plate_temp_initial_layer",
            temp.to_string(),
        );
    }

    // === Flow & volumetric speed ===
    if let Some(mvs) = specs.max_volumetric_speed {
        set_dual(
            profile,
            "filament_max_volumetric_speed",
            format!("{:.0}", mvs),
        );
    }
    if let Some(ratio) = specs.filament_flow_ratio {
        set_dual(profile, "filament_flow_ratio", format!("{:.2}", ratio));
    }
    if let Some(pa) = specs.pressure_advance {
        set_dual(profile, "pressure_advance", format!("{:.3}", pa));
    }

    // === Fan/cooling ===
    // Prefer explicit fan_min/max, fall back to legacy fan_speed_percent
    if let Some(fan_max) = specs.fan_max_speed.or(specs.fan_speed_percent) {
        set_dual(profile, "fan_max_speed", fan_max.to_string());
    }
    if let Some(fan_min) = specs
        .fan_min_speed
        .or(specs.fan_speed_percent.map(|f| (f as f32 * 0.6) as u8))
    {
        set_dual(profile, "fan_min_speed", fan_min.to_string());
    }
    if let Some(overhang) = specs.overhang_fan_speed {
        set_dual(profile, "overhang_fan_speed", overhang.to_string());
    }
    if let Some(layers) = specs.close_fan_the_first_x_layers {
        set_dual(profile, "close_fan_the_first_x_layers", layers.to_string());
    }
    if let Some(aux) = specs.additional_cooling_fan_speed {
        set_dual(profile, "additional_cooling_fan_speed", aux.to_string());
    }

    // === Cooling slowdown ===
    if let Some(time) = specs.slow_down_layer_time {
        set_dual(profile, "slow_down_layer_time", time.to_string());
    }
    if let Some(speed) = specs.slow_down_min_speed {
        set_dual(profile, "slow_down_min_speed", speed.to_string());
    }

    // === Retraction ===
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
    if let Some(speed) = specs.deretraction_speed_mm_s {
        set_dual(profile, "filament_deretraction_speed", speed.to_string());
    }

    // === Bridge ===
    if let Some(speed) = specs.bridge_speed {
        set_dual(profile, "filament_bridge_speed", speed.to_string());
    }

    // === Physical properties ===
    if let Some(density) = specs.density_g_cm3 {
        set_dual(profile, "filament_density", format!("{:.2}", density));
    }
    if let Some(vitrification) = specs.temperature_vitrification {
        set_dual(
            profile,
            "temperature_vitrification",
            vitrification.to_string(),
        );
    }
    if let Some(cost) = specs.filament_cost {
        set_dual(profile, "filament_cost", format!("{:.2}", cost));
    }

    // Material identity (always set, not optional)
    set_dual(profile, "filament_type", specs.material.clone());
    set_dual(profile, "filament_vendor", specs.brand.clone());
}

/// Extract FilamentSpecs from an existing Bambu Studio profile.
///
/// This is the reverse of `apply_specs_to_profile`: it reads BS profile fields
/// and maps them back into a `FilamentSpecs` struct so the user can view/edit
/// them through the SpecsEditor UI.
pub fn extract_specs_from_profile(profile: &FilamentProfile) -> FilamentSpecs {
    // Helper: get first element of a dual-extruder string array and parse it
    let get_u16 = |key: &str| -> Option<u16> {
        profile
            .get_string_array(key)
            .and_then(|arr| arr.first().and_then(|s| s.parse().ok()))
    };
    let get_u8 = |key: &str| -> Option<u8> {
        profile
            .get_string_array(key)
            .and_then(|arr| arr.first().and_then(|s| s.parse().ok()))
    };
    let get_f32 = |key: &str| -> Option<f32> {
        profile
            .get_string_array(key)
            .and_then(|arr| arr.first().and_then(|s| s.parse().ok()))
    };
    let get_str = |key: &str| -> String {
        profile
            .get_string_array(key)
            .and_then(|arr| arr.first().map(|s| s.to_string()))
            .or_else(|| {
                profile
                    .raw()
                    .get(key)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default()
    };

    FilamentSpecs {
        serial: {
            // Derive serial from full profile name by stripping brand+material
            let full_name = profile.name().unwrap_or("").to_string();
            crate::scraper::html_extractor::infer_serial(&full_name)
        },
        brand: get_str("filament_vendor"),
        material: get_str("filament_type"),

        nozzle_temp_min: get_u16("nozzle_temperature_range_low"),
        nozzle_temp_max: get_u16("nozzle_temperature_range_high").map(|v| v.saturating_sub(20)),
        bed_temp_min: get_u16("cool_plate_temp"),
        bed_temp_max: get_u16("hot_plate_temp"),

        nozzle_temperature: get_u16("nozzle_temperature"),
        nozzle_temperature_initial_layer: get_u16("nozzle_temperature_initial_layer"),

        hot_plate_temp: get_u16("hot_plate_temp"),
        hot_plate_temp_initial_layer: get_u16("hot_plate_temp_initial_layer"),
        cool_plate_temp: get_u16("cool_plate_temp"),
        cool_plate_temp_initial_layer: get_u16("cool_plate_temp_initial_layer"),
        eng_plate_temp: get_u16("eng_plate_temp"),
        eng_plate_temp_initial_layer: get_u16("eng_plate_temp_initial_layer"),
        textured_plate_temp: get_u16("textured_plate_temp"),
        textured_plate_temp_initial_layer: get_u16("textured_plate_temp_initial_layer"),

        max_volumetric_speed: get_f32("filament_max_volumetric_speed"),
        filament_flow_ratio: get_f32("filament_flow_ratio"),
        pressure_advance: get_f32("pressure_advance"),

        fan_min_speed: get_u8("fan_min_speed"),
        fan_max_speed: get_u8("fan_max_speed"),
        overhang_fan_speed: get_u8("overhang_fan_speed"),
        close_fan_the_first_x_layers: get_u8("close_fan_the_first_x_layers"),
        additional_cooling_fan_speed: get_u8("additional_cooling_fan_speed"),
        fan_speed_percent: None,

        slow_down_layer_time: get_u8("slow_down_layer_time"),
        slow_down_min_speed: get_u16("slow_down_min_speed"),

        retraction_distance_mm: get_f32("filament_retraction_length"),
        retraction_speed_mm_s: get_u16("filament_retraction_speed"),
        deretraction_speed_mm_s: get_u16("filament_deretraction_speed"),

        bridge_speed: get_u16("filament_bridge_speed"),

        density_g_cm3: get_f32("filament_density"),
        diameter_mm: get_f32("filament_diameter"),
        temperature_vitrification: get_u16("temperature_vitrification"),
        filament_cost: get_f32("filament_cost"),

        max_speed_mm_s: None,

        source_url: "profile".to_string(),
        extraction_confidence: 1.0,
    }
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
    base_profile_override: Option<&str>,
) -> Result<(FilamentProfile, ProfileMetadata, String)> {
    let material = MaterialType::from_str(&specs.material);
    let default_base = base_profile_name(&material);
    let base_name = base_profile_override.unwrap_or(default_base);

    debug!(
        "Generating profile for {} {} (material={:?}, base={})",
        specs.brand, specs.serial, material, base_name
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
    let profile_name = if specs.serial.is_empty() {
        format!("{} {} @{}", specs.brand, specs.material, printer)
    } else {
        format!("{} {} {} @{}", specs.brand, specs.material, specs.serial, printer)
    };

    profile.set_string("name", profile_name.clone());
    profile.set_string("inherits", String::new()); // Fully flattened
    profile.set_string("from", "User".to_string());
    profile.set_string("filament_id", generate_filament_id());
    profile.set_string("instantiation", "true".to_string());

    // 3. Set display identifier (single element matching profile name)
    profile.set_string_array("filament_settings_id", vec![profile_name.clone()]);

    // 4. Apply scraped spec overrides
    apply_specs_to_profile(&mut profile, specs);

    // 5. Apply compatibility defaults for fields required by newer Bambu Studio
    //    versions that may be absent from older system profile installations.
    apply_compat_defaults(&mut profile);

    // 6. Set compatible_printers to empty array for universal compatibility
    profile.set_string_array("compatible_printers", vec![]);

    // 7. Generate metadata
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

    // 8. Generate filename
    let filename = if specs.serial.is_empty() {
        format!("{} {} @{}.json", specs.brand, specs.material, printer)
    } else {
        format!("{} {} {} @{}.json", specs.brand, specs.material, specs.serial, printer)
    };

    debug!(
        "Generated profile '{}' with {} fields (base: {})",
        profile_name,
        profile.field_count(),
        base_name
    );

    Ok((profile, metadata, filename))
}

/// Apply compatibility fallback defaults for fields required by Bambu Studio 2.x.
///
/// Bambu Studio 2.x added several fields to `fdm_filament_common` that are absent
/// from older installations. If a user generates a profile against an older install
/// (or against system profiles that pre-date 2.x) these fields will be missing and
/// BS will reject the profile on import.
///
/// **Priority order** (highest → lowest):
/// 1. Values resolved from the installed system profile chain (set in step 1 of
///    `generate_profile` via `resolve_inheritance`).
/// 2. Values written by `apply_specs_to_profile` (scraped spec overrides).
/// 3. The defaults in this function — only applied when the key is absent.
///
/// ### Forward-compatibility
/// Most new fields added in future Bambu Studio versions are handled **automatically**
/// through two other mechanisms:
/// - Non-nil new fields in `fdm_filament_common` come through the ancestor merge in
///   `resolve_inheritance`.
/// - Nil-placeholder new fields in `fdm_filament_common` are preserved by the
///   nil-preservation pass in `resolve_inheritance`.
///
/// Add an entry here **only** when a required field is completely absent from all
/// levels of a system profile chain (i.e. it was back-ported as a new top-level
/// requirement without a corresponding entry in older `fdm_filament_common` files).
fn apply_compat_defaults(profile: &mut FilamentProfile) {
    use serde_json::{json, Value};

    // Fields added in Bambu Studio 2.x that may not appear in older system
    // profiles. Values match the defaults shipped with BS 2.7.
    let defaults = [
        ("default_filament_colour",      json!([""])),
        ("enable_overhang_bridge_fan",   json!(["1"])),
        ("enable_pressure_advance",      json!(["0"])),
        ("filament_change_length_nc",    json!(["4"])),
        ("filament_notes",               Value::String(String::new())),
        ("filament_wipe",                json!(["1", "1"])),
        ("first_x_layer_fan_speed",      json!(["0"])),
        ("first_x_layer_part_fan_speed", json!(["0"])),
    ];

    for (key, value) in defaults {
        if !profile.raw().contains_key(key) {
            profile.raw_mut().insert(key.to_string(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compat_defaults_added_when_fields_absent() {
        let mut profile = FilamentProfile::from_map(serde_json::Map::new());
        apply_compat_defaults(&mut profile);

        for key in &[
            "default_filament_colour",
            "enable_overhang_bridge_fan",
            "enable_pressure_advance",
            "filament_change_length_nc",
            "filament_notes",
            "filament_wipe",
            "first_x_layer_fan_speed",
            "first_x_layer_part_fan_speed",
        ] {
            assert!(profile.raw().contains_key(*key),
                "compat default for '{}' should be present", key);
        }

        assert_eq!(profile.raw()["enable_pressure_advance"], json!(["0"]));
        assert_eq!(profile.raw()["filament_wipe"], json!(["1", "1"]));
        assert_eq!(profile.raw()["first_x_layer_fan_speed"], json!(["0"]));
    }

    #[test]
    fn compat_defaults_do_not_overwrite_system_or_spec_values() {
        let mut profile = FilamentProfile::from_json(
            r#"{"enable_pressure_advance": ["1"], "filament_wipe": ["0", "0"]}"#
        ).unwrap();

        apply_compat_defaults(&mut profile);

        // Pre-existing values must not be touched
        assert_eq!(profile.raw()["enable_pressure_advance"], json!(["1"]),
            "system profile value must not be overwritten by compat default");
        assert_eq!(profile.raw()["filament_wipe"], json!(["0", "0"]),
            "spec-derived value must not be overwritten by compat default");
    }
}

/// Check if Bambu Studio is currently running.
///
/// Uses platform-specific process detection:
/// - macOS/Linux: `pgrep -f BambuStudio`
/// - Windows: `tasklist /FI` filtering for BambuStudio.exe
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

/// Check if Bambu Studio is currently running on Windows.
///
/// Uses `tasklist` to search for both possible process names.
#[cfg(target_os = "windows")]
pub fn is_bambu_studio_running() -> bool {
    for exe_name in &["BambuStudio.exe", "bambu-studio.exe"] {
        if let Ok(output) = process_command::new_command("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}", exe_name), "/NH"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains(exe_name) {
                return true;
            }
        }
    }
    false
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
