# Phase 5: Defect Knowledge Base - Research

**Researched:** 2026-02-05
**Domain:** TOML-based rule engine for 3D print defect-to-parameter mapping
**Confidence:** HIGH

## Summary

This phase creates a data-driven rule engine that maps detected print defects to ranked profile parameter adjustments. The engine must be TOML-configured (not hardcoded), produce ranked recommendations (most likely fix first), and detect conflicts when multiple defect fixes interact negatively.

The research identified that 3D print defects have many-to-many relationships with fixes -- the same defect can have multiple causes, and the same fix can address multiple defects. This requires a rule engine that:
1. Loads defect-to-adjustment mappings from TOML
2. Applies severity-weighted rankings to recommendations
3. Detects when two adjustments conflict (e.g., stringing wants more retraction, but under-extrusion wants less)
4. Respects material-specific safe operating ranges from the existing `validation.rs` constraints

The standard approach is a custom Rust module that parses TOML rules at startup, evaluates defect reports against rules, and produces ranked `ParameterAdjustment` recommendations. No external rule engine library is needed -- the problem is well-scoped and hand-rolling is simpler than adapting a graph-based engine.

**Primary recommendation:** Build a simple, custom rule engine using the `toml` crate (v0.9+) for parsing, with typed Rust structs for rules, and integrate with existing `MaterialConstraints` for safe-range enforcement.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| toml | 0.9.11 | Parse TOML rule config into typed Rust structs | Official Rust TOML library, serde integration, actively maintained |
| serde | 1.x | Deserialize TOML into Rust types | Already used throughout BambuMate |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| thiserror | 1.x | Error types for rule parsing/evaluation | Already in project |
| ordered-float | 4.x | Sortable f32/f64 for severity comparisons | Optional if sorting by severity |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom rule engine | zen (gorules/zen) | Zen uses JSON graphs, requires learning new DSL, overkill for static mappings |
| Custom rule engine | json-rules-engine crate | JSON-based, less readable than TOML for human-edited rules |
| TOML config | JSON config | TOML is more human-readable for the editing use case (adding new defects) |

**Installation:**
```bash
# Already in Cargo.toml, no new dependencies needed
cargo add toml --features derive  # If not already present
```

## Architecture Patterns

### Recommended Project Structure
```
src-tauri/src/
├── mapper/                    # New module for Phase 5
│   ├── mod.rs                 # Public API: load_rules(), evaluate(), RuleEngine
│   ├── types.rs               # DefectType, Adjustment, Conflict, Recommendation
│   ├── rules.rs               # TOML parsing, rule loading
│   ├── engine.rs              # Rule evaluation logic
│   ├── conflicts.rs           # Conflict detection between adjustments
│   └── constraints.rs         # Safe-range enforcement per material
│
└── config/
    └── defect_rules.toml      # Rule definitions (data, not code)
```

### Pattern 1: Data-Driven Rule Definition
**What:** All defect-to-parameter mappings stored in TOML, not Rust code.
**When to use:** Any domain rule that changes based on user feedback or domain knowledge.
**Example:**
```toml
# config/defect_rules.toml

# Defect type definitions with display info
[defects.stringing]
display_name = "Stringing / Oozing"
description = "Thin strings of plastic between features during travel moves"
severity_range = [0.1, 1.0]

# Rule: stringing -> adjustments
[[rules]]
defect = "stringing"
severity_min = 0.3  # Only apply if severity >= 0.3

[[rules.adjustments]]
parameter = "filament_retraction_length"
operation = "increase"
amount = 0.4
unit = "mm"
priority = 1  # Primary fix
rationale = "Pull filament back further to prevent oozing during travel"

[[rules.adjustments]]
parameter = "nozzle_temperature"
operation = "decrease"
amount = 5
unit = "C"
priority = 2  # Secondary fix
rationale = "Lower temperature reduces filament fluidity"

[[rules.adjustments]]
parameter = "filament_retraction_speed"
operation = "increase"
amount = 10
unit = "mm/s"
priority = 3
rationale = "Faster retraction reduces time for oozing"
```

