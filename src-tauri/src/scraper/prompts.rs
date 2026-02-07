use serde_json;

/// Return a compact text version of the JSON schema for embedding in prompts.
/// Used by providers (Claude, OpenRouter) that can't handle 33+ union-typed
/// parameters in strict JSON schema mode.
pub fn filament_specs_schema_text() -> String {
    let schema = filament_specs_json_schema();
    serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string())
}

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
            "nozzle_temperature": {
                "type": ["integer", "null"],
                "description": "Default nozzle temperature for printing in Celsius. This is the main temp, not the range."
            },
            "nozzle_temperature_initial_layer": {
                "type": ["integer", "null"],
                "description": "Nozzle temperature for the first layer in Celsius. Often 5-10C higher than nozzle_temperature."
            },
            "hot_plate_temp": {
                "type": ["integer", "null"],
                "description": "Heated/hot plate bed temperature in Celsius."
            },
            "hot_plate_temp_initial_layer": {
                "type": ["integer", "null"],
                "description": "Heated/hot plate bed temperature for first layer in Celsius."
            },
            "cool_plate_temp": {
                "type": ["integer", "null"],
                "description": "Cool/smooth PEI plate bed temperature in Celsius."
            },
            "cool_plate_temp_initial_layer": {
                "type": ["integer", "null"],
                "description": "Cool plate bed temperature for first layer in Celsius."
            },
            "eng_plate_temp": {
                "type": ["integer", "null"],
                "description": "Engineering plate bed temperature in Celsius."
            },
            "eng_plate_temp_initial_layer": {
                "type": ["integer", "null"],
                "description": "Engineering plate bed temperature for first layer in Celsius."
            },
            "textured_plate_temp": {
                "type": ["integer", "null"],
                "description": "Textured plate bed temperature in Celsius."
            },
            "textured_plate_temp_initial_layer": {
                "type": ["integer", "null"],
                "description": "Textured plate bed temperature for first layer in Celsius."
            },
            "max_volumetric_speed": {
                "type": ["number", "null"],
                "description": "Maximum volumetric flow rate in mm\u{00b3}/s. The key speed parameter in Bambu Studio."
            },
            "filament_flow_ratio": {
                "type": ["number", "null"],
                "description": "Extrusion multiplier / flow ratio. Typically 0.95-1.0."
            },
            "pressure_advance": {
                "type": ["number", "null"],
                "description": "Linear/pressure advance value. Typically 0.01-0.06."
            },
            "fan_min_speed": {
                "type": ["integer", "null"],
                "description": "Minimum part cooling fan speed 0-100%."
            },
            "fan_max_speed": {
                "type": ["integer", "null"],
                "description": "Maximum part cooling fan speed 0-100%."
            },
            "overhang_fan_speed": {
                "type": ["integer", "null"],
                "description": "Fan speed for overhangs 0-100%."
            },
            "close_fan_the_first_x_layers": {
                "type": ["integer", "null"],
                "description": "Number of initial layers with fan completely off."
            },
            "additional_cooling_fan_speed": {
                "type": ["integer", "null"],
                "description": "Auxiliary/additional cooling fan speed 0-100%."
            },
            "slow_down_layer_time": {
                "type": ["integer", "null"],
                "description": "Minimum layer time in seconds before speed reduction."
            },
            "slow_down_min_speed": {
                "type": ["integer", "null"],
                "description": "Minimum print speed in mm/s when slowing down for cooling."
            },
            "deretraction_speed_mm_s": {
                "type": ["integer", "null"],
                "description": "De-retraction speed in mm/s."
            },
            "bridge_speed": {
                "type": ["integer", "null"],
                "description": "Bridge print speed in mm/s."
            },
            "temperature_vitrification": {
                "type": ["integer", "null"],
                "description": "Glass transition temperature in Celsius."
            },
            "diameter_mm": {
                "type": ["number", "null"],
                "description": "Filament diameter in mm (usually 1.75)."
            },
            "filament_cost": {
                "type": ["number", "null"],
                "description": "Filament cost per kg in USD."
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
            "density_g_cm3",
            "nozzle_temperature", "nozzle_temperature_initial_layer",
            "hot_plate_temp", "hot_plate_temp_initial_layer",
            "cool_plate_temp", "cool_plate_temp_initial_layer",
            "eng_plate_temp", "eng_plate_temp_initial_layer",
            "textured_plate_temp", "textured_plate_temp_initial_layer",
            "max_volumetric_speed", "filament_flow_ratio", "pressure_advance",
            "fan_min_speed", "fan_max_speed", "overhang_fan_speed",
            "close_fan_the_first_x_layers", "additional_cooling_fan_speed",
            "slow_down_layer_time", "slow_down_min_speed",
            "deretraction_speed_mm_s", "bridge_speed",
            "temperature_vitrification", "diameter_mm", "filament_cost",
            "confidence"
        ],
        "additionalProperties": false
    })
}

