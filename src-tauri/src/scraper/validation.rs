use super::types::{FilamentSpecs, MaterialType, ValidationWarning};

/// Physical constraints for a filament material type.
/// Values outside these ranges indicate likely extraction errors or LLM hallucination.
pub struct MaterialConstraints {
    nozzle_temp_min: u16,
    nozzle_temp_max: u16,
    bed_temp_min: u16,
    bed_temp_max: u16,
}

/// Return the physical constraint ranges for a given material type.
/// These ranges represent the outer bounds of what is physically reasonable
/// for each material. Values outside these indicate extraction errors.
pub fn constraints_for_material(material: &MaterialType) -> MaterialConstraints {
    match material {
        MaterialType::PLA => MaterialConstraints {
            nozzle_temp_min: 180,
            nozzle_temp_max: 235,
            bed_temp_min: 0,
            bed_temp_max: 70,
        },
        MaterialType::PETG => MaterialConstraints {
            nozzle_temp_min: 210,
            nozzle_temp_max: 260,
            bed_temp_min: 40,
            bed_temp_max: 100,
        },
        MaterialType::ABS => MaterialConstraints {
            nozzle_temp_min: 210,
            nozzle_temp_max: 270,
            bed_temp_min: 70,
            bed_temp_max: 120,
        },
        MaterialType::ASA => MaterialConstraints {
            nozzle_temp_min: 220,
            nozzle_temp_max: 270,
            bed_temp_min: 80,
            bed_temp_max: 120,
        },
        MaterialType::TPU => MaterialConstraints {
            nozzle_temp_min: 200,
            nozzle_temp_max: 250,
            bed_temp_min: 20,
            bed_temp_max: 70,
        },
        MaterialType::Nylon => MaterialConstraints {
            nozzle_temp_min: 230,
            nozzle_temp_max: 300,
            bed_temp_min: 50,
            bed_temp_max: 100,
        },
        MaterialType::PC => MaterialConstraints {
            nozzle_temp_min: 250,
            nozzle_temp_max: 320,
            bed_temp_min: 90,
            bed_temp_max: 150,
        },
        MaterialType::PVA => MaterialConstraints {
            nozzle_temp_min: 170,
            nozzle_temp_max: 220,
            bed_temp_min: 30,
            bed_temp_max: 65,
        },
        MaterialType::HIPS => MaterialConstraints {
            nozzle_temp_min: 210,
            nozzle_temp_max: 260,
            bed_temp_min: 80,
            bed_temp_max: 115,
        },
        MaterialType::Other(_) => MaterialConstraints {
            // Permissive fallback for unknown materials
            nozzle_temp_min: 150,
            nozzle_temp_max: 400,
            bed_temp_min: 0,
            bed_temp_max: 120,
        },
    }
}

