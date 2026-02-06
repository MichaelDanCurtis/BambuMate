//! Type definitions for AI vision analysis.
//!
//! These types support structured output from vision API calls
//! and integration with the defect mapping engine.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::mapper::{Conflict, DetectedDefect, Recommendation};

/// Result of AI vision analysis of a 3D print photo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefectReport {
    /// List of detected defects with severity and confidence
    pub defects: Vec<DetectedDefect>,
    /// Overall print quality assessment
    pub overall_quality: String,
    /// Optional notes from the AI about the analysis
    pub notes: Option<String>,
}

/// Request structure for image analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRequest {
    /// Raw image bytes (JPEG, PNG, WebP, etc.)
    #[serde(skip)]
    pub image_bytes: Vec<u8>,
    /// Path to the current profile (for loading settings)
    pub profile_path: Option<String>,
    /// Material type for safe-range enforcement (e.g., "PLA", "PETG")
    pub material_type: Option<String>,
}

/// Complete analysis result combining vision output with rule engine recommendations.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    /// Raw defect report from vision API
    pub defect_report: DefectReport,
    /// Parameter recommendations from rule engine
    pub recommendations: Vec<Recommendation>,
    /// Detected conflicts between recommendations
    pub conflicts: Vec<Conflict>,
    /// Current profile values used for context
    pub profile_context: HashMap<String, f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defect_report_serialize() {
        let report = DefectReport {
            defects: vec![DetectedDefect {
                defect_type: "stringing".to_string(),
                severity: 0.6,
                confidence: 0.85,
            }],
            overall_quality: "acceptable".to_string(),
            notes: Some("Minor stringing visible".to_string()),
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("stringing"));
        assert!(json.contains("acceptable"));
    }

    #[test]
    fn test_defect_report_deserialize() {
        let json = r#"{
            "defects": [
                {"defect_type": "warping", "severity": 0.7, "confidence": 0.9}
            ],
            "overall_quality": "poor",
            "notes": null
        }"#;

        let report: DefectReport = serde_json::from_str(json).unwrap();
        assert_eq!(report.defects.len(), 1);
        assert_eq!(report.defects[0].defect_type, "warping");
        assert_eq!(report.overall_quality, "poor");
        assert!(report.notes.is_none());
    }

    #[test]
    fn test_analysis_request_default() {
        let request = AnalysisRequest {
            image_bytes: vec![1, 2, 3],
            profile_path: None,
            material_type: Some("PLA".to_string()),
        };

        assert_eq!(request.image_bytes.len(), 3);
        assert!(request.profile_path.is_none());
        assert_eq!(request.material_type, Some("PLA".to_string()));
    }

    #[test]
    fn test_analysis_result_serialize() {
        let result = AnalysisResult {
            defect_report: DefectReport {
                defects: vec![],
                overall_quality: "excellent".to_string(),
                notes: None,
            },
            recommendations: vec![],
            conflicts: vec![],
            profile_context: HashMap::new(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("defect_report"));
        assert!(json.contains("recommendations"));
        assert!(json.contains("excellent"));
    }
}