### Pattern 2: Typed Rule Structs with Serde
**What:** Parse TOML directly into typed Rust structs using serde.
**When to use:** Loading rule config at startup.
**Example:**
```rust
// mapper/types.rs
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RulesConfig {
    pub defects: std::collections::HashMap<String, DefectInfo>,
    pub rules: Vec<DefectRule>,
    pub conflicts: Option<Vec<ConflictDefinition>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefectInfo {
    pub display_name: String,
    pub description: String,
    pub severity_range: [f32; 2],
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefectRule {
    pub defect: String,
    pub severity_min: Option<f32>,
    pub adjustments: Vec<Adjustment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Adjustment {
    pub parameter: String,
    pub operation: Operation,
    pub amount: f32,
    pub unit: String,
    pub priority: u8,
    pub rationale: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Increase,
    Decrease,
    Set,
}
```

### Pattern 3: Conflict Detection Graph
**What:** Define known conflicts between parameters in TOML, detect at evaluation time.
**When to use:** When multiple defects produce conflicting adjustments.
**Example:**
```toml
# Conflict definitions
[[conflicts]]
name = "retraction_extrusion_tradeoff"
description = "Increasing retraction can cause under-extrusion; decreasing can cause stringing"
parameters = ["filament_retraction_length", "filament_flow_ratio"]
when = [
    { param = "filament_retraction_length", op = "increase", conflicts_with = { param = "filament_flow_ratio", when = "low" } }
]

[[conflicts]]
name = "temperature_stringing_warping"
description = "Lower temp reduces stringing but can cause layer adhesion issues"
parameters = ["nozzle_temperature"]
effects = ["stringing", "layer_adhesion"]
```

### Pattern 4: Safe-Range Enforcement
**What:** Clamp all adjustments to material-specific safe ranges from existing constraints.
**When to use:** After computing raw adjustments, before presenting to user.
**Example:**
```rust
// mapper/constraints.rs
use crate::scraper::validation::constraints_for_material;
use crate::scraper::types::MaterialType;

pub fn clamp_to_safe_range(
    param: &str,
    current: f32,
    adjustment: f32,
    material: &MaterialType
) -> (f32, bool) {  // Returns (clamped_value, was_clamped)
    let constraints = constraints_for_material(material);

    let (min, max) = match param {
        "nozzle_temperature" => (constraints.nozzle_temp_min as f32, constraints.nozzle_temp_max as f32),
        "cool_plate_temp" | "hot_plate_temp" => (constraints.bed_temp_min as f32, constraints.bed_temp_max as f32),
        "filament_retraction_length" => (0.0, 15.0),
        "filament_flow_ratio" => (0.85, 1.15),
        "fan_min_speed" | "fan_max_speed" => (0.0, 100.0),
        _ => return (current + adjustment, false),  // No constraints known
    };

    let new_value = current + adjustment;
    if new_value < min {
        (min, true)
    } else if new_value > max {
        (max, true)
    } else {
        (new_value, false)
    }
}
```

### Anti-Patterns to Avoid
- **Hardcoding rules in match statements:** Defeats the purpose of data-driven design; requires recompilation to add new defects
- **Complex graph-based rule engines:** Overkill for static mappings; adds dependency bloat
- **Storing rules in JSON:** Less human-readable than TOML for the "domain expert edits rules" use case
- **Ignoring material type:** PLA at 300C is dangerous; always enforce material-specific ranges

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| TOML parsing | Custom parser | `toml` crate with serde | Well-tested, handles edge cases |
| Material constraints | New constraint system | Existing `scraper::validation::constraints_for_material` | Already implemented in Phase 3 |
| Error types | String errors | `thiserror` | Consistent with project patterns |

**Key insight:** The rule engine itself is simple enough to hand-roll (just TOML -> struct -> evaluate), but use existing crates for TOML parsing and integrate with existing constraint code.

## Common Pitfalls

### Pitfall 1: Ignoring Severity Thresholds
**What goes wrong:** Low-severity defects trigger aggressive adjustments that overcorrect.
**Why it happens:** Rules apply regardless of defect severity.
**How to avoid:** Every rule should have a `severity_min` threshold; adjustments should scale with severity.
**Warning signs:** User applies recommendation, problem gets worse.

### Pitfall 2: Conflicting Adjustments Without Warning
**What goes wrong:** User applies two recommendations that cancel each other out (e.g., increase retraction for stringing, decrease for under-extrusion).
**Why it happens:** No conflict detection between recommendations.
**How to avoid:** Explicitly model known conflicts in TOML; detect at evaluation time; warn user.
**Warning signs:** Multiple defects in same report produce contradictory parameter changes.

