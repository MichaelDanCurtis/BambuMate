use std::path::PathBuf;

use bambumate_tauri::profile::*;
use bambumate_tauri::profile::reader::read_profile;
use bambumate_tauri::profile::writer::{write_profile_atomic, write_profile_with_metadata};
use bambumate_tauri::profile::types::ProfileMetadata;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_round_trip_preserves_all_fields() {
    let path = fixture_path("sample_profile.json");
    let profile = read_profile(&path).expect("Failed to read fixture");

    let original_count = profile.field_count();
    assert!(original_count > 30, "Fixture should have 30+ fields, got {}", original_count);

    // Serialize and re-parse
    let json = profile.to_json_4space().expect("Failed to serialize");
    let reparsed = FilamentProfile::from_json(&json).expect("Failed to re-parse");

    assert_eq!(
        original_count,
        reparsed.field_count(),
        "Field count changed after round-trip: {} -> {}",
        original_count,
        reparsed.field_count()
    );

    // Verify every key is present in reparsed
    for key in profile.raw().keys() {
        assert!(
            reparsed.raw().contains_key(key),
            "Key '{}' missing after round-trip",
            key
        );
        assert_eq!(
            profile.raw().get(key),
            reparsed.raw().get(key),
            "Value for key '{}' changed after round-trip",
            key
        );
    }
}

#[test]
fn test_round_trip_byte_identical() {
    let path = fixture_path("sample_profile.json");
    let raw_input = std::fs::read_to_string(&path).expect("Failed to read fixture file");

    let profile = FilamentProfile::from_json(&raw_input).expect("Failed to parse");
    let output = profile.to_json_4space().expect("Failed to serialize");

    assert_eq!(
        raw_input, output,
        "Round-trip produced different bytes.\n\
         Input length: {}, Output length: {}\n\
         First difference at byte: {}",
        raw_input.len(),
        output.len(),
        raw_input
            .bytes()
            .zip(output.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(std::cmp::min(raw_input.len(), output.len()))
    );
}

#[test]
fn test_nil_values_preserved() {
    let path = fixture_path("sample_profile.json");
    let raw_input = std::fs::read_to_string(&path).expect("Failed to read fixture");

    let profile = FilamentProfile::from_json(&raw_input).expect("Failed to parse");
    let output = profile.to_json_4space().expect("Failed to serialize");
    let reparsed = FilamentProfile::from_json(&output).expect("Failed to re-parse");

    // Check specific nil fields from the fixture
    let nil_fields = [
        "filament_deretraction_speed",
        "filament_retraction_minimum_travel",
        "filament_retraction_wipe",
        "filament_shrink",
        "filament_wipe_distance",
        "filament_z_hop",
        "filament_z_hop_types",
        "required_nozzle_HRC",
    ];

    for field in &nil_fields {
        let values = reparsed
            .get_string_array(field)
            .unwrap_or_else(|| panic!("Field '{}' missing after round-trip", field));
        assert_eq!(
            values,
            vec!["nil", "nil"],
            "Nil values in '{}' not preserved: {:?}",
            field,
            values
        );
    }
}

#[test]
fn test_dual_extruder_arrays_preserved() {
    let path = fixture_path("sample_profile.json");
    let raw_input = std::fs::read_to_string(&path).expect("Failed to read fixture");

    let profile = FilamentProfile::from_json(&raw_input).expect("Failed to parse");
    let output = profile.to_json_4space().expect("Failed to serialize");
    let reparsed = FilamentProfile::from_json(&output).expect("Failed to re-parse");

    // These fields should all have exactly 2 elements
    let dual_fields = [
        "nozzle_temperature",
        "nozzle_temperature_initial_layer",
        "bed_temperature",
        "filament_type",
        "filament_flow_ratio",
        "filament_retraction_length",
        "filament_retraction_speed",
        "fan_cooling_layer_time",
        "fan_max_speed",
        "fan_min_speed",
    ];

    for field in &dual_fields {
        let values = reparsed
            .get_string_array(field)
            .unwrap_or_else(|| panic!("Field '{}' missing after round-trip", field));
        assert_eq!(
            values.len(),
            2,
            "Dual-extruder array '{}' should have 2 elements, got {}",
            field,
            values.len()
        );
    }

    // Verify specific values
    let temps = reparsed.nozzle_temperature().expect("nozzle_temperature missing");
    assert_eq!(temps, vec!["220", "220"]);

    // Verify percentages are preserved as strings
    let fan_max = reparsed
        .get_string_array("fan_max_speed")
        .expect("fan_max_speed missing");
    assert_eq!(fan_max, vec!["80%", "80%"]);
}

#[test]
fn test_metadata_round_trip() {
    let path = fixture_path("sample_profile.info");
    let raw_input = std::fs::read_to_string(&path).expect("Failed to read info fixture");

    let meta = ProfileMetadata::from_info_string(&raw_input).expect("Failed to parse metadata");
    let output = meta.to_info_string();

    assert_eq!(
        raw_input, output,
        "Metadata round-trip produced different output.\nInput:  {:?}\nOutput: {:?}",
        raw_input, output
    );

    // Verify parsed values
    assert_eq!(meta.sync_info, "");
    assert_eq!(meta.user_id, "1881310893");
    assert_eq!(meta.setting_id, "PFUS50d8c9d5139548");
    assert_eq!(meta.base_id, "");
    assert_eq!(meta.updated_time, 1770267863);
}

#[test]
fn test_atomic_write_creates_file() {
    let path = fixture_path("sample_profile.json");
    let profile = read_profile(&path).expect("Failed to read fixture");

    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let target = tmp_dir.path().join("output_profile.json");

    write_profile_atomic(&profile, &target).expect("Failed to write profile");

    assert!(target.exists(), "Written profile file should exist");

    // Verify it's valid JSON that round-trips correctly
    let written_content = std::fs::read_to_string(&target).expect("Failed to read written file");
    let reparsed =
        FilamentProfile::from_json(&written_content).expect("Written file is not valid JSON");
    assert_eq!(profile.field_count(), reparsed.field_count());
    assert_eq!(profile.name(), reparsed.name());
}

#[test]
fn test_atomic_write_with_metadata() {
    let json_path = fixture_path("sample_profile.json");
    let profile = read_profile(&json_path).expect("Failed to read fixture");

    let info_path = fixture_path("sample_profile.info");
    let info_content = std::fs::read_to_string(&info_path).expect("Failed to read info fixture");
    let metadata =
        ProfileMetadata::from_info_string(&info_content).expect("Failed to parse metadata");

    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let target_json = tmp_dir.path().join("test_profile.json");

    write_profile_with_metadata(&profile, &target_json, &metadata)
        .expect("Failed to write profile with metadata");

    // Both files should exist
    assert!(target_json.exists(), "Profile JSON file should exist");
    let target_info = target_json.with_extension("info");
    assert!(target_info.exists(), "Profile .info file should exist");

    // Verify JSON content
    let written_json = std::fs::read_to_string(&target_json).expect("Failed to read written JSON");
    let reparsed =
        FilamentProfile::from_json(&written_json).expect("Written JSON is not valid");
    assert_eq!(profile.name(), reparsed.name());

    // Verify metadata content
    let written_info =
        std::fs::read_to_string(&target_info).expect("Failed to read written .info");
    let reparsed_meta =
        ProfileMetadata::from_info_string(&written_info).expect("Written .info is not valid");
    assert_eq!(metadata.user_id, reparsed_meta.user_id);
    assert_eq!(metadata.setting_id, reparsed_meta.setting_id);
    assert_eq!(metadata.updated_time, reparsed_meta.updated_time);
}
