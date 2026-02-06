//! Vision API calls for defect analysis across all supported providers.
//!
//! Extends the pattern from scraper/extraction.rs to support image content.

use std::collections::HashMap;

use super::types::DefectReport;

/// Analyze an image for print defects using the specified AI provider.
pub async fn analyze_image(
    _image_bytes: &[u8],
    _current_settings: &HashMap<String, f32>,
    _material_type: &str,
    _provider: &str,
    _model: &str,
    _api_key: &str,
) -> Result<DefectReport, String> {
    todo!("Implemented in Task 3")
}
