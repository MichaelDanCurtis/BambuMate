use serde_json;

/// Return the JSON schema for FilamentSpecs extraction.
/// This schema is used with LLM structured output APIs to guarantee
/// valid JSON conforming to our FilamentSpecs struct.
///
/// All optional spec fields use `["integer", "null"]` or `["number", "null"]`
/// types so the LLM can return null for missing values.
pub fn filament_specs_json_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Full filament product name"
            },
            "brand": {
                "type": "string",
                "description": "Manufacturer/brand name"
            },
            "material": {
                "type": "string",
                "description": "Material type: PLA, PETG, ABS, TPU, Nylon, PC, ASA, PVA, HIPS, or other"
            },
            "nozzle_temp_min": {
                "type": ["integer", "null"],
                "description": "Minimum nozzle temperature in Celsius. null if not found in source."
            },
            "nozzle_temp_max": {
                "type": ["integer", "null"],
                "description": "Maximum nozzle temperature in Celsius. null if not found in source."
            },
            "bed_temp_min": {
                "type": ["integer", "null"],
                "description": "Minimum bed temperature in Celsius. null if not found in source."
            },
            "bed_temp_max": {
                "type": ["integer", "null"],
                "description": "Maximum bed temperature in Celsius. null if not found in source."
            },
            "max_speed_mm_s": {
                "type": ["integer", "null"],
                "description": "Maximum recommended print speed in mm/s. null if not found."
            },
            "fan_speed_percent": {
                "type": ["integer", "null"],
                "description": "Recommended cooling fan speed 0-100. null if not found."
            },
            "retraction_distance_mm": {
                "type": ["number", "null"],
                "description": "Retraction distance in mm. null if not found."
            },
            "retraction_speed_mm_s": {
                "type": ["integer", "null"],
                "description": "Retraction speed in mm/s. null if not found."
            },
            "density_g_cm3": {
                "type": ["number", "null"],
                "description": "Material density in g/cm3. null if not found."
            },
            "confidence": {
                "type": "number",
                "description": "Your confidence that the extracted data is correct, 0.0-1.0. Use 0.0 if no data was found in source."
            }
        },
        "required": [
            "name", "brand", "material",
            "nozzle_temp_min", "nozzle_temp_max",
            "bed_temp_min", "bed_temp_max",
            "max_speed_mm_s", "fan_speed_percent",
            "retraction_distance_mm", "retraction_speed_mm_s",
            "density_g_cm3", "confidence"
        ],
        "additionalProperties": false
    })
}

/// Build the extraction prompt for the LLM.
/// Contains anti-hallucination rules and confidence scoring guidelines.
/// The prompt instructs the LLM to return null for any value not explicitly
/// stated in the source text.
pub fn build_extraction_prompt(filament_name: &str, page_text: &str) -> String {
    format!(
        r#"Extract 3D printing specifications for the filament "{filament_name}" from the following text.

RULES:
- Only extract values explicitly stated in the text below.
- If a value is NOT present in the text, return null for that field.
- Do NOT guess, infer, or use general knowledge about filament types.
- Temperature values must be in Celsius.
- Speed values must be in mm/s.
- Set confidence to 0.0 if no printing parameters were found in the text.
- Set confidence to 0.3-0.6 if only some parameters were found.
- Set confidence to 0.7-1.0 if most parameters were found.

SOURCE TEXT:
{page_text}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_schema_has_all_required_fields() {
        let schema = filament_specs_json_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();

        assert!(required_strs.contains(&"name"));
        assert!(required_strs.contains(&"brand"));
        assert!(required_strs.contains(&"material"));
        assert!(required_strs.contains(&"nozzle_temp_min"));
        assert!(required_strs.contains(&"nozzle_temp_max"));
        assert!(required_strs.contains(&"bed_temp_min"));
        assert!(required_strs.contains(&"bed_temp_max"));
        assert!(required_strs.contains(&"max_speed_mm_s"));
        assert!(required_strs.contains(&"fan_speed_percent"));
        assert!(required_strs.contains(&"retraction_distance_mm"));
        assert!(required_strs.contains(&"retraction_speed_mm_s"));
        assert!(required_strs.contains(&"density_g_cm3"));
        assert!(required_strs.contains(&"confidence"));
    }

    #[test]
    fn test_json_schema_nullable_fields() {
        let schema = filament_specs_json_schema();
        let properties = schema["properties"].as_object().unwrap();

        // Temperature fields should be nullable integers
        let nozzle_type = properties["nozzle_temp_min"]["type"].as_array().unwrap();
        assert!(nozzle_type.contains(&serde_json::json!("integer")));
        assert!(nozzle_type.contains(&serde_json::json!("null")));

        // Retraction distance should be nullable number (float)
        let retract_type = properties["retraction_distance_mm"]["type"].as_array().unwrap();
        assert!(retract_type.contains(&serde_json::json!("number")));
        assert!(retract_type.contains(&serde_json::json!("null")));

        // Confidence should be a non-nullable number
        let confidence_type = &properties["confidence"]["type"];
        assert_eq!(confidence_type, "number");
    }

    #[test]
    fn test_json_schema_string_fields() {
        let schema = filament_specs_json_schema();
        let properties = schema["properties"].as_object().unwrap();

        assert_eq!(properties["name"]["type"], "string");
        assert_eq!(properties["brand"]["type"], "string");
        assert_eq!(properties["material"]["type"], "string");
    }

    #[test]
    fn test_json_schema_no_additional_properties() {
        let schema = filament_specs_json_schema();
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn test_json_schema_is_valid_json() {
        let schema = filament_specs_json_schema();
        let json_str = serde_json::to_string(&schema).unwrap();
        let reparsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(schema, reparsed);
    }

    #[test]
    fn test_extraction_prompt_contains_filament_name() {
        let prompt = build_extraction_prompt("Polymaker PLA Pro", "some text content");
        assert!(
            prompt.contains("Polymaker PLA Pro"),
            "Prompt should contain filament name"
        );
    }

    #[test]
    fn test_extraction_prompt_contains_source_text() {
        let source = "Nozzle Temperature: 190-220C, Bed Temperature: 50-60C";
        let prompt = build_extraction_prompt("Test PLA", source);
        assert!(
            prompt.contains(source),
            "Prompt should contain source text"
        );
    }

    #[test]
    fn test_extraction_prompt_anti_hallucination_rules() {
        let prompt = build_extraction_prompt("Test PLA", "some text");
        assert!(
            prompt.contains("Do NOT guess"),
            "Prompt should contain anti-hallucination rule"
        );
        assert!(
            prompt.contains("return null"),
            "Prompt should instruct to return null for missing values"
        );
        assert!(
            prompt.contains("NOT present in the text"),
            "Prompt should reference missing data handling"
        );
    }

    #[test]
    fn test_extraction_prompt_confidence_guidelines() {
        let prompt = build_extraction_prompt("Test PLA", "some text");
        assert!(
            prompt.contains("confidence"),
            "Prompt should mention confidence scoring"
        );
        assert!(
            prompt.contains("0.0"),
            "Prompt should mention 0.0 confidence for no data"
        );
    }
}