/// Build the extraction prompt for the LLM.
/// Contains anti-hallucination rules and confidence scoring guidelines.
/// The prompt instructs the LLM to return null for any value not explicitly
/// stated in the source text.
pub fn build_extraction_prompt(filament_name: &str, page_text: &str) -> String {
    let schema = filament_specs_schema_text();
    format!(
        r#"Extract 3D printing specifications for the filament "{filament_name}" from the following text.

RULES:
- Only extract values explicitly stated in the text below.
- If a value is NOT present in the text, return null for that field.
- Do NOT guess, infer, or use general knowledge about filament types.
- Temperature values must be in Celsius.
- Speed values must be in mm/s.
- For Bambu Studio specific fields (nozzle_temperature, plate temps, max_volumetric_speed, filament_flow_ratio, pressure_advance, fan speeds, cooling settings, etc.), extract these if the source provides them. These are distinct from the general temp ranges.
- nozzle_temperature is the single recommended printing temp, NOT the range. nozzle_temp_min/max are the range.
- Plate-specific bed temps (hot_plate_temp, cool_plate_temp, eng_plate_temp, textured_plate_temp) and their initial_layer variants should be extracted if available.
- Set confidence to 0.0 if no printing parameters were found in the text.
- Set confidence to 0.3-0.6 if only some parameters were found.
- Set confidence to 0.7-1.0 if most parameters were found.

Return a JSON object matching this schema:
{schema}

SOURCE TEXT:
{page_text}"#
    )
}

/// Build the extraction prompt for raw HTML content.
/// Unlike the text extraction prompt, this preserves HTML structure (tables,
/// meta tags, JSON-LD, spec lists) so the LLM can parse structured data directly.
/// The HTML is truncated to stay within reasonable token limits.
pub fn build_html_extraction_prompt(filament_name: &str, html: &str) -> String {
    // Truncate HTML to ~50K chars to stay within context limits
    let max_len = 50_000;
    let truncated = if html.len() > max_len {
        &html[..max_len]
    } else {
        html
    };

    let schema = filament_specs_schema_text();
    format!(
        r#"Extract 3D printing specifications for the filament "{filament_name}" from the following HTML page.

RULES:
- Parse the HTML structure directly — look for spec tables, product details, meta tags, JSON-LD structured data, and specification lists.
- Only extract values explicitly stated in the page content.
- If a value is NOT present anywhere in the HTML, return null for that field.
- Do NOT guess, infer, or use general knowledge about filament types.
- Temperature values must be in Celsius. If the page shows Fahrenheit, convert to Celsius.
- Speed values must be in mm/s.
- For Bambu Studio specific fields (nozzle_temperature, plate temps, max_volumetric_speed, filament_flow_ratio, pressure_advance, fan speeds, cooling settings, etc.), extract these if the source provides them. These are distinct from the general temp ranges.
- nozzle_temperature is the single recommended printing temp, NOT the range. nozzle_temp_min/max are the range.
- Plate-specific bed temps (hot_plate_temp, cool_plate_temp, eng_plate_temp, textured_plate_temp) and their initial_layer variants should be extracted if available.
- Look for specs in these common locations:
  - <table> elements with headers like "Specifications", "Properties", "Technical Data"
  - <dl>/<dt>/<dd> definition lists
  - <meta> tags and JSON-LD scripts for product data
  - Product description sections
  - Data sheet download links or embedded PDFs
- Set confidence to 0.0 if no printing parameters were found in the HTML.
- Set confidence to 0.3-0.6 if only some parameters were found.
- Set confidence to 0.7-1.0 if most parameters were found.

Return a JSON object matching this schema:
{schema}

HTML CONTENT:
{truncated}"#
    )
}

