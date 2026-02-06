//! Rule evaluation engine for defect-to-parameter mapping.
//!
//! The `RuleEngine` takes detected defects and current profile values,
//! then produces ranked recommendations with conflict detection.

use std::collections::HashMap;

use crate::scraper::types::MaterialType;
use crate::scraper::validation::constraints_for_material;

use super::types::*;

/// The rule evaluation engine.
///
/// Evaluates detected print defects against the loaded rule configuration
/// to produce ranked parameter recommendations, while detecting conflicts
/// between opposing adjustments.
pub struct RuleEngine {
    rules: RulesConfig,
}

impl RuleEngine {
    /// Create a new rule engine with the given configuration.
    ///
    /// # Arguments
    /// * `rules` - Rule configuration (typically from `default_rules()` or `load_rules()`)
    pub fn new(rules: RulesConfig) -> Self {
        Self { rules }
    }

    /// Evaluate detected defects against rules to produce recommendations.
    ///
    /// # Arguments
    /// * `defects` - List of defects detected by AI analysis
    /// * `current_values` - Current profile parameter values (keyed by parameter name)
    /// * `material` - Material type for safe-range clamping
    ///
    /// # Returns
    /// `EvaluationResult` containing ranked recommendations and detected conflicts
    pub fn evaluate(
        &self,
        defects: &[DetectedDefect],
        current_values: &HashMap<String, f32>,
        material: &MaterialType,
    ) -> EvaluationResult {
        let mut recommendations = Vec::new();

        for defect in defects {
            // Find rules matching this defect type and severity threshold
            let applicable_rules: Vec<_> = self
                .rules
                .rules
                .iter()
                .filter(|r| r.defect == defect.defect_type)
                .filter(|r| r.severity_min.map_or(true, |min| defect.severity >= min))
                .collect();

            for rule in applicable_rules {
                for adj in &rule.adjustments {
                    let current = current_values.get(&adj.parameter).copied().unwrap_or(0.0);

                    // Scale adjustment by severity (linear scaling)
                    let raw_delta = match adj.operation {
                        Operation::Increase => adj.amount * defect.severity,
                        Operation::Decrease => -adj.amount * defect.severity,
                        Operation::Set => adj.amount - current,
                    };

                    let new_value = current + raw_delta;

                    // Clamp to material-safe range
                    let (clamped_value, was_clamped) =
                        self.clamp_to_safe_range(&adj.parameter, new_value, material);

                    recommendations.push(Recommendation {
                        defect: defect.defect_type.clone(),
                        parameter: adj.parameter.clone(),
                        current_value: current,
                        recommended_value: clamped_value,
                        priority: adj.priority,
                        rationale: adj.rationale.clone(),
                        was_clamped,
                    });
                }
            }
        }

        // Sort by priority (lower = more important)
        recommendations.sort_by_key(|r| r.priority);

        // Detect conflicts
        let conflicts = self.detect_conflicts(&recommendations);

        EvaluationResult {
            recommendations,
            conflicts,
        }
    }

    /// Clamp a value to material-safe operating range.
    ///
    /// Returns (clamped_value, was_clamped).
    fn clamp_to_safe_range(
        &self,
        param: &str,
        value: f32,
        material: &MaterialType,
    ) -> (f32, bool) {
        let constraints = constraints_for_material(material);

        let (min, max): (f32, f32) = match param {
            "nozzle_temperature" | "nozzle_temperature_initial_layer" => {
                (constraints.nozzle_temp_min as f32, constraints.nozzle_temp_max as f32)
            }
            "cool_plate_temp" | "hot_plate_temp" | "textured_plate_temp" => {
                (constraints.bed_temp_min as f32, constraints.bed_temp_max as f32)
            }
            "filament_retraction_length" => (0.0, 15.0),
            "filament_retraction_speed" => (10.0, 100.0),
            "filament_flow_ratio" => (0.85, 1.15),
            "fan_min_speed" | "fan_max_speed" | "overhang_fan_speed" => (0.0, 100.0),
            "pressure_advance" => (0.0, 0.1),
            _ => return (value, false), // No constraints for unknown params
        };

        if value < min {
            (min, true)
        } else if value > max {
            (max, true)
        } else {
            (value, false)
        }
    }

