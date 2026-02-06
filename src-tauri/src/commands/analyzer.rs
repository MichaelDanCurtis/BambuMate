//! Tauri commands for AI print analysis.
//!
//! Provides the analyze_print command that:
//! 1. Loads the current profile for context
//! 2. Calls the vision API for defect detection
//! 3. Runs defects through the rule engine for recommendations

use std::collections::HashMap;
use std::path::Path;

use base64::Engine;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

use crate::analyzer::{analyze_image, DefectReport};
use crate::mapper::{default_rules, Conflict, RuleEngine};
use crate::profile::FilamentProfile;
use crate::scraper::types::MaterialType;

/// Request payload for print analysis.
#[derive(Debug, Deserialize)]
pub struct AnalyzeRequest {
    /// Base64-encoded image data (from frontend FileReader)
    pub image_base64: String,
    /// Path to the profile to use for context (optional)
    pub profile_path: Option<String>,
    /// Material type override (if not using profile)
    pub material_type: Option<String>,
}

/// Full analysis response including defects, recommendations, and conflicts.
#[derive(Debug, Serialize)]
pub struct AnalyzeResponse {
    /// AI-detected defects with severity and confidence
    pub defect_report: DefectReport,
    /// Parameter recommendations from rule engine
    pub recommendations: Vec<RecommendationDisplay>,
    /// Detected conflicts between recommendations
    pub conflicts: Vec<Conflict>,
    /// Current profile values used for analysis
    pub current_values: HashMap<String, f32>,
    /// Material type used for safe-range enforcement
    pub material_type: String,
}

/// Recommendation with display-friendly formatting.
#[derive(Debug, Serialize)]
pub struct RecommendationDisplay {
    /// Which defect triggered this
    pub defect: String,
    /// Parameter name (Bambu Studio)
    pub parameter: String,
    /// Display label for the parameter
    pub parameter_label: String,
    /// Current value
    pub current_value: f32,
    /// Recommended new value
    pub recommended_value: f32,
    /// Formatted change string (e.g., "215 -> 210")
    pub change_display: String,
    /// Unit for the parameter
    pub unit: String,
    /// Priority (1 = most important)
    pub priority: u8,
    /// Why this change helps
    pub rationale: String,
    /// Was this clamped to safe range?
    pub was_clamped: bool,
}

/// Analyze a print photo for defects and generate recommendations.
///
/// # Arguments
/// * `app` - Tauri app handle for accessing preferences
/// * `request` - Analysis request with image and optional profile
///
/// # Returns
/// Complete analysis with defects, recommendations, and conflicts.
#[tauri::command]
pub async fn analyze_print(
    app: tauri::AppHandle,
    request: AnalyzeRequest,
) -> Result<AnalyzeResponse, String> {
    info!("Starting print analysis");

    // Decode base64 image
    let image_bytes = base64::engine::general_purpose::STANDARD
        .decode(&request.image_base64)
        .map_err(|e| format!("Invalid base64 image data: {}", e))?;

    // Get AI provider settings
    let (provider, model, api_key) = get_ai_settings(&app)?;

    // Load profile for current values (or use defaults)
    let (current_values, material_type) = if let Some(ref profile_path) = request.profile_path {
        load_profile_context(profile_path)?
    } else {
        let material = request
            .material_type
            .clone()
            .unwrap_or_else(|| "PLA".to_string());
        (default_profile_values(), material)
    };

    // Call vision API
    let defect_report = analyze_image(
        &image_bytes,
        &current_values,
        &material_type,
        &provider,
        &model,
        &api_key,
    )
    .await?;

    // Run through rule engine for recommendations
    let material = MaterialType::from_str(&material_type);
    let engine = RuleEngine::new(default_rules());
    let evaluation = engine.evaluate(&defect_report.defects, &current_values, &material);

    // Format recommendations for display
    let recommendations = format_recommendations(&evaluation, &current_values);

    info!(
        "Analysis complete: {} defects, {} recommendations, {} conflicts",
        defect_report.defects.len(),
        recommendations.len(),
        evaluation.conflicts.len()
    );

    Ok(AnalyzeResponse {
        defect_report,
        recommendations,
        conflicts: evaluation.conflicts,
        current_values,
        material_type,
    })
}