### Pitfall 3: Exceeding Safe Material Ranges
**What goes wrong:** Recommendation suggests PLA at 250C (dangerous) or ABS with 100% fan (causes warping).
**Why it happens:** Rules don't know the material type.
**How to avoid:** Always pass material type to the engine; clamp all adjustments to `MaterialConstraints` ranges.
**Warning signs:** Recommendations outside normal ranges for filament type.

### Pitfall 4: Hardcoded Bambu Studio Parameter Names
**What goes wrong:** Rule uses wrong parameter name, profile writer ignores it.
**Why it happens:** Bambu Studio field names are undocumented, easy to guess wrong.
**How to avoid:** Use exact field names from actual profiles: `filament_retraction_length` not `retraction_length`.
**Warning signs:** Adjustments written to profile but no effect in Bambu Studio.

### Pitfall 5: Missing Rationale for Recommendations
**What goes wrong:** User doesn't understand why a change was recommended, applies blindly, makes things worse.
**Why it happens:** Rules produce adjustments without explanation.
**How to avoid:** Every adjustment in TOML must have a `rationale` field; UI displays it.
**Warning signs:** Users ask "why did it suggest this?"

## Code Examples

### Loading Rules from TOML
```rust
// mapper/rules.rs
use std::path::Path;
use anyhow::Result;
use crate::mapper::types::RulesConfig;

pub fn load_rules(path: &Path) -> Result<RulesConfig> {
    let content = std::fs::read_to_string(path)?;
    let config: RulesConfig = toml::from_str(&content)?;
    Ok(config)
}

// Embed default rules in binary for distribution
pub fn default_rules() -> RulesConfig {
    const DEFAULT_RULES: &str = include_str!("../../config/defect_rules.toml");
    toml::from_str(DEFAULT_RULES).expect("embedded rules must be valid")
}
```

### Rule Evaluation Engine
```rust
// mapper/engine.rs
use crate::mapper::types::*;
use crate::scraper::types::MaterialType;

pub struct RuleEngine {
    rules: RulesConfig,
}

impl RuleEngine {
    pub fn new(rules: RulesConfig) -> Self {
        Self { rules }
    }

    /// Evaluate a defect report and produce ranked recommendations
    pub fn evaluate(
        &self,
        defects: &[DetectedDefect],
        current_profile: &FilamentProfile,
        material: &MaterialType,
    ) -> EvaluationResult {
        let mut recommendations = Vec::new();
        let mut conflicts = Vec::new();

        for defect in defects {
            // Find rules for this defect type
            let applicable_rules: Vec<_> = self.rules.rules.iter()
                .filter(|r| r.defect == defect.defect_type)
                .filter(|r| r.severity_min.map_or(true, |min| defect.severity >= min))
                .collect();

            for rule in applicable_rules {
                for adj in &rule.adjustments {
                    // Get current value from profile
                    let current = current_profile.get_numeric_value(&adj.parameter);

                    // Compute scaled adjustment based on severity
                    let scaled_amount = adj.amount * defect.severity;

                    // Clamp to safe range for material
                    let (clamped, was_clamped) = clamp_to_safe_range(
                        &adj.parameter,
                        current,
                        scaled_amount,
                        material
                    );

                    recommendations.push(Recommendation {
                        defect: defect.defect_type.clone(),
                        parameter: adj.parameter.clone(),
                        current_value: current,
                        recommended_value: clamped,
                        priority: adj.priority,
                        rationale: adj.rationale.clone(),
                        was_clamped,
                    });
                }
            }
        }

        // Sort by priority (primary fixes first)
        recommendations.sort_by_key(|r| r.priority);

        // Detect conflicts
        conflicts = self.detect_conflicts(&recommendations);

        EvaluationResult {
            recommendations,
            conflicts,
        }
    }
}
```