/// Build a prompt for generating filament specs from AI knowledge.
/// This is the OPPOSITE of extraction - we WANT the AI to use its training knowledge.
/// Used as a fallback when catalog search and web scraping fail.
pub fn build_knowledge_prompt(filament_name: &str) -> String {
    let schema = filament_specs_schema_text();
    format!(
        r#"Provide recommended 3D printing specifications for the filament "{filament_name}".

Use your training knowledge about this filament brand and material type. If you know specific recommended settings for this exact filament from manufacturer documentation or community experience, use those. If you don't know this specific filament, provide reasonable defaults based on the material type (PLA, PETG, ABS, etc.) that you can infer from the name.

RULES:
- Parse the filament name to identify the brand (first word typically) and material type.
- Provide your best recommendation for each parameter based on your knowledge.
- Temperature values must be in Celsius.
- Speed values must be in mm/s.
- Set confidence based on how well you know this specific filament:
  - 0.8-1.0 if you have specific knowledge of this exact filament
  - 0.5-0.7 if you're using general knowledge of the material type
  - 0.3-0.4 if you're mostly guessing based on the name

Common material defaults if unknown:
- PLA: nozzle range 190-220°C, bed 50-60°C, fan 100%
  nozzle_temperature 210, initial_layer 215, hot_plate 55, cool_plate 50, eng_plate 55, textured_plate 55
  max_volumetric_speed 21, flow_ratio 0.98, pressure_advance 0.04
  fan_min 100, fan_max 100, overhang_fan 100, close_fan_first_layers 1, additional_cooling_fan 80
  slow_down_layer_time 8, slow_down_min_speed 20, vitrification 55
- PETG: nozzle range 230-250°C, bed 70-85°C, fan 30-50%
  nozzle_temperature 250, initial_layer 255, hot_plate 70, cool_plate 0 (not recommended), eng_plate 70, textured_plate 70
  max_volumetric_speed 18, flow_ratio 0.97, pressure_advance 0.02
  fan_min 20, fan_max 40, overhang_fan 100, close_fan_first_layers 3, additional_cooling_fan 50
  slow_down_layer_time 10, slow_down_min_speed 20, vitrification 60
- ABS: nozzle range 230-260°C, bed 90-110°C, fan 0-30%
  nozzle_temperature 250, initial_layer 255, hot_plate 100, cool_plate 0, eng_plate 100, textured_plate 100
  max_volumetric_speed 16, flow_ratio 0.98, pressure_advance 0.02
  fan_min 0, fan_max 30, overhang_fan 80, close_fan_first_layers 3, additional_cooling_fan 0
  vitrification 100
- TPU: nozzle range 220-240°C, bed 40-60°C, fan 50-80%
  nozzle_temperature 230, initial_layer 230, hot_plate 50, cool_plate 50
  max_volumetric_speed 8, flow_ratio 1.0
  fan_min 50, fan_max 80, close_fan_first_layers 3, vitrification 40

For initial_layer plate temps, use the same value as the corresponding plate temp unless you have specific knowledge otherwise.

Provide all values - do not return null unless you truly cannot determine even a reasonable default.

Return a JSON object matching this schema:
{schema}"#
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

        // Original fields
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

        // New Bambu Studio specific fields
        assert!(required_strs.contains(&"nozzle_temperature"));
        assert!(required_strs.contains(&"nozzle_temperature_initial_layer"));
        assert!(required_strs.contains(&"hot_plate_temp"));
        assert!(required_strs.contains(&"hot_plate_temp_initial_layer"));
        assert!(required_strs.contains(&"cool_plate_temp"));
        assert!(required_strs.contains(&"cool_plate_temp_initial_layer"));
        assert!(required_strs.contains(&"eng_plate_temp"));
        assert!(required_strs.contains(&"eng_plate_temp_initial_layer"));
        assert!(required_strs.contains(&"textured_plate_temp"));
        assert!(required_strs.contains(&"textured_plate_temp_initial_layer"));
        assert!(required_strs.contains(&"max_volumetric_speed"));
        assert!(required_strs.contains(&"filament_flow_ratio"));
        assert!(required_strs.contains(&"pressure_advance"));
        assert!(required_strs.contains(&"fan_min_speed"));
        assert!(required_strs.contains(&"fan_max_speed"));
        assert!(required_strs.contains(&"overhang_fan_speed"));
        assert!(required_strs.contains(&"close_fan_the_first_x_layers"));
        assert!(required_strs.contains(&"additional_cooling_fan_speed"));
        assert!(required_strs.contains(&"slow_down_layer_time"));
        assert!(required_strs.contains(&"slow_down_min_speed"));
        assert!(required_strs.contains(&"deretraction_speed_mm_s"));
        assert!(required_strs.contains(&"bridge_speed"));
        assert!(required_strs.contains(&"temperature_vitrification"));
        assert!(required_strs.contains(&"filament_cost"));
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

        // New nullable integer fields
        let nozzle_temp_type = properties["nozzle_temperature"]["type"].as_array().unwrap();
        assert!(nozzle_temp_type.contains(&serde_json::json!("integer")));
        assert!(nozzle_temp_type.contains(&serde_json::json!("null")));

        let hot_plate_type = properties["hot_plate_temp"]["type"].as_array().unwrap();
        assert!(hot_plate_type.contains(&serde_json::json!("integer")));
        assert!(hot_plate_type.contains(&serde_json::json!("null")));

        let fan_min_type = properties["fan_min_speed"]["type"].as_array().unwrap();
        assert!(fan_min_type.contains(&serde_json::json!("integer")));
        assert!(fan_min_type.contains(&serde_json::json!("null")));

        let vitrification_type = properties["temperature_vitrification"]["type"].as_array().unwrap();
        assert!(vitrification_type.contains(&serde_json::json!("integer")));
        assert!(vitrification_type.contains(&serde_json::json!("null")));

        // New nullable number (float) fields
        let mvs_type = properties["max_volumetric_speed"]["type"].as_array().unwrap();
        assert!(mvs_type.contains(&serde_json::json!("number")));
        assert!(mvs_type.contains(&serde_json::json!("null")));

        let flow_type = properties["filament_flow_ratio"]["type"].as_array().unwrap();
        assert!(flow_type.contains(&serde_json::json!("number")));
        assert!(flow_type.contains(&serde_json::json!("null")));

        let pa_type = properties["pressure_advance"]["type"].as_array().unwrap();
        assert!(pa_type.contains(&serde_json::json!("number")));
        assert!(pa_type.contains(&serde_json::json!("null")));

        let cost_type = properties["filament_cost"]["type"].as_array().unwrap();
        assert!(cost_type.contains(&serde_json::json!("number")));
        assert!(cost_type.contains(&serde_json::json!("null")));
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
    fn test_json_schema_new_fields_in_required() {
        let schema = filament_specs_json_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();

        let new_fields = [
            "nozzle_temperature",
            "nozzle_temperature_initial_layer",
            "hot_plate_temp",
            "hot_plate_temp_initial_layer",
            "cool_plate_temp",
            "cool_plate_temp_initial_layer",
            "eng_plate_temp",
            "eng_plate_temp_initial_layer",
            "textured_plate_temp",
            "textured_plate_temp_initial_layer",
            "max_volumetric_speed",
            "filament_flow_ratio",
            "pressure_advance",
            "fan_min_speed",
            "fan_max_speed",
            "overhang_fan_speed",
            "close_fan_the_first_x_layers",
            "additional_cooling_fan_speed",
            "slow_down_layer_time",
            "slow_down_min_speed",
            "deretraction_speed_mm_s",
            "bridge_speed",
            "temperature_vitrification",
            "filament_cost",
        ];

        for field in &new_fields {
            assert!(
                required_strs.contains(field),
                "Field '{}' should be in the required array",
                field
            );
        }
    }

    #[test]
    fn test_json_schema_all_properties_are_required() {
        let schema = filament_specs_json_schema();
        let properties = schema["properties"].as_object().unwrap();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();

        for key in properties.keys() {
            assert!(
                required_strs.contains(&key.as_str()),
                "Property '{}' is defined but not in the required array",
                key
            );
        }
    }

    #[test]
    fn test_json_schema_new_fields_have_descriptions() {
        let schema = filament_specs_json_schema();
        let properties = schema["properties"].as_object().unwrap();

        let new_fields = [
            "nozzle_temperature",
            "nozzle_temperature_initial_layer",
            "hot_plate_temp",
            "hot_plate_temp_initial_layer",
            "cool_plate_temp",
            "cool_plate_temp_initial_layer",
            "eng_plate_temp",
            "eng_plate_temp_initial_layer",
            "textured_plate_temp",
            "textured_plate_temp_initial_layer",
            "max_volumetric_speed",
            "filament_flow_ratio",
            "pressure_advance",
            "fan_min_speed",
            "fan_max_speed",
            "overhang_fan_speed",
            "close_fan_the_first_x_layers",
            "additional_cooling_fan_speed",
            "slow_down_layer_time",
            "slow_down_min_speed",
            "deretraction_speed_mm_s",
            "bridge_speed",
            "temperature_vitrification",
            "filament_cost",
        ];

        for field in &new_fields {
            assert!(
                properties.contains_key(*field),
                "Property '{}' should exist in schema properties",
                field
            );
            let desc = properties[*field]["description"].as_str();
            assert!(
                desc.is_some() && !desc.unwrap().is_empty(),
                "Property '{}' should have a non-empty description",
                field
            );
        }
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
