//! Type definitions for the defect-to-parameter mapping engine.
//!
//! These types support both TOML deserialization (for loading rules)
//! and JSON serialization (for frontend communication).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// =============================================================================
// CONFIGURATION TYPES (loaded from TOML)
// =============================================================================

/// Root configuration loaded from defect_rules.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct RulesConfig {
    /// Defect type definitions keyed by defect ID (e.g., "stringing", "warping")
    pub defects: HashMap<String, DefectInfo>,
    /// List of adjustment rules for each defect type
    pub rules: Vec<DefectRule>,
    /// Known parameter conflicts
    #[serde(default)]
    pub conflicts: Vec<ConflictDefinition>,
}

/// Information about a defect type.
#[derive(Debug, Clone, Deserialize)]
pub struct DefectInfo {
    /// Human-readable name for display
    pub display_name: String,
    /// Description of the defect for user education
    pub description: String,
    /// Valid severity range [min, max] for this defect type
    pub severity_range: [f32; 2],
}

/// A rule mapping a defect to parameter adjustments.
#[derive(Debug, Clone, Deserialize)]
pub struct DefectRule {
    /// The defect type this rule applies to (must match key in defects map)
    pub defect: String,
    /// Minimum severity threshold for this rule to activate (None = always active)
    #[serde(default)]
    pub severity_min: Option<f32>,
    /// Parameter adjustments to apply when this rule activates
    pub adjustments: Vec<Adjustment>,
}

/// A parameter adjustment within a defect rule.
#[derive(Debug, Clone, Deserialize)]
pub struct Adjustment {
    /// Bambu Studio parameter name (e.g., "filament_retraction_length")
    pub parameter: String,
    /// How to modify the parameter
    pub operation: Operation,
    /// Amount of adjustment (interpretation depends on operation)
    pub amount: f32,
    /// Unit for display (e.g., "mm", "C", "%")
    pub unit: String,
    /// Priority ranking (1 = primary fix, higher = secondary)
    pub priority: u8,
    /// Explanation of why this adjustment helps
    pub rationale: String,
}

/// Types of parameter modification operations.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    /// Add the amount to the current value
    Increase,
    /// Subtract the amount from the current value
    Decrease,
    /// Set to an absolute value
    Set,
}

/// Definition of a known conflict between parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct ConflictDefinition {
    /// Conflict identifier
    pub name: String,
    /// Human-readable description of the conflict
    pub description: String,
    /// Parameters involved in this conflict
    pub parameters: Vec<String>,
}

// =============================================================================
// OUTPUT TYPES (serialized to frontend)
// =============================================================================

/// A single parameter recommendation produced by the rule engine.
#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    /// Which defect triggered this recommendation
    pub defect: String,
    /// Bambu Studio parameter to adjust
    pub parameter: String,
    /// Current value from the profile
    pub current_value: f32,
    /// Recommended new value
    pub recommended_value: f32,
    /// Priority (1 = most important)
    pub priority: u8,
    /// Explanation of why this change helps
    pub rationale: String,
    /// True if the recommended value was clamped to safe range
    pub was_clamped: bool,
}

/// A detected conflict between recommendations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    /// Parameter(s) involved in the conflict
    pub parameter: String,
    /// Defect types that conflict on this parameter
    pub conflicting_defects: Vec<String>,
    /// Description of the conflict
    pub description: String,
}

/// Complete result of rule engine evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct EvaluationResult {
    /// Ranked list of parameter recommendations (sorted by priority)
    pub recommendations: Vec<Recommendation>,
    /// Detected conflicts between recommendations
    pub conflicts: Vec<Conflict>,
}

// =============================================================================
// INPUT TYPES (from AI analysis in Phase 6)
// =============================================================================

/// A defect detected by AI vision analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedDefect {
    /// Defect type identifier (must match key in defects map)
    pub defect_type: String,
    /// Severity of the defect (0.0 = barely noticeable, 1.0 = severe)
    pub severity: f32,
    /// AI's confidence in the detection (0.0 - 1.0)
    pub confidence: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_deserialize() {
        let json = r#""increase""#;
        let op: Operation = serde_json::from_str(json).unwrap();
        assert_eq!(op, Operation::Increase);

        let json = r#""decrease""#;
        let op: Operation = serde_json::from_str(json).unwrap();
        assert_eq!(op, Operation::Decrease);

        let json = r#""set""#;
        let op: Operation = serde_json::from_str(json).unwrap();
        assert_eq!(op, Operation::Set);
    }

    #[test]
    fn test_detected_defect_deserialize() {
        let json = r#"{
            "defect_type": "stringing",
            "severity": 0.7,
            "confidence": 0.85
        }"#;
        let defect: DetectedDefect = serde_json::from_str(json).unwrap();
        assert_eq!(defect.defect_type, "stringing");
        assert_eq!(defect.severity, 0.7);
        assert_eq!(defect.confidence, 0.85);
    }

    #[test]
    fn test_recommendation_serialize() {
        let rec = Recommendation {
            defect: "stringing".to_string(),
            parameter: "filament_retraction_length".to_string(),
            current_value: 0.8,
            recommended_value: 1.3,
            priority: 1,
            rationale: "Increase retraction to reduce stringing".to_string(),
            was_clamped: false,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("stringing"));
        assert!(json.contains("filament_retraction_length"));
    }

    #[test]
    fn test_evaluation_result_serialize() {
        let result = EvaluationResult {
            recommendations: vec![],
            conflicts: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("recommendations"));
        assert!(json.contains("conflicts"));
    }
}