### Conflict Detection
```rust
// mapper/conflicts.rs
use crate::mapper::types::*;

impl RuleEngine {
    pub fn detect_conflicts(&self, recommendations: &[Recommendation]) -> Vec<Conflict> {
        let mut conflicts = Vec::new();

        // Group recommendations by parameter
        let by_param: std::collections::HashMap<_, Vec<_>> = recommendations.iter()
            .fold(std::collections::HashMap::new(), |mut acc, r| {
                acc.entry(&r.parameter).or_default().push(r);
                acc
            });

        // Check for same parameter adjusted in opposite directions
        for (param, recs) in &by_param {
            if recs.len() > 1 {
                let directions: Vec<_> = recs.iter()
                    .map(|r| (r.recommended_value - r.current_value).signum())
                    .collect();

                // If some increase and some decrease, we have a conflict
                if directions.iter().any(|d| *d > 0.0) &&
                   directions.iter().any(|d| *d < 0.0) {
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

        // Check defined conflict pairs from rules
        if let Some(conflict_defs) = &self.rules.conflicts {
            for def in conflict_defs {
                let affected: Vec<_> = recommendations.iter()
                    .filter(|r| def.parameters.contains(&r.parameter))
                    .collect();

                if affected.len() > 1 {
                    conflicts.push(Conflict {
                        parameter: def.parameters.join(", "),
                        conflicting_defects: affected.iter().map(|r| r.defect.clone()).collect(),
                        description: def.description.clone(),
                    });
                }
            }
        }

        conflicts
    }
}
```

## Bambu Studio Profile Parameters

Verified parameter names from actual profiles (use exactly these names):

### Temperature Parameters
| Field Name | Type | Description |
|------------|------|-------------|
| `nozzle_temperature` | Array<String> | Printing temperature per extruder |
| `nozzle_temperature_initial_layer` | Array<String> | First layer temp |
| `nozzle_temperature_range_low` | Array<String> | Min safe temp |
| `nozzle_temperature_range_high` | Array<String> | Max safe temp |
| `cool_plate_temp` | Array<String> | Smooth PEI bed temp |
| `hot_plate_temp` | Array<String> | High-temp plate temp |
| `textured_plate_temp` | Array<String> | Textured PEI temp |

### Retraction Parameters
| Field Name | Type | Description |
|------------|------|-------------|
| `filament_retraction_length` | Array<String> | Retraction distance (mm) |
| `filament_retraction_speed` | Array<String> | Retraction speed (mm/s) |
| `filament_retract_length_nc` | Number | Retraction for nozzle change |
| `filament_z_hop` | Array<String> | Z-hop distance |

### Fan/Cooling Parameters
| Field Name | Type | Description |
|------------|------|-------------|
| `fan_min_speed` | Array<String> | Minimum fan speed (%) |
| `fan_max_speed` | Array<String> | Maximum fan speed (%) |
| `overhang_fan_speed` | Array<String> | Fan speed for overhangs |

### Flow Parameters
| Field Name | Type | Description |
|------------|------|-------------|
| `filament_flow_ratio` | Array<String> | Extrusion multiplier (1.0 = 100%) |
| `pressure_advance` | Array<String> | Linear advance value |
| `filament_prime_volume` | Number | Prime blob size |

**Important quirk:** Most values are stored as strings in single-element arrays (e.g., `["215"]`). The rule engine must handle this format when reading current values.

## Complete Defect-to-Parameter Mapping

Based on 3D printing domain research:

### Stringing / Oozing
| Priority | Parameter | Direction | Amount | Rationale |
|----------|-----------|-----------|--------|-----------|
| 1 | `filament_retraction_length` | increase | +0.5mm | Pull filament back further |
| 2 | `nozzle_temperature` | decrease | -5C | Reduce fluidity |
| 3 | `filament_retraction_speed` | increase | +10mm/s | Faster retraction |

### Warping
| Priority | Parameter | Direction | Amount | Rationale |
|----------|-----------|-----------|--------|-----------|
| 1 | `cool_plate_temp` | increase | +5C | Better first layer adhesion |
| 2 | `fan_min_speed` | decrease | -20% | Slower cooling reduces stress |
| 3 | `nozzle_temperature_initial_layer` | increase | +5C | Hotter first layer sticks better |

### Layer Adhesion / Delamination
| Priority | Parameter | Direction | Amount | Rationale |
|----------|-----------|-----------|--------|-----------|
| 1 | `nozzle_temperature` | increase | +5C | Better layer bonding |
| 2 | `fan_max_speed` | decrease | -15% | Slower cooling improves adhesion |
| 3 | `filament_flow_ratio` | increase | +0.02 | More material between layers |

### Elephant's Foot
| Priority | Parameter | Direction | Amount | Rationale |
|----------|-----------|-----------|--------|-----------|
| 1 | `cool_plate_temp` | decrease | -5C | Less squish on first layer |
| 2 | `nozzle_temperature_initial_layer` | decrease | -5C | Firmer first layer |

