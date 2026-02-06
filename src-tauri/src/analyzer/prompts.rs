//! Prompts and schemas for defect analysis vision API calls.

/// JSON schema placeholder - implemented in Task 3
pub fn defect_report_schema() -> serde_json::Value {
    serde_json::json!({})
}

/// Build prompt placeholder - implemented in Task 3
pub fn build_defect_analysis_prompt(
    _current_settings: &std::collections::HashMap<String, f32>,
    _material_type: &str,
) -> String {
    String::new()
}
