//! Prompts and schemas for defect analysis vision API calls.

use std::collections::HashMap;

/// JSON schema for structured defect report output.
/// Matches the DetectedDefect type from mapper::types.
pub fn defect_report_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "defects": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "defect_type": {
                            "type": "string",
                            "enum": ["stringing", "warping", "layer_adhesion",
                                    "elephants_foot", "under_extrusion",
                                    "over_extrusion", "z_banding"]
                        },
                        "severity": {
                            "type": "number",
                            "description": "0.0-1.0 scale: 0.3=minor, 0.5=noticeable, 0.7=significant, 0.9=severe"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "0.0-1.0 confidence in detection accuracy"
                        }
                    },
                    "required": ["defect_type", "severity", "confidence"],
                    "additionalProperties": false
                }
            },
            "overall_quality": {
                "type": "string",
                "enum": ["excellent", "good", "acceptable", "poor", "failed"]
            },
            "notes": {
                "type": ["string", "null"],
                "description": "Brief observation about the print (optional)"
            }
        },
        "required": ["defects", "overall_quality"],
        "additionalProperties": false
    })
}

/// Build the defect analysis prompt with current profile context.
///
/// # Arguments
/// * `current_settings` - Current profile parameter values
/// * `material_type` - Material type string (e.g., "PLA", "PETG")
pub fn build_defect_analysis_prompt(
    current_settings: &HashMap<String, f32>,
    material_type: &str,
) -> String {
    let nozzle_temp = current_settings
        .get("nozzle_temperature")
        .unwrap_or(&200.0);
    let bed_temp = current_settings
        .get("cool_plate_temp")
        .or_else(|| current_settings.get("hot_plate_temp"))
        .unwrap_or(&60.0);
    let retraction = current_settings
        .get("filament_retraction_length")
        .unwrap_or(&0.8);
    let flow = current_settings
        .get("filament_flow_ratio")
        .unwrap_or(&1.0);

    format!(
        r#"Analyze this 3D print photo for defects.

Current print settings:
- Material: {material}
- Nozzle temperature: {nozzle_temp}C
- Bed temperature: {bed_temp}C
- Retraction: {retraction}mm
- Flow ratio: {flow}

Identify any defects from this list:
- stringing: Fine threads/wisps between parts or during travel moves
- warping: Corners or edges lifting from the print bed
- layer_adhesion: Weak bonds between layers, visible gaps, delamination
- elephants_foot: First layer(s) bulging outward, wider than intended
- under_extrusion: Gaps in walls, missing material, thin/weak layers
- over_extrusion: Blobs, rough surfaces, material oozing, dimensional inaccuracy
- z_banding: Horizontal lines/ridges at regular intervals on vertical surfaces

For each defect found, rate:
- severity: 0.3=minor/cosmetic only, 0.5=noticeable but functional, 0.7=significant quality issue, 0.9=severe/print failure
- confidence: How certain you are this defect is actually present (not lighting/angle artifact)

Rate overall print quality:
- excellent: No visible defects, professional quality
- good: Minor cosmetic issues only
- acceptable: Some defects but functional
- poor: Significant quality issues
- failed: Print unusable

If no defects are visible, return an empty defects array and rate as excellent/good.
If image is unclear or not a 3D print, note this in the notes field."#,
        material = material_type,
        nozzle_temp = nozzle_temp,
        bed_temp = bed_temp,
        retraction = retraction,
        flow = flow,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defect_report_schema_structure() {
        let schema = defect_report_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["defects"].is_object());
        assert!(schema["properties"]["overall_quality"].is_object());
    }

    #[test]
    fn test_defect_report_schema_defect_types() {
        let schema = defect_report_schema();
        let defect_types = &schema["properties"]["defects"]["items"]["properties"]["defect_type"]
            ["enum"];
        assert!(defect_types.is_array());

        let types: Vec<&str> = defect_types
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        assert!(types.contains(&"stringing"));
        assert!(types.contains(&"warping"));
        assert!(types.contains(&"layer_adhesion"));
        assert!(types.contains(&"elephants_foot"));
        assert!(types.contains(&"under_extrusion"));
        assert!(types.contains(&"over_extrusion"));
        assert!(types.contains(&"z_banding"));
    }

    #[test]
    fn test_defect_report_schema_quality_levels() {
        let schema = defect_report_schema();
        let quality_levels = &schema["properties"]["overall_quality"]["enum"];
        assert!(quality_levels.is_array());

        let levels: Vec<&str> = quality_levels
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        assert!(levels.contains(&"excellent"));
        assert!(levels.contains(&"good"));
        assert!(levels.contains(&"acceptable"));
        assert!(levels.contains(&"poor"));
        assert!(levels.contains(&"failed"));
    }

    #[test]
    fn test_build_prompt_includes_settings() {
        let mut settings = HashMap::new();
        settings.insert("nozzle_temperature".to_string(), 215.0);
        settings.insert("cool_plate_temp".to_string(), 55.0);

        let prompt = build_defect_analysis_prompt(&settings, "PLA");
        assert!(prompt.contains("215"));
        assert!(prompt.contains("55"));
        assert!(prompt.contains("PLA"));
    }

    #[test]
    fn test_build_prompt_uses_defaults() {
        let settings = HashMap::new();
        let prompt = build_defect_analysis_prompt(&settings, "PETG");
        assert!(prompt.contains("200")); // default nozzle temp
        assert!(prompt.contains("60")); // default bed temp
    }

    #[test]
    fn test_build_prompt_uses_hot_plate_temp_fallback() {
        let mut settings = HashMap::new();
        settings.insert("hot_plate_temp".to_string(), 80.0);

        let prompt = build_defect_analysis_prompt(&settings, "ABS");
        assert!(prompt.contains("80"));
    }

    #[test]
    fn test_build_prompt_includes_all_defect_types() {
        let settings = HashMap::new();
        let prompt = build_defect_analysis_prompt(&settings, "PLA");

        assert!(prompt.contains("stringing"));
        assert!(prompt.contains("warping"));
        assert!(prompt.contains("layer_adhesion"));
        assert!(prompt.contains("elephants_foot"));
        assert!(prompt.contains("under_extrusion"));
        assert!(prompt.contains("over_extrusion"));
        assert!(prompt.contains("z_banding"));
    }

    #[test]
    fn test_build_prompt_includes_severity_guidance() {
        let settings = HashMap::new();
        let prompt = build_defect_analysis_prompt(&settings, "PLA");

        assert!(prompt.contains("0.3=minor"));
        assert!(prompt.contains("0.5=noticeable"));
        assert!(prompt.contains("0.7=significant"));
        assert!(prompt.contains("0.9=severe"));
    }
}
