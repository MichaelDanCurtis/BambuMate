//! TOML rule loading for the defect mapping engine.
//!
//! Provides two loading methods:
//! - `default_rules()` - Loads embedded rules compiled into the binary
//! - `load_rules(path)` - Loads custom rules from a file path

use anyhow::Result;
use std::path::Path;

use super::types::RulesConfig;

/// Default rules embedded in the binary at compile time.
/// These are loaded from `src-tauri/config/defect_rules.toml`.
const DEFAULT_RULES: &str = include_str!("../../config/defect_rules.toml");

/// Load rules from a TOML file at the given path.
///
/// # Arguments
/// * `path` - Path to the TOML file containing rules
///
/// # Returns
/// * `Ok(RulesConfig)` - Parsed rules configuration
/// * `Err` - If file cannot be read or TOML is invalid
///
/// # Example
/// ```ignore
/// let rules = load_rules(Path::new("/path/to/custom_rules.toml"))?;
/// ```
pub fn load_rules(path: &Path) -> Result<RulesConfig> {
    let content = std::fs::read_to_string(path)?;
    let config: RulesConfig = toml::from_str(&content)?;
    Ok(config)
}

/// Get the default rules embedded in the binary.
///
/// These rules cover the 7 common 3D printing defects:
/// - Stringing/oozing
/// - Warping/lifting
/// - Poor layer adhesion
/// - Elephant's foot
/// - Under-extrusion
/// - Over-extrusion
/// - Z-banding
///
/// # Panics
/// Panics if the embedded TOML is invalid (this would be a compile-time bug).
///
/// # Example
/// ```ignore
/// use bambumate::mapper::{default_rules, RuleEngine};
///
/// let rules = default_rules();
/// let engine = RuleEngine::new(rules);
/// ```
pub fn default_rules() -> RulesConfig {
    toml::from_str(DEFAULT_RULES).expect("embedded defect_rules.toml must be valid TOML")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_rules_loads() {
        let rules = default_rules();
        assert!(!rules.defects.is_empty(), "Should have defect definitions");
        assert!(!rules.rules.is_empty(), "Should have rules");
    }

    #[test]
    fn test_default_rules_has_seven_defect_types() {
        let rules = default_rules();
        assert_eq!(
            rules.defects.len(),
            7,
            "Should have exactly 7 defect types"
        );

        // Verify expected defect types exist
        assert!(rules.defects.contains_key("stringing"));
        assert!(rules.defects.contains_key("warping"));
        assert!(rules.defects.contains_key("layer_adhesion"));
        assert!(rules.defects.contains_key("elephants_foot"));
        assert!(rules.defects.contains_key("under_extrusion"));
        assert!(rules.defects.contains_key("over_extrusion"));
        assert!(rules.defects.contains_key("z_banding"));
    }

    #[test]
    fn test_stringing_rules_exist() {
        let rules = default_rules();
        let stringing_rules: Vec<_> = rules
            .rules
            .iter()
            .filter(|r| r.defect == "stringing")
            .collect();

        assert!(
            !stringing_rules.is_empty(),
            "Should have stringing rules"
        );

        // Should have retraction adjustment
        let has_retraction = stringing_rules.iter().any(|r| {
            r.adjustments
                .iter()
                .any(|a| a.parameter == "filament_retraction_length")
        });
        assert!(has_retraction, "Stringing rules should adjust retraction");
    }

    #[test]
    fn test_warping_rules_exist() {
        let rules = default_rules();
        let warping_rules: Vec<_> = rules
            .rules
            .iter()
            .filter(|r| r.defect == "warping")
            .collect();

        assert!(!warping_rules.is_empty(), "Should have warping rules");

        // Should have bed temp adjustment
        let has_bed_temp = warping_rules.iter().any(|r| {
            r.adjustments.iter().any(|a| {
                a.parameter.contains("plate_temp")
            })
        });
        assert!(has_bed_temp, "Warping rules should adjust bed temp");
    }

    #[test]
    fn test_conflicts_defined() {
        let rules = default_rules();
        assert!(
            !rules.conflicts.is_empty(),
            "Should have conflict definitions"
        );

        // Should have retraction/extrusion conflict
        let has_retraction_conflict = rules.conflicts.iter().any(|c| {
            c.parameters.contains(&"filament_retraction_length".to_string())
        });
        assert!(
            has_retraction_conflict,
            "Should have retraction conflict defined"
        );
    }

    #[test]
    fn test_severity_thresholds() {
        let rules = default_rules();

        // Some rules should have severity thresholds
        let rules_with_threshold: Vec<_> = rules
            .rules
            .iter()
            .filter(|r| r.severity_min.is_some())
            .collect();

        assert!(
            !rules_with_threshold.is_empty(),
            "Some rules should have severity thresholds"
        );

        // Severe stringing rule should have threshold around 0.7
        let severe_stringing = rules
            .rules
            .iter()
            .find(|r| r.defect == "stringing" && r.severity_min.is_some());

        if let Some(rule) = severe_stringing {
            let threshold = rule.severity_min.unwrap();
            assert!(
                threshold >= 0.5 && threshold <= 0.9,
                "Severe stringing threshold should be in reasonable range"
            );
        }
    }

    #[test]
    fn test_adjustments_have_required_fields() {
        let rules = default_rules();

        for rule in &rules.rules {
            for adj in &rule.adjustments {
                assert!(!adj.parameter.is_empty(), "Parameter should not be empty");
                assert!(!adj.unit.is_empty(), "Unit should not be empty");
                assert!(!adj.rationale.is_empty(), "Rationale should not be empty");
                assert!(adj.priority >= 1, "Priority should be at least 1");
            }
        }
    }
}