/// Validate extracted filament specs against physical constraints for the material type.
/// Returns a list of warnings for values that fall outside expected ranges.
/// Warnings indicate possible LLM hallucination, not necessarily hard errors.
pub fn validate_specs(specs: &FilamentSpecs) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();
    let material = MaterialType::from_str(&specs.material);
    let constraints = constraints_for_material(&material);

    // Validate nozzle temperature min
    if let Some(min) = specs.nozzle_temp_min {
        if min < constraints.nozzle_temp_min || min > constraints.nozzle_temp_max {
            warnings.push(ValidationWarning {
                field: "nozzle_temp_min".to_string(),
                message: format!(
                    "Nozzle temp min {}C out of range for {} ({}-{}C)",
                    min, specs.material, constraints.nozzle_temp_min, constraints.nozzle_temp_max
                ),
                value: min.to_string(),
            });
        }
    }

    // Validate nozzle temperature max
    if let Some(max) = specs.nozzle_temp_max {
        if max < constraints.nozzle_temp_min || max > constraints.nozzle_temp_max {
            warnings.push(ValidationWarning {
                field: "nozzle_temp_max".to_string(),
                message: format!(
                    "Nozzle temp max {}C out of range for {} ({}-{}C)",
                    max, specs.material, constraints.nozzle_temp_min, constraints.nozzle_temp_max
                ),
                value: max.to_string(),
            });
        }
    }

    // Validate bed temperature min
    if let Some(min) = specs.bed_temp_min {
        if min < constraints.bed_temp_min || min > constraints.bed_temp_max {
            warnings.push(ValidationWarning {
                field: "bed_temp_min".to_string(),
                message: format!(
                    "Bed temp min {}C out of range for {} ({}-{}C)",
                    min, specs.material, constraints.bed_temp_min, constraints.bed_temp_max
                ),
                value: min.to_string(),
            });
        }
    }

    // Validate bed temperature max
    if let Some(max) = specs.bed_temp_max {
        if max < constraints.bed_temp_min || max > constraints.bed_temp_max {
            warnings.push(ValidationWarning {
                field: "bed_temp_max".to_string(),
                message: format!(
                    "Bed temp max {}C out of range for {} ({}-{}C)",
                    max, specs.material, constraints.bed_temp_min, constraints.bed_temp_max
                ),
                value: max.to_string(),
            });
        }
    }

    // Validate retraction distance (0-15mm is reasonable for any material)
    if let Some(retraction) = specs.retraction_distance_mm {
        if retraction < 0.0 || retraction > 15.0 {
            warnings.push(ValidationWarning {
                field: "retraction_distance_mm".to_string(),
                message: format!(
                    "Retraction distance {}mm out of range (0-15mm)",
                    retraction
                ),
                value: retraction.to_string(),
            });
        }
    }

    // Validate retraction speed (0-100 mm/s is reasonable)
    if let Some(speed) = specs.retraction_speed_mm_s {
        if speed > 100 {
            warnings.push(ValidationWarning {
                field: "retraction_speed_mm_s".to_string(),
                message: format!(
                    "Retraction speed {}mm/s out of range (0-100mm/s)",
                    speed
                ),
                value: speed.to_string(),
            });
        }
    }

    // Validate fan speed (0-100%)
    if let Some(fan) = specs.fan_speed_percent {
        if fan > 100 {
            warnings.push(ValidationWarning {
                field: "fan_speed_percent".to_string(),
                message: format!("Fan speed {}% out of range (0-100%)", fan),
                value: fan.to_string(),
            });
        }
    }

    // Validate diameter (1.0-3.5mm covers 1.75mm and 2.85mm standard diameters)
    if let Some(diameter) = specs.diameter_mm {
        if diameter < 1.0 || diameter > 3.5 {
            warnings.push(ValidationWarning {
                field: "diameter_mm".to_string(),
                message: format!(
                    "Diameter {}mm out of range (1.0-3.5mm)",
                    diameter
                ),
                value: diameter.to_string(),
            });
        }
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pla_specs() -> FilamentSpecs {
        FilamentSpecs {
            name: "Test PLA".to_string(),
            brand: "TestBrand".to_string(),
            material: "PLA".to_string(),
            nozzle_temp_min: Some(190),
            nozzle_temp_max: Some(210),
            bed_temp_min: Some(25),
            bed_temp_max: Some(60),
            max_speed_mm_s: Some(200),
            fan_speed_percent: Some(100),
            retraction_distance_mm: Some(0.8),
            retraction_speed_mm_s: Some(30),
            density_g_cm3: Some(1.24),
            diameter_mm: Some(1.75),
            source_url: "https://example.com".to_string(),
            extraction_confidence: 0.9,
        }
    }

    #[test]
    fn test_pla_valid_specs_no_warnings() {
        let specs = make_pla_specs();
        let warnings = validate_specs(&specs);
        assert!(warnings.is_empty(), "Expected no warnings, got: {:?}", warnings);
    }

    #[test]
    fn test_pla_nozzle_temp_too_high() {
        let mut specs = make_pla_specs();
        specs.nozzle_temp_max = Some(350);
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "nozzle_temp_max");
        assert!(warnings[0].message.contains("350"));
        assert!(warnings[0].message.contains("PLA"));
    }

    #[test]
    fn test_pla_nozzle_temp_too_low() {
        let mut specs = make_pla_specs();
        specs.nozzle_temp_min = Some(100);
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "nozzle_temp_min");
    }

    #[test]
    fn test_bed_temp_out_of_range() {
        let mut specs = make_pla_specs();
        specs.bed_temp_max = Some(120); // PLA bed max is 70
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "bed_temp_max");
    }

    #[test]
    fn test_retraction_distance_out_of_range() {
        let mut specs = make_pla_specs();
        specs.retraction_distance_mm = Some(20.0);
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "retraction_distance_mm");
    }

    #[test]
    fn test_retraction_speed_out_of_range() {
        let mut specs = make_pla_specs();
        specs.retraction_speed_mm_s = Some(150);
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "retraction_speed_mm_s");
    }

    #[test]
    fn test_fan_speed_out_of_range() {
        let mut specs = make_pla_specs();
        // u8 max is 255, so we can test above 100
        specs.fan_speed_percent = Some(150);
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "fan_speed_percent");
    }

    #[test]
    fn test_diameter_out_of_range() {
        let mut specs = make_pla_specs();
        specs.diameter_mm = Some(5.0);
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].field, "diameter_mm");
    }

    #[test]
    fn test_multiple_warnings() {
        let mut specs = make_pla_specs();
        specs.nozzle_temp_max = Some(350);
        specs.bed_temp_max = Some(120);
        specs.retraction_distance_mm = Some(20.0);
        let warnings = validate_specs(&specs);
        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn test_null_fields_no_warnings() {
        let specs = FilamentSpecs {
            name: "Test PLA".to_string(),
            brand: "TestBrand".to_string(),
            material: "PLA".to_string(),
            nozzle_temp_min: None,
            nozzle_temp_max: None,
            bed_temp_min: None,
            bed_temp_max: None,
            max_speed_mm_s: None,
            fan_speed_percent: None,
            retraction_distance_mm: None,
            retraction_speed_mm_s: None,
            density_g_cm3: None,
            diameter_mm: None,
            source_url: "https://example.com".to_string(),
            extraction_confidence: 0.0,
        };
        let warnings = validate_specs(&specs);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_other_material_permissive_ranges() {
        let specs = FilamentSpecs {
            name: "Test Wood Fill".to_string(),
            brand: "TestBrand".to_string(),
            material: "Wood Fill".to_string(),
            nozzle_temp_min: Some(170),
            nozzle_temp_max: Some(380),
            bed_temp_min: Some(0),
            bed_temp_max: Some(100),
            max_speed_mm_s: Some(100),
            fan_speed_percent: Some(100),
            retraction_distance_mm: Some(5.0),
            retraction_speed_mm_s: Some(30),
            density_g_cm3: Some(1.1),
            diameter_mm: Some(1.75),
            source_url: "https://example.com".to_string(),
            extraction_confidence: 0.5,
        };
        let warnings = validate_specs(&specs);
        assert!(warnings.is_empty(), "Other material should use permissive ranges, got: {:?}", warnings);
    }

    #[test]
    fn test_petg_constraints() {
        let constraints = constraints_for_material(&MaterialType::PETG);
        assert_eq!(constraints.nozzle_temp_min, 210);
        assert_eq!(constraints.nozzle_temp_max, 260);
        assert_eq!(constraints.bed_temp_min, 40);
        assert_eq!(constraints.bed_temp_max, 100);
    }

    #[test]
    fn test_abs_constraints() {
        let constraints = constraints_for_material(&MaterialType::ABS);
        assert_eq!(constraints.nozzle_temp_min, 210);
        assert_eq!(constraints.nozzle_temp_max, 270);
        assert_eq!(constraints.bed_temp_min, 70);
        assert_eq!(constraints.bed_temp_max, 120);
    }

    #[test]
    fn test_tpu_constraints() {
        let constraints = constraints_for_material(&MaterialType::TPU);
        assert_eq!(constraints.nozzle_temp_min, 200);
        assert_eq!(constraints.nozzle_temp_max, 250);
    }

    #[test]
    fn test_nylon_constraints() {
        let constraints = constraints_for_material(&MaterialType::Nylon);
        assert_eq!(constraints.nozzle_temp_min, 230);
        assert_eq!(constraints.nozzle_temp_max, 300);
    }

    #[test]
    fn test_pc_constraints() {
        let constraints = constraints_for_material(&MaterialType::PC);
        assert_eq!(constraints.nozzle_temp_min, 250);
        assert_eq!(constraints.nozzle_temp_max, 320);
    }
}
