use serde::{Deserialize, Serialize};

/// Structured filament specifications extracted from manufacturer data.
/// All temperature fields are in Celsius. Speed fields in mm/s.
/// Optional fields represent data that may not be available for all filaments.
///
/// This struct captures the key parameters needed to generate a working
/// Bambu Studio filament profile. A real profile has ~120 fields, but most
/// can be derived from material-type defaults. These ~35 fields cover the
/// parameters that vary meaningfully between filaments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilamentSpecs {
    /// Filament name as provided by manufacturer
    pub name: String,
    /// Brand/manufacturer name
    pub brand: String,
    /// Material type (PLA, PETG, ABS, TPU, etc.)
    pub material: String,

    // === Temperature ranges (for nozzle slider bounds) ===
    pub nozzle_temp_min: Option<u16>,
    pub nozzle_temp_max: Option<u16>,
    pub bed_temp_min: Option<u16>,
    pub bed_temp_max: Option<u16>,

    // === Actual printing temperatures ===
    /// Default nozzle temperature for printing (maps to nozzle_temperature)
    pub nozzle_temperature: Option<u16>,
    /// Nozzle temp for the first layer (often 5-10C higher)
    pub nozzle_temperature_initial_layer: Option<u16>,

    // === Per-plate bed temperatures ===
    /// Heated/hot plate bed temperature
    pub hot_plate_temp: Option<u16>,
    pub hot_plate_temp_initial_layer: Option<u16>,
    /// Cool plate (PEI smooth) bed temperature
    pub cool_plate_temp: Option<u16>,
    pub cool_plate_temp_initial_layer: Option<u16>,
    /// Engineering plate bed temperature
    pub eng_plate_temp: Option<u16>,
    pub eng_plate_temp_initial_layer: Option<u16>,
    /// Textured plate bed temperature
    pub textured_plate_temp: Option<u16>,
    pub textured_plate_temp_initial_layer: Option<u16>,

    // === Flow & volumetric speed ===
    /// Maximum volumetric speed in mm³/s (THE key speed parameter in Bambu Studio)
    pub max_volumetric_speed: Option<f32>,
    /// Extrusion multiplier / flow ratio (typically 0.95-1.0)
    pub filament_flow_ratio: Option<f32>,
    /// Linear/pressure advance value
    pub pressure_advance: Option<f32>,

    // === Fan/cooling curve ===
    /// Minimum part cooling fan speed (0-100%)
    pub fan_min_speed: Option<u8>,
    /// Maximum part cooling fan speed (0-100%)
    pub fan_max_speed: Option<u8>,
    /// Fan speed for overhangs (0-100%)
    pub overhang_fan_speed: Option<u8>,
    /// Number of initial layers with fan off
    pub close_fan_the_first_x_layers: Option<u8>,
    /// Auxiliary/additional cooling fan speed (0-100%)
    pub additional_cooling_fan_speed: Option<u8>,

    // === Legacy fan field (kept for backward compat) ===
    pub fan_speed_percent: Option<u8>,

    // === Cooling slowdown ===
    /// Minimum layer time before speed reduction (seconds)
    pub slow_down_layer_time: Option<u8>,
    /// Minimum speed when slowing down for cooling (mm/s)
    pub slow_down_min_speed: Option<u16>,

    // === Retraction ===
    pub retraction_distance_mm: Option<f32>,
    pub retraction_speed_mm_s: Option<u16>,
    /// De-retraction speed (mm/s), often different from retraction speed
    pub deretraction_speed_mm_s: Option<u16>,

    // === Overhang/bridge ===
    /// Bridge print speed (mm/s)
    pub bridge_speed: Option<u16>,

    // === Physical properties ===
    pub density_g_cm3: Option<f32>,
    pub diameter_mm: Option<f32>,
    /// Glass transition temperature (°C) — important for drying and AMS limits
    pub temperature_vitrification: Option<u16>,
    /// Filament cost per unit (typically per kg)
    pub filament_cost: Option<f32>,

    // === Speed (legacy) ===
    pub max_speed_mm_s: Option<u16>,

    // === Metadata ===
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
            nozzle_temperature: Some(210),
            nozzle_temperature_initial_layer: Some(215),
            hot_plate_temp: Some(55),
            hot_plate_temp_initial_layer: Some(55),
            cool_plate_temp: Some(50),
            cool_plate_temp_initial_layer: Some(50),
            eng_plate_temp: Some(55),
            eng_plate_temp_initial_layer: Some(55),
            textured_plate_temp: Some(55),
            textured_plate_temp_initial_layer: Some(55),
            max_volumetric_speed: Some(21.0),
            filament_flow_ratio: Some(0.98),
            pressure_advance: Some(0.04),
            fan_min_speed: Some(100),
            fan_max_speed: Some(100),
            overhang_fan_speed: Some(100),
            close_fan_the_first_x_layers: Some(1),
            additional_cooling_fan_speed: Some(80),
            fan_speed_percent: Some(100),
            slow_down_layer_time: Some(8),
            slow_down_min_speed: Some(20),
            max_speed_mm_s: Some(200),
            retraction_distance_mm: Some(0.8),
            retraction_speed_mm_s: Some(30),
            deretraction_speed_mm_s: None,
            bridge_speed: Some(25),
            density_g_cm3: Some(1.24),
            diameter_mm: Some(1.75),
            temperature_vitrification: Some(55),
            filament_cost: Some(24.99),
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
            "nozzle_temperature": null,
            "nozzle_temperature_initial_layer": null,
            "hot_plate_temp": null,
            "hot_plate_temp_initial_layer": null,
            "cool_plate_temp": null,
            "cool_plate_temp_initial_layer": null,
            "eng_plate_temp": null,
            "eng_plate_temp_initial_layer": null,
            "textured_plate_temp": null,
            "textured_plate_temp_initial_layer": null,
            "max_volumetric_speed": null,
            "filament_flow_ratio": null,
            "pressure_advance": null,
            "fan_min_speed": null,
            "fan_max_speed": null,
            "overhang_fan_speed": null,
            "close_fan_the_first_x_layers": null,
            "additional_cooling_fan_speed": null,
            "fan_speed_percent": null,
            "slow_down_layer_time": null,
            "slow_down_min_speed": null,
            "max_speed_mm_s": null,
            "retraction_distance_mm": null,
            "retraction_speed_mm_s": null,
            "deretraction_speed_mm_s": null,
            "bridge_speed": null,
            "density_g_cm3": null,
            "diameter_mm": 1.75,
            "temperature_vitrification": null,
            "filament_cost": null,
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