/// Get AI provider settings from preferences and keychain.
fn get_ai_settings(app: &tauri::AppHandle) -> Result<(String, String, String), String> {
    // Get provider preference (default to claude)
    let store = app.store("preferences.json").map_err(|e| {
        warn!("Failed to open preferences store: {}", e);
        e.to_string()
    })?;

    let provider = store
        .get("ai_provider")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "claude".to_string());

    // Get model preference (default based on provider)
    let default_model = match provider.as_str() {
        "claude" => "claude-sonnet-4-20250514",
        "openai" => "gpt-4o",
        "kimi" => "moonshot-v1-128k",
        "openrouter" => "anthropic/claude-sonnet-4",
        _ => "claude-sonnet-4-20250514",
    };
    let model = store
        .get("ai_model")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| default_model.to_string());

    // Get API key from keychain
    let service = match provider.as_str() {
        "claude" => "bambumate-claude-api",
        "openai" => "bambumate-openai-api",
        "kimi" => "bambumate-kimi-api",
        "openrouter" => "bambumate-openrouter-api",
        _ => return Err(format!("Unknown AI provider: {}", provider)),
    };

    let entry = Entry::new(service, "bambumate").map_err(|e| e.to_string())?;
    let api_key = match entry.get_password() {
        Ok(key) => key,
        Err(keyring::Error::NoEntry) => {
            return Err(format!(
                "No API key configured for '{}'. Please set it in Settings.",
                provider
            ))
        }
        Err(e) => return Err(format!("Failed to read API key for '{}': {}", provider, e)),
    };

    Ok((provider, model, api_key))
}

/// Load current values and material type from a profile.
fn load_profile_context(profile_path: &str) -> Result<(HashMap<String, f32>, String), String> {
    let path = Path::new(profile_path);

    // Read the profile
    let profile = crate::profile::reader::read_profile(path)
        .map_err(|e| format!("Failed to load profile: {}", e))?;

    // Extract current values for key parameters
    let current_values = extract_profile_values(&profile);

    // Detect material type from profile name or inherits field
    let material_type = detect_material_type(&profile);

    Ok((current_values, material_type))
}

/// Extract relevant parameter values from a profile.
fn extract_profile_values(profile: &FilamentProfile) -> HashMap<String, f32> {
    let mut values = HashMap::new();

    // Helper to extract float from string array or single value
    let extract_float = |key: &str| -> Option<f32> {
        profile.raw().get(key).and_then(|v| {
            if let Some(arr) = v.as_array() {
                arr.first()
                    .and_then(|s| s.as_str())
                    .and_then(|s| s.parse().ok())
            } else if let Some(s) = v.as_str() {
                s.parse().ok()
            } else if let Some(n) = v.as_f64() {
                Some(n as f32)
            } else {
                None
            }
        })
    };

    // Temperature parameters
    if let Some(v) = extract_float("nozzle_temperature") {
        values.insert("nozzle_temperature".to_string(), v);
    }
    if let Some(v) = extract_float("nozzle_temperature_initial_layer") {
        values.insert("nozzle_temperature_initial_layer".to_string(), v);
    }
    if let Some(v) = extract_float("cool_plate_temp") {
        values.insert("cool_plate_temp".to_string(), v);
    }
    if let Some(v) = extract_float("hot_plate_temp") {
        values.insert("hot_plate_temp".to_string(), v);
    }
    if let Some(v) = extract_float("textured_plate_temp") {
        values.insert("textured_plate_temp".to_string(), v);
    }

    // Retraction parameters
    if let Some(v) = extract_float("filament_retraction_length") {
        values.insert("filament_retraction_length".to_string(), v);
    }
    if let Some(v) = extract_float("filament_retraction_speed") {
        values.insert("filament_retraction_speed".to_string(), v);
    }

    // Flow and pressure
    if let Some(v) = extract_float("filament_flow_ratio") {
        values.insert("filament_flow_ratio".to_string(), v);
    }
    if let Some(v) = extract_float("pressure_advance") {
        values.insert("pressure_advance".to_string(), v);
    }

    // Cooling
    if let Some(v) = extract_float("fan_min_speed") {
        values.insert("fan_min_speed".to_string(), v);
    }
    if let Some(v) = extract_float("fan_max_speed") {
        values.insert("fan_max_speed".to_string(), v);
    }
    if let Some(v) = extract_float("overhang_fan_speed") {
        values.insert("overhang_fan_speed".to_string(), v);
    }

    values
}

/// Detect material type from profile data.
fn detect_material_type(profile: &FilamentProfile) -> String {
    // Check filament_type field first
    if let Some(ft) = profile.raw().get("filament_type") {
        if let Some(arr) = ft.as_array() {
            if let Some(s) = arr.first().and_then(|v| v.as_str()) {
                return s.to_string();
            }
        } else if let Some(s) = ft.as_str() {
            return s.to_string();
        }
    }

    // Check inherits field for material hint
    if let Some(inherits) = profile.raw().get("inherits") {
        if let Some(s) = inherits.as_str() {
            // Parse "Generic PLA" -> "PLA"
            for material in ["PLA", "PETG", "ABS", "ASA", "TPU", "PA", "PC", "PVA", "HIPS"] {
                if s.to_uppercase().contains(material) {
                    return material.to_string();
                }
            }
        }
    }

    // Default to PLA
    "PLA".to_string()
}