### Under-Extrusion
| Priority | Parameter | Direction | Amount | Rationale |
|----------|-----------|-----------|--------|-----------|
| 1 | `filament_flow_ratio` | increase | +0.03 | More material |
| 2 | `nozzle_temperature` | increase | +5C | Better flow |
| 3 | `filament_retraction_length` | decrease | -0.3mm | Less retraction prevents gaps |

### Over-Extrusion / Blobs
| Priority | Parameter | Direction | Amount | Rationale |
|----------|-----------|-----------|--------|-----------|
| 1 | `filament_flow_ratio` | decrease | -0.03 | Less material |
| 2 | `nozzle_temperature` | decrease | -5C | Reduce fluidity |

### Z-Banding / Ribbing
| Priority | Parameter | Direction | Amount | Rationale |
|----------|-----------|-----------|--------|-----------|
| 1 | Hardware check | N/A | N/A | Z-banding is usually mechanical, not slicer settings |
| 2 | `pressure_advance` | tune | calibrate | PA inconsistency can cause visible lines |

### Known Conflicts
| Conflict | Parameters | Description |
|----------|------------|-------------|
| Stringing vs Under-Extrusion | `filament_retraction_length` | Stringing wants more retraction; under-extrusion wants less |
| Temperature vs Everything | `nozzle_temperature` | Higher temp helps flow but increases stringing |
| Cooling vs Adhesion | `fan_min_speed`, `fan_max_speed` | More cooling prevents stringing but causes warping |

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Hardcoded if/else trees | Data-driven TOML rules | Always preferred | Rules can be updated without recompiling |
| Single-cause assumption | Multi-cause with ranking | Modern practice | Users see all possible fixes, not just one |
| Fixed adjustment amounts | Severity-scaled adjustments | Modern practice | Light stringing gets light fix |

**Deprecated/outdated:**
- Generic "increase retraction" advice: Modern approach specifies exact amounts
- Single-fix recommendations: Now always show ranked alternatives

## Open Questions

1. **How to handle compound defects?**
   - What we know: AI may detect multiple defects with different severities
   - What's unclear: Should conflicting recommendations cancel out or show both?
   - Recommendation: Show both with explicit conflict warning; let user choose

2. **Should adjustments scale linearly with severity?**
   - What we know: Severity 0.3 is mild, 0.9 is severe
   - What's unclear: Is `amount * severity` the right formula?
   - Recommendation: Start with linear scaling; add multiplier curves in v2 if needed

3. **Where should the TOML file live in production?**
   - What we know: Bundled in binary via `include_str!` works for defaults
   - What's unclear: Should users be able to override with custom rules?
   - Recommendation: Bundle defaults; support optional user override at `~/.bambumate/defect_rules.toml`

## Sources

### Primary (HIGH confidence)
- `toml` crate documentation (v0.9.11): https://docs.rs/toml/latest/toml/ - TOML parsing patterns
- Bambu Studio fdm_filament_common.json: https://github.com/bambulab/BambuStudio/blob/master/resources/profiles/BBL/filament/fdm_filament_common.json - Verified parameter names
- Existing BambuMate code: `src-tauri/src/scraper/validation.rs` - MaterialConstraints already implemented
- Existing BambuMate code: `src-tauri/src/profile/types.rs` - FilamentProfile with Map<String, Value>

### Secondary (MEDIUM confidence)
- Simplify3D Troubleshooting Guide: https://www.simplify3d.com/resources/print-quality-troubleshooting/stringing-or-oozing/ - Defect-to-fix mappings
- 3DXTech 27 Common Problems: https://www.3dxtech.com/blogs/trouble-shooting/27-common-fdm-3d-printing-problems-and-how-to-fix-them - Comprehensive defect list
- UnionFab Stringing Guide: https://www.unionfab.com/blog/2024/05/3d-print-stringing - Retraction parameter ranges

### Tertiary (LOW confidence)
- gorules/zen engine: https://github.com/gorules/zen - Evaluated but not recommended (JSON graph, overkill)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - toml crate is standard, serde already in project
- Architecture: HIGH - Pattern follows existing BambuMate module structure
- Defect mappings: MEDIUM - Based on community knowledge, may need tuning
- Conflict detection: MEDIUM - Algorithm is sound, conflict definitions need validation

**Research date:** 2026-02-05
**Valid until:** 60 days (domain knowledge is stable; 3D printing best practices don't change rapidly)