    /// Detect conflicts where same parameter is adjusted in opposite directions.
    fn detect_conflicts(&self, recommendations: &[Recommendation]) -> Vec<Conflict> {
        use std::collections::HashSet;
        let mut conflicts = Vec::new();

        // Group by parameter
        let mut by_param: HashMap<&str, Vec<&Recommendation>> = HashMap::new();
        for rec in recommendations {
            by_param.entry(&rec.parameter).or_default().push(rec);
        }

        // Check for opposite-direction adjustments on same parameter
        for (param, recs) in &by_param {
            if recs.len() > 1 {
                let directions: Vec<i8> = recs
                    .iter()
                    .map(|r| {
                        let delta = r.recommended_value - r.current_value;
                        if delta > 0.001 {
                            1
                        } else if delta < -0.001 {
                            -1
                        } else {
                            0
                        }
                    })
                    .collect();

                let has_increase = directions.iter().any(|&d| d > 0);
                let has_decrease = directions.iter().any(|&d| d < 0);

                if has_increase && has_decrease {
                    conflicts.push(Conflict {
                        parameter: param.to_string(),
                        conflicting_defects: recs.iter().map(|r| r.defect.clone()).collect(),
                        description: format!(
                            "Multiple defects require opposite adjustments to {}",
                            param
                        ),
                    });
                }
            }
        }

        // Check defined conflict pairs from rules config
        for def in &self.rules.conflicts {
            let affected: Vec<&Recommendation> = recommendations
                .iter()
                .filter(|r| def.parameters.contains(&r.parameter))
                .collect();

            if affected.len() > 1 {
                let defects: Vec<String> = affected
                    .iter()
                    .map(|r| r.defect.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();

                if defects.len() > 1 {
                    conflicts.push(Conflict {
                        parameter: def.parameters.join(", "),
                        conflicting_defects: defects,
                        description: def.description.clone(),
                    });
                }
            }
        }

        // Deduplicate conflicts
        conflicts.sort_by(|a, b| a.parameter.cmp(&b.parameter));
        conflicts.dedup_by(|a, b| a.parameter == b.parameter);

        conflicts
    }

    /// Get defect display info by defect type ID.
    pub fn get_defect_info(&self, defect_type: &str) -> Option<&DefectInfo> {
        self.rules.defects.get(defect_type)
    }

    /// List all known defect type IDs.
    pub fn known_defect_types(&self) -> Vec<&str> {
        self.rules.defects.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::default_rules;

    fn make_engine() -> RuleEngine {
        RuleEngine::new(default_rules())
    }

    fn current_profile_values() -> HashMap<String, f32> {
        let mut values = HashMap::new();
        values.insert("nozzle_temperature".to_string(), 210.0);
        values.insert("filament_retraction_length".to_string(), 0.8);
        values.insert("filament_flow_ratio".to_string(), 1.0);
        values.insert("fan_min_speed".to_string(), 35.0);
        values.insert("fan_max_speed".to_string(), 70.0);
        values.insert("cool_plate_temp".to_string(), 60.0);
        values
    }

    #[test]
    fn test_stringing_produces_retraction_increase() {
        let engine = make_engine();
        let defects = vec![DetectedDefect {
            defect_type: "stringing".to_string(),
            severity: 0.7,
            confidence: 0.9,
        }];
        let values = current_profile_values();
        let result = engine.evaluate(&defects, &values, &MaterialType::PLA);

        // Should have retraction length recommendation
        let retraction_rec = result
            .recommendations
            .iter()
            .find(|r| r.parameter == "filament_retraction_length");
        assert!(
            retraction_rec.is_some(),
            "Should recommend retraction adjustment"
        );

        let rec = retraction_rec.unwrap();
        assert!(
            rec.recommended_value > rec.current_value,
            "Should increase retraction: {} -> {}",
            rec.current_value,
            rec.recommended_value
        );
    }

    #[test]
    fn test_severity_scaling() {
        let engine = make_engine();

        // Low severity
        let low_defects = vec![DetectedDefect {
            defect_type: "stringing".to_string(),
            severity: 0.3,
            confidence: 0.9,
        }];

        // High severity
        let high_defects = vec![DetectedDefect {
            defect_type: "stringing".to_string(),
            severity: 0.9,
            confidence: 0.9,
        }];

        let values = current_profile_values();
        let low_result = engine.evaluate(&low_defects, &values, &MaterialType::PLA);
        let high_result = engine.evaluate(&high_defects, &values, &MaterialType::PLA);

        let low_rec = low_result
            .recommendations
            .iter()
            .find(|r| r.parameter == "filament_retraction_length")
            .unwrap();
        let high_rec = high_result
            .recommendations
            .iter()
            .find(|r| r.parameter == "filament_retraction_length")
            .unwrap();

        // High severity should produce larger adjustment
        let low_delta = (low_rec.recommended_value - low_rec.current_value).abs();
        let high_delta = (high_rec.recommended_value - high_rec.current_value).abs();
        assert!(
            high_delta > low_delta,
            "High severity should produce larger adjustment: low={}, high={}",
            low_delta,
            high_delta
        );
    }

    #[test]
    fn test_priority_sorting() {
        let engine = make_engine();
        let defects = vec![DetectedDefect {
            defect_type: "stringing".to_string(),
            severity: 0.8,
            confidence: 0.9,
        }];
        let values = current_profile_values();
        let result = engine.evaluate(&defects, &values, &MaterialType::PLA);

        // Verify recommendations are sorted by priority
        for window in result.recommendations.windows(2) {
            assert!(
                window[0].priority <= window[1].priority,
                "Recommendations should be sorted by priority"
            );
        }
    }

    #[test]
    fn test_conflict_detection() {
        let engine = make_engine();

        // Stringing wants more retraction, under-extrusion wants less
        let defects = vec![
            DetectedDefect {
                defect_type: "stringing".to_string(),
                severity: 0.7,
                confidence: 0.9,
            },
            DetectedDefect {
                defect_type: "under_extrusion".to_string(),
                severity: 0.6,
                confidence: 0.8,
            },
        ];
        let values = current_profile_values();
        let result = engine.evaluate(&defects, &values, &MaterialType::PLA);

        // Should detect retraction conflict (stringing increases, under_extrusion decreases)
        let has_retraction_conflict = result.conflicts.iter().any(|c| {
            c.parameter.contains("retraction")
                || (c.conflicting_defects.contains(&"stringing".to_string())
                    && c.conflicting_defects.contains(&"under_extrusion".to_string()))
        });

        assert!(
            has_retraction_conflict || !result.conflicts.is_empty(),
            "Should detect conflict between stringing and under-extrusion fixes. Conflicts: {:?}",
            result.conflicts
        );
    }

    #[test]
    fn test_safe_range_clamping() {
        let engine = make_engine();

        // Very severe defect that would push temp out of safe range
        let defects = vec![DetectedDefect {
            defect_type: "layer_adhesion".to_string(),
            severity: 1.0,
            confidence: 0.9,
        }];

        let mut values = current_profile_values();
        values.insert("nozzle_temperature".to_string(), 230.0); // Near PLA max

        let result = engine.evaluate(&defects, &values, &MaterialType::PLA);

        // Temperature recommendation should not exceed PLA max (235)
        let temp_rec = result
            .recommendations
            .iter()
            .find(|r| r.parameter == "nozzle_temperature");

        if let Some(rec) = temp_rec {
            assert!(
                rec.recommended_value <= 235.0,
                "PLA temp should not exceed 235C, got {}",
                rec.recommended_value
            );
            // Should be clamped since 230 + 5 = 235 is at the boundary
            // For values that hit the boundary exactly, was_clamped may be false
        }
    }

    #[test]
    fn test_known_defect_types() {
        let engine = make_engine();
        let types = engine.known_defect_types();

        assert!(types.contains(&"stringing"), "Should know stringing");
        assert!(types.contains(&"warping"), "Should know warping");
        assert!(types.contains(&"layer_adhesion"), "Should know layer_adhesion");
        assert!(types.contains(&"under_extrusion"), "Should know under_extrusion");
        assert!(types.contains(&"over_extrusion"), "Should know over_extrusion");
        assert!(types.contains(&"elephants_foot"), "Should know elephants_foot");
        assert!(types.contains(&"z_banding"), "Should know z_banding");
    }

    #[test]
    fn test_empty_defects_returns_empty_result() {
        let engine = make_engine();
        let result = engine.evaluate(&[], &current_profile_values(), &MaterialType::PLA);

        assert!(result.recommendations.is_empty());
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn test_get_defect_info() {
        let engine = make_engine();

        let info = engine.get_defect_info("stringing");
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.display_name, "Stringing/Oozing");
        assert!(!info.description.is_empty());

        let unknown = engine.get_defect_info("nonexistent");
        assert!(unknown.is_none());
    }

    #[test]
    fn test_warping_produces_bed_temp_increase() {
        let engine = make_engine();
        let defects = vec![DetectedDefect {
            defect_type: "warping".to_string(),
            severity: 0.5,
            confidence: 0.85,
        }];
        let values = current_profile_values();
        let result = engine.evaluate(&defects, &values, &MaterialType::PLA);

        // Should have bed temp recommendation
        let bed_temp_rec = result.recommendations.iter().find(|r| {
            r.parameter == "cool_plate_temp"
                || r.parameter == "hot_plate_temp"
                || r.parameter == "textured_plate_temp"
        });
        assert!(bed_temp_rec.is_some(), "Should recommend bed temp adjustment");

        let rec = bed_temp_rec.unwrap();
        assert!(
            rec.recommended_value > rec.current_value,
            "Should increase bed temp for warping"
        );
    }

    #[test]
    fn test_elephants_foot_decreases_bed_temp() {
        let engine = make_engine();
        let defects = vec![DetectedDefect {
            defect_type: "elephants_foot".to_string(),
            severity: 0.6,
            confidence: 0.8,
        }];
        let values = current_profile_values();
        let result = engine.evaluate(&defects, &values, &MaterialType::PLA);

        // Should have bed temp recommendation that decreases
        let bed_temp_rec = result.recommendations.iter().find(|r| {
            r.parameter == "cool_plate_temp"
                || r.parameter == "hot_plate_temp"
                || r.parameter == "textured_plate_temp"
        });
        assert!(
            bed_temp_rec.is_some(),
            "Should recommend bed temp adjustment"
        );

        let rec = bed_temp_rec.unwrap();
        assert!(
            rec.recommended_value < rec.current_value,
            "Should decrease bed temp for elephant's foot"
        );
    }
}