/// Default profile values when no profile is loaded.
fn default_profile_values() -> HashMap<String, f32> {
    let mut values = HashMap::new();
    values.insert("nozzle_temperature".to_string(), 200.0);
    values.insert("cool_plate_temp".to_string(), 60.0);
    values.insert("filament_retraction_length".to_string(), 0.8);
    values.insert("filament_flow_ratio".to_string(), 1.0);
    values.insert("fan_min_speed".to_string(), 35.0);
    values.insert("fan_max_speed".to_string(), 70.0);
    values
}

/// Format recommendations with display-friendly labels.
fn format_recommendations(
    evaluation: &crate::mapper::EvaluationResult,
    _current_values: &HashMap<String, f32>,
) -> Vec<RecommendationDisplay> {
    evaluation
        .recommendations
        .iter()
        .map(|rec| {
            let (label, unit) = parameter_display_info(&rec.parameter);
            let current = rec.current_value;
            let recommended = rec.recommended_value;

            RecommendationDisplay {
                defect: rec.defect.clone(),
                parameter: rec.parameter.clone(),
                parameter_label: label,
                current_value: current,
                recommended_value: recommended,
                change_display: format_change(current, recommended, &unit),
                unit,
                priority: rec.priority,
                rationale: rec.rationale.clone(),
                was_clamped: rec.was_clamped,
            }
        })
        .collect()
}

/// Get display label and unit for a parameter.
fn parameter_display_info(param: &str) -> (String, String) {
    match param {
        "nozzle_temperature" => ("Nozzle Temperature".to_string(), "C".to_string()),
        "nozzle_temperature_initial_layer" => {
            ("Initial Layer Nozzle Temp".to_string(), "C".to_string())
        }
        "cool_plate_temp" => ("Bed Temperature (Cool Plate)".to_string(), "C".to_string()),
        "hot_plate_temp" => ("Bed Temperature (Hot Plate)".to_string(), "C".to_string()),
        "textured_plate_temp" => ("Bed Temperature (Textured)".to_string(), "C".to_string()),
        "filament_retraction_length" => ("Retraction Length".to_string(), "mm".to_string()),
        "filament_retraction_speed" => ("Retraction Speed".to_string(), "mm/s".to_string()),
        "filament_flow_ratio" => ("Flow Ratio".to_string(), "".to_string()),
        "pressure_advance" => ("Pressure Advance".to_string(), "".to_string()),
        "fan_min_speed" => ("Min Fan Speed".to_string(), "%".to_string()),
        "fan_max_speed" => ("Max Fan Speed".to_string(), "%".to_string()),
        "overhang_fan_speed" => ("Overhang Fan Speed".to_string(), "%".to_string()),
        _ => (param.replace('_', " "), "".to_string()),
    }
}

