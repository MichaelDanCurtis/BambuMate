use serde::{Deserialize, Serialize};

/// Structured filament specifications extracted from manufacturer data.
/// All temperature fields are in Celsius. Speed fields in mm/s.
/// Optional fields represent data that may not be available for all filaments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilamentSpecs {
    /// Filament name as provided by manufacturer
    pub name: String,
    /// Brand/manufacturer name
    pub brand: String,
    /// Material type (PLA, PETG, ABS, TPU, etc.)
    pub material: String,

    // Temperature ranges
    pub nozzle_temp_min: Option<u16>,
    pub nozzle_temp_max: Option<u16>,
    pub bed_temp_min: Option<u16>,
    pub bed_temp_max: Option<u16>,

    // Speed
    pub max_speed_mm_s: Option<u16>,

    // Cooling
    pub fan_speed_percent: Option<u8>,

    // Retraction
    pub retraction_distance_mm: Option<f32>,
    pub retraction_speed_mm_s: Option<u16>,

    // Physical properties
    pub density_g_cm3: Option<f32>,
    pub diameter_mm: Option<f32>,

    // Metadata
    pub source_url: String,
    pub extraction_confidence: f32,
}

/// Recognized material types for filament classification.
/// Used for physical constraint validation lookups.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MaterialType {
    PLA,
    PETG,
    ABS,
    ASA,
    TPU,
    Nylon,
    PC,
    PVA,
    HIPS,
    Other(String),
}

impl MaterialType {
    /// Parse a material string into a MaterialType using case-insensitive
    /// substring matching. Priority order prevents false positives
    /// (e.g., "PLA" is checked before "PA" so "PLA" doesn't match as Nylon).
    pub fn from_str(input: &str) -> MaterialType {
        let upper = input.to_uppercase();

        // Order matters: check more specific substrings before generic ones.
        // - PLA before PA (to avoid "PLA" matching as Nylon/PA)
        // - PETG before PC (PETG doesn't contain PC, but be explicit)
        // - PC before ABS (to catch "PC-ABS" as PC, not ABS)
        // - HIPS before PC (HIPS doesn't contain PC, but keeps specifics first)
        if upper.contains("PLA") {
            MaterialType::PLA
        } else if upper.contains("PETG") {
            MaterialType::PETG
        } else if upper.contains("ASA") {
            MaterialType::ASA
        } else if upper.contains("HIPS") {
            MaterialType::HIPS
        } else if upper.contains("PVA") {
            MaterialType::PVA
        } else if upper.contains("PC") || upper.contains("POLYCARBONATE") {
            MaterialType::PC
        } else if upper.contains("ABS") {
            MaterialType::ABS
        } else if upper.contains("TPU") || upper.contains("TPE") {
            MaterialType::TPU
        } else if upper.contains("PA") || upper.contains("NYLON") {
            MaterialType::Nylon
        } else {
            MaterialType::Other(input.to_string())
        }
    }
}

/// A warning produced by physical constraint validation.
/// Warnings indicate that an extracted value falls outside the expected
/// range for the material type, suggesting possible LLM hallucination.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationWarning {
    /// The field name that triggered the warning
    pub field: String,
    /// Human-readable warning message
    pub message: String,
    /// The value that was out of range (as a string for display)
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filament_specs_serde_roundtrip() {
        let specs = FilamentSpecs {
            name: "PolyLite PLA Pro".to_string(),
            brand: "Polymaker".to_string(),
            material: "PLA".to_string(),
            nozzle_temp_min: Some(190),
            nozzle_temp_max: Some(220),
            bed_temp_min: Some(25),
            bed_temp_max: Some(60),
            max_speed_mm_s: Some(200),
            fan_speed_percent: Some(100),
            retraction_distance_mm: Some(0.8),
            retraction_speed_mm_s: Some(30),
            density_g_cm3: Some(1.24),
            diameter_mm: Some(1.75),
            source_url: "https://polymaker.com/products/polylite-pla-pro".to_string(),
            extraction_confidence: 0.85,
        };

        let json = serde_json::to_string(&specs).unwrap();
        let deserialized: FilamentSpecs = serde_json::from_str(&json).unwrap();
        assert_eq!(specs, deserialized);
    }

    #[test]
    fn test_filament_specs_nullable_fields() {
        let json = r#"{
            "name": "Test PLA",
            "brand": "TestBrand",
            "material": "PLA",
            "nozzle_temp_min": null,
            "nozzle_temp_max": 210,
            "bed_temp_min": null,
            "bed_temp_max": null,
            "max_speed_mm_s": null,
            "fan_speed_percent": null,
            "retraction_distance_mm": null,
            "retraction_speed_mm_s": null,
            "density_g_cm3": null,
            "diameter_mm": 1.75,
            "source_url": "https://example.com",
            "extraction_confidence": 0.3
        }"#;

        let specs: FilamentSpecs = serde_json::from_str(json).unwrap();
        assert_eq!(specs.name, "Test PLA");
        assert_eq!(specs.nozzle_temp_min, None);
        assert_eq!(specs.nozzle_temp_max, Some(210));
        assert_eq!(specs.diameter_mm, Some(1.75));
    }

    #[test]
    fn test_material_type_pla_variants() {
        assert_eq!(MaterialType::from_str("PLA"), MaterialType::PLA);
        assert_eq!(MaterialType::from_str("PLA+"), MaterialType::PLA);
        assert_eq!(MaterialType::from_str("pla pro"), MaterialType::PLA);
        assert_eq!(MaterialType::from_str("PolyLite PLA"), MaterialType::PLA);
        assert_eq!(MaterialType::from_str("PLA-CF"), MaterialType::PLA);
    }

    #[test]
    fn test_material_type_petg() {
        assert_eq!(MaterialType::from_str("PETG"), MaterialType::PETG);
        assert_eq!(MaterialType::from_str("PETG-CF"), MaterialType::PETG);
        assert_eq!(MaterialType::from_str("petg"), MaterialType::PETG);
    }

    #[test]
    fn test_material_type_abs_asa() {
        assert_eq!(MaterialType::from_str("ABS"), MaterialType::ABS);
        assert_eq!(MaterialType::from_str("ABS+"), MaterialType::ABS);
        assert_eq!(MaterialType::from_str("ASA"), MaterialType::ASA);
    }

    #[test]
    fn test_material_type_tpu_tpe() {
        assert_eq!(MaterialType::from_str("TPU"), MaterialType::TPU);
        assert_eq!(MaterialType::from_str("TPU 95A"), MaterialType::TPU);
        assert_eq!(MaterialType::from_str("TPE"), MaterialType::TPU);
    }

    #[test]
    fn test_material_type_nylon() {
        assert_eq!(MaterialType::from_str("PA6"), MaterialType::Nylon);
        assert_eq!(MaterialType::from_str("PA12"), MaterialType::Nylon);
        assert_eq!(MaterialType::from_str("PA6-CF"), MaterialType::Nylon);
        assert_eq!(MaterialType::from_str("Nylon"), MaterialType::Nylon);
    }

    #[test]
    fn test_material_type_pc() {
        assert_eq!(MaterialType::from_str("PC"), MaterialType::PC);
        assert_eq!(MaterialType::from_str("PC-ABS"), MaterialType::PC);
        assert_eq!(MaterialType::from_str("Polycarbonate"), MaterialType::PC);
    }

    #[test]
    fn test_material_type_pva_hips() {
        assert_eq!(MaterialType::from_str("PVA"), MaterialType::PVA);
        assert_eq!(MaterialType::from_str("HIPS"), MaterialType::HIPS);
    }

    #[test]
    fn test_material_type_other() {
        assert_eq!(
            MaterialType::from_str("Carbon Fiber Filled"),
            MaterialType::Other("Carbon Fiber Filled".to_string())
        );
        assert_eq!(
            MaterialType::from_str("Wood Fill"),
            MaterialType::Other("Wood Fill".to_string())
        );
    }

    #[test]
    fn test_pla_not_matched_as_pa() {
        // PLA must be checked before PA to avoid false positive
        assert_eq!(MaterialType::from_str("PLA"), MaterialType::PLA);
        // PA should match as Nylon only when not part of PLA
        assert_eq!(MaterialType::from_str("PA"), MaterialType::Nylon);
    }
}