/// Format a value change for display.
fn format_change(current: f32, recommended: f32, unit: &str) -> String {
    if unit == "C" || unit == "%" || unit == "mm/s" {
        // Integer display for these
        format!("{:.0} -> {:.0}{}", current, recommended, unit)
    } else if unit == "mm" {
        format!("{:.1} -> {:.1}{}", current, recommended, unit)
    } else {
        // Generic with 2 decimal places
        format!("{:.2} -> {:.2}", current, recommended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_display_info() {
        let (label, unit) = parameter_display_info("nozzle_temperature");
        assert_eq!(label, "Nozzle Temperature");
        assert_eq!(unit, "C");

        let (label, unit) = parameter_display_info("filament_retraction_length");
        assert_eq!(label, "Retraction Length");
        assert_eq!(unit, "mm");
    }

    #[test]
    fn test_format_change() {
        assert_eq!(format_change(215.0, 210.0, "C"), "215 -> 210C");
        assert_eq!(format_change(0.8, 1.2, "mm"), "0.8 -> 1.2mm");
        assert_eq!(format_change(1.0, 0.95, ""), "1.00 -> 0.95");
    }

    #[test]
    fn test_default_profile_values() {
        let values = default_profile_values();
        assert!(values.contains_key("nozzle_temperature"));
        assert!(values.contains_key("filament_retraction_length"));
        assert_eq!(values.get("nozzle_temperature"), Some(&200.0));
    }

    #[test]
    fn test_detect_material_type_default() {
        use serde_json::json;

        let json_str = serde_json::to_string(&json!({})).unwrap();
        let profile = FilamentProfile::from_json(&json_str).unwrap();
        assert_eq!(detect_material_type(&profile), "PLA");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::mapper::DetectedDefect;

    #[test]
    fn test_format_recommendations_from_evaluation() {
        let defects = vec![DetectedDefect {
            defect_type: "stringing".to_string(),
            severity: 0.6,
            confidence: 0.85,
        }];

        let mut current_values = HashMap::new();
        current_values.insert("filament_retraction_length".to_string(), 0.8);
        current_values.insert("nozzle_temperature".to_string(), 210.0);

        let engine = RuleEngine::new(default_rules());
        let evaluation = engine.evaluate(&defects, &current_values, &MaterialType::PLA);

        let recommendations = format_recommendations(&evaluation, &current_values);

        // Should have retraction recommendation for stringing
        assert!(!recommendations.is_empty(), "Should produce recommendations");

        let retraction_rec = recommendations
            .iter()
            .find(|r| r.parameter == "filament_retraction_length");
        assert!(
            retraction_rec.is_some(),
            "Should have retraction recommendation"
        );

        let rec = retraction_rec.unwrap();
        assert!(rec.change_display.contains("->"), "Should show change");
        assert_eq!(rec.unit, "mm");
    }

    #[test]
    fn test_extract_profile_values_with_arrays() {
        use serde_json::json;

        // Bambu Studio stores many values as string arrays
        let json_str = serde_json::to_string(&json!({
            "nozzle_temperature": ["215"],
            "cool_plate_temp": ["55"],
            "filament_retraction_length": ["0.8"],
            "filament_type": ["PLA"]
        }))
        .unwrap();

        let profile = FilamentProfile::from_json(&json_str).unwrap();

        let values = extract_profile_values(&profile);
        assert_eq!(values.get("nozzle_temperature"), Some(&215.0));
        assert_eq!(values.get("cool_plate_temp"), Some(&55.0));
        assert_eq!(values.get("filament_retraction_length"), Some(&0.8));
    }

    #[test]
    fn test_detect_material_type_from_inherits() {
        use serde_json::json;

        let json_str = serde_json::to_string(&json!({
            "inherits": "Generic PETG @BBL"
        }))
        .unwrap();

        let profile = FilamentProfile::from_json(&json_str).unwrap();

        assert_eq!(detect_material_type(&profile), "PETG");
    }

    #[test]
    fn test_detect_material_type_from_filament_type() {
        use serde_json::json;

        let json_str = serde_json::to_string(&json!({
            "filament_type": ["ABS"]
        }))
        .unwrap();

        let profile = FilamentProfile::from_json(&json_str).unwrap();

        assert_eq!(detect_material_type(&profile), "ABS");
    }

    #[test]
    fn test_full_pipeline_with_mock_defects() {
        // Test the full format_recommendations pipeline
        let mut current_values = HashMap::new();
        current_values.insert("nozzle_temperature".to_string(), 220.0);
        current_values.insert("filament_retraction_length".to_string(), 0.5);
        current_values.insert("cool_plate_temp".to_string(), 65.0);

        let defects = vec![
            DetectedDefect {
                defect_type: "stringing".to_string(),
                severity: 0.7,
                confidence: 0.9,
            },
            DetectedDefect {
                defect_type: "warping".to_string(),
                severity: 0.4,
                confidence: 0.75,
            },
        ];

        let engine = RuleEngine::new(default_rules());
        let evaluation = engine.evaluate(&defects, &current_values, &MaterialType::PLA);
        let recommendations = format_recommendations(&evaluation, &current_values);

        // Should have recommendations for both defects
        let stringing_recs: Vec<_> = recommendations
            .iter()
            .filter(|r| r.defect == "stringing")
            .collect();
        let warping_recs: Vec<_> = recommendations
            .iter()
            .filter(|r| r.defect == "warping")
            .collect();

        assert!(
            !stringing_recs.is_empty(),
            "Should have stringing recommendations"
        );
        assert!(
            !warping_recs.is_empty(),
            "Should have warping recommendations"
        );
    }

    #[test]
    fn test_recommendation_display_fields() {
        let mut current_values = HashMap::new();
        current_values.insert("nozzle_temperature".to_string(), 215.0);

        let defects = vec![DetectedDefect {
            defect_type: "stringing".to_string(),
            severity: 0.5,
            confidence: 0.8,
        }];

        let engine = RuleEngine::new(default_rules());
        let evaluation = engine.evaluate(&defects, &current_values, &MaterialType::PLA);
        let recommendations = format_recommendations(&evaluation, &current_values);

        // Check that all display fields are populated
        for rec in &recommendations {
            assert!(!rec.defect.is_empty(), "Defect should be set");
            assert!(!rec.parameter.is_empty(), "Parameter should be set");
            assert!(!rec.parameter_label.is_empty(), "Label should be set");
            assert!(
                rec.change_display.contains("->"),
                "Change display should show arrow"
            );
            assert!(!rec.rationale.is_empty(), "Rationale should be set");
        }
    }
}
